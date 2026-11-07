//! Binary-level tests of the paths that open no window: --help, --version,
//! argument errors and --report. PATH points at fake lpstat/lpoptions/lp
//! scripts that log their arguments, so no real CUPS command runs and no job
//! can reach a printer. The fake queue URI is not usb://, or its serial
//! matches no usblp device, so no status request is sent.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_escp-utility");

struct Shim {
    dir: PathBuf,
}

impl Shim {
    /// `lpstat -v` prints `lpstat_v`, `lpstat -o` prints `jobs`; `lpoptions -l`
    /// prints a canned option list.
    fn new(name: &str, lpstat_v: &str, jobs: &str) -> Shim {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("cli-shim-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("bin")).unwrap();
        fs::create_dir_all(dir.join("home")).unwrap();
        fs::write(dir.join("lpstat_v"), lpstat_v).unwrap();
        fs::write(dir.join("jobs"), jobs).unwrap();
        let log = dir.join("log");
        let d = dir.display();
        let lpstat = format!(
            "#!/bin/sh\necho \"lpstat $*\" >> {d}/log\ncase \"$1\" in\n  -v) cat {d}/lpstat_v ;;\n  -o) cat {d}/jobs ;;\nesac\nexit 0\n"
        );
        let lpoptions = format!(
            "#!/bin/sh\necho \"lpoptions $*\" >> {d}/log\nfor a in \"$@\"; do [ \"$a\" = -l ] && {{ echo 'PageSize/Media Size: *A4 Letter'; exit 0; }}; done\necho \"printer-make-and-model='Epson L800 Series'\"\n"
        );
        let lp = format!("#!/bin/sh\necho \"lp $*\" >> {d}/log\nexit 0\n");
        for (n, body) in [("lpstat", lpstat), ("lpoptions", lpoptions), ("lp", lp)] {
            let p = dir.join("bin").join(n);
            fs::write(&p, body).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let _ = log;
        Shim { dir }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(BIN)
            .args(args)
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", self.dir.join("bin").display()))
            .env("HOME", self.dir.join("home"))
            .env("XDG_CACHE_HOME", self.dir.join("home/.cache"))
            .env("XDG_CONFIG_HOME", self.dir.join("home/.config"))
            .env("LC_ALL", "C")
            .output()
            .unwrap()
    }

    fn log(&self) -> String {
        fs::read_to_string(self.dir.join("log")).unwrap_or_default()
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn help_prints_usage_and_exits_0() {
    let s = Shim::new("help", "", "");
    for flag in ["--help", "-h"] {
        let o = s.run(&[flag]);
        assert_eq!(o.status.code(), Some(0));
        assert!(stdout(&o).starts_with("Usage: escp-utility"), "{}", stdout(&o));
        assert!(stdout(&o).contains("--report"));
    }
    assert_eq!(s.log(), "", "help runs no CUPS command");
}

#[test]
fn version_prints_the_crate_version() {
    let s = Shim::new("version", "", "");
    let o = s.run(&["--version"]);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(stdout(&o), format!("escp-utility {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn bad_arguments_exit_2_before_any_window() {
    let s = Shim::new("badargs", "", "");
    let o = s.run(&["--bogus"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("unknown argument: --bogus"));
    let o = s.run(&["--dump-dir", "/nonexistent"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("--dump-dir needs --dry-run"));
    let o = s.run(&["--queue"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("--queue needs a value"));
    assert_eq!(s.log(), "");
}

#[test]
fn unreadable_profile_exits_2() {
    let s = Shim::new("profile", "", "");
    let o = s.run(&["--report", "--queue", "FAKE", "--profile", "/nonexistent/profiles.toml"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("/nonexistent/profiles.toml"));
    assert!(!s.log().contains("lp "), "{}", s.log());
}

#[test]
fn report_on_a_fake_queue_reads_options_and_never_prints() {
    let s = Shim::new("report", "device for FAKE: ipp://192.0.2.1/ipp/print\n", "");
    let o = s.run(&["--report", "--queue", "FAKE"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("report\n\nQueue: FAKE\n"), "{out}");
    assert!(out.contains("$ lpoptions -p FAKE -l\nPageSize/Media Size: *A4 Letter"), "{out}");
    assert!(out.contains("Status: no reply"), "{out}");
    let log = s.log();
    assert!(log.lines().any(|l| l == "lpoptions -p FAKE -l"), "{log}");
    assert!(log.lines().any(|l| l == "lpstat -o FAKE"), "jobs are checked before any status query: {log}");
    assert!(!log.lines().any(|l| l.starts_with("lp ")), "report must never run lp: {log}");
}

#[test]
fn report_does_not_query_while_cups_has_jobs() {
    let s = Shim::new("busy", "device for FAKE: ipp://192.0.2.1/ipp/print\n", "FAKE-1 user 1024 Sat 07 Nov 2026\n");
    let o = s.run(&["--report", "--queue", "FAKE"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(stdout(&o).contains("Status: not read, the CUPS queue has jobs."), "{}", stdout(&o));
}

#[test]
fn report_masks_the_usb_serial_from_the_uri() {
    let serial = "ZZNOSUCHSERIAL0042";
    let s = Shim::new("serial", &format!("device for FAKE: usb://EPSON/L800?serial={serial}\n"), "");
    let o = s.run(&["--report", "--queue", "FAKE"]);
    assert_eq!(o.status.code(), Some(0), "{}", stderr(&o));
    let out = stdout(&o);
    assert!(!out.contains(serial), "serial leaked: {out}");
    assert!(out.contains(&"X".repeat(serial.len())), "{out}");
    assert!(!s.log().lines().any(|l| l.starts_with("lp ")));
}

#[test]
fn report_without_queue_and_two_epson_queues_lists_them() {
    let s = Shim::new("twoq", "device for A: usb://EPSON/L800?serial=ZZ1\ndevice for B: usb://EPSON/L805?serial=ZZ2\n", "");
    let o = s.run(&["--report"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("--report needs --queue NAME (Epson queues: A, B)"), "{}", stderr(&o));
    assert!(!s.log().lines().any(|l| l.starts_with("lp ")));
}

//! Sending a maintenance job, or writing it to a file in dry-run mode.
//!
//! Windows opens the printer with `StartDocPrinterW(datatype "RAW")` and
//! `WritePrinter`s the bytes (`E_IHUTG5P` `FUN_00408008`, `FUN_004081a0`).
//! The Linux equivalent is a raw CUPS job: `lp -d <queue> -o raw`. CUPS keeps
//! ownership of the USB device, so this does not fight the usb backend or ecbd.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Clone, Debug)]
pub enum Mode {
    /// Submit with `lp -d <queue> -o raw`.
    Live,
    /// Write the bytes to a file in this directory. Never calls `lp`.
    DryRun(PathBuf),
}

#[derive(Clone, Debug)]
pub struct Sent {
    /// One line for the user: the CUPS request id, or the dump file.
    pub summary: String,
    /// Set in dry-run mode.
    pub dump: Option<(PathBuf, String)>,
}

/// Default dry-run directory: `$XDG_CACHE_HOME/escp-utility`, or `~/.cache/escp-utility`.
pub fn default_dump_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("escp-utility")
}

/// Submit `bytes` as one raw job. `name` is a short slug such as "nozzle-check".
pub fn submit(mode: &Mode, queue: &str, name: &str, bytes: &[u8]) -> Result<Sent, String> {
    match mode {
        Mode::DryRun(dir) => write_dump(dir, name, bytes),
        Mode::Live => lp_raw(queue, name, bytes),
    }
}

fn write_dump(dir: &Path, name: &str, bytes: &[u8]) -> Result<Sent, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = dir.join(format!("{name}-{stamp}.bin"));
    std::fs::write(&path, bytes).map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
    Ok(Sent {
        summary: format!("Dry run: {} bytes written to {}. Nothing was sent.", bytes.len(), path.display()),
        dump: Some((path, crate::commands::hexdump(bytes))),
    })
}

fn lp_raw(queue: &str, name: &str, bytes: &[u8]) -> Result<Sent, String> {
    let mut child = Command::new("lp")
        .args(["-d", queue, "-o", "raw", "-t", name])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Cannot run lp: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("lp has no stdin")?
        .write_all(bytes)
        .map_err(|e| format!("Cannot write to lp: {e}"))?;
    let out = child.wait_with_output().map_err(|e| format!("lp failed: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(Sent { summary: String::from_utf8_lossy(&out.stdout).trim().to_string(), dump: None })
}

/// Why the queue cannot take a job right now. Windows checks the spooler
/// before sending (`FUN_00429120`, GetPrinter level 2: "Work Offline" or
/// "Paused" gives message 3000). The CUPS equivalent is a disabled
/// (stopped) queue or one that rejects jobs.
pub fn queue_problem(queue: &str) -> Option<String> {
    queue_problem_with("lpstat", queue)
}

/// `queue_problem` with the lpstat program named, so tests can use a stub.
fn queue_problem_with(lpstat: &str, queue: &str) -> Option<String> {
    // C locale: the lines parsed below are translated in other locales.
    let out = match Command::new(lpstat).env("LC_ALL", "C").args(["-p", queue, "-a", queue]).output() {
        Ok(o) => o,
        Err(e) => return Some(format!("Cannot run lpstat: {e}")),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Some(format!("CUPS queue \"{queue}\": {}", err.trim()));
    }
    parse_queue_state(queue, &text)
}

fn parse_queue_state(queue: &str, lpstat: &str) -> Option<String> {
    for line in lpstat.lines() {
        let l = line.trim();
        if l.starts_with(&format!("printer {queue} ")) && l.contains(" disabled") {
            return Some(format!("CUPS queue \"{queue}\" is paused (disabled)."));
        }
        if l.starts_with(&format!("{queue} not accepting")) {
            return Some(format!("CUPS queue \"{queue}\" is not accepting jobs."));
        }
    }
    None
}

/// Text for "Printer and Option Information": what CUPS knows about the queue.
/// Read-only queries to cupsd; nothing reaches the printer.
pub fn queue_info(queue: &str) -> String {
    let mut s = String::new();
    for args in [vec!["-l", "-p", queue], vec!["-v", queue], vec!["-a", queue], vec!["-o", queue]] {
        s.push_str(&format!("$ lpstat {}\n", args.join(" ")));
        match Command::new("lpstat").args(&args).output() {
            Ok(o) => {
                s.push_str(&String::from_utf8_lossy(&o.stdout));
                s.push_str(&String::from_utf8_lossy(&o.stderr));
            }
            Err(e) => s.push_str(&format!("cannot run lpstat: {e}\n")),
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_writes_exact_bytes_and_never_needs_lp() {
        let dir = std::env::temp_dir().join(format!("l800-dry-{}", std::process::id()));
        let bytes = b"NC\x02\x00\x00\x00";
        let sent = submit(&Mode::DryRun(dir.clone()), "no-such-queue", "nozzle-check", bytes).unwrap();
        let (path, dump) = sent.dump.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(dump.starts_with("00000000  4e 43 02"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn queue_state_parsing() {
        let ok = "printer L800 is idle.  enabled since Fri 07 Nov 2026\nL800 accepting requests since Fri 07 Nov 2026\n";
        assert_eq!(parse_queue_state("L800", ok), None);
        let paused = "printer L800 disabled since Fri 07 Nov 2026 -\n\tPaused\nL800 accepting requests since Fri 07 Nov 2026\n";
        assert!(parse_queue_state("L800", paused).unwrap().contains("paused"));
        let rejecting = "printer L800 is idle.  enabled since x\nL800 not accepting requests since x -\n";
        assert!(parse_queue_state("L800", rejecting).unwrap().contains("not accepting"));
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("escp-transport-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A fake lpstat that prints `stdout`, `stderr` and exits with `code`.
    fn stub(dir: &Path, stdout: &str, stderr: &str, code: i32) -> String {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join("lpstat");
        std::fs::write(&p, format!("#!/bin/sh\nprintf '%s' '{stdout}'\nprintf '%s' '{stderr}' >&2\nexit {code}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.to_str().unwrap().to_string()
    }

    #[test]
    fn dry_run_file_name_content_and_hexdump() {
        let dir = scratch("dump");
        let bytes: Vec<u8> = (0u8..20).chain(*b"AB").collect();
        let sent = submit(&Mode::DryRun(dir.join("sub")), "L800", "head-cleaning", &bytes).unwrap();
        let (path, dump) = sent.dump.unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        // head-cleaning-YYYYMMDD-HHMMSS.bin
        assert!(name.starts_with("head-cleaning-") && name.ends_with(".bin"), "{name}");
        assert_eq!(name.len(), "head-cleaning-".len() + 15 + 4);
        assert_eq!(path.parent().unwrap(), dir.join("sub"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(sent.summary.contains("22 bytes") && sent.summary.contains("Nothing was sent"));
        let lines: Vec<&str> = dump.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].starts_with("00000010  10 11 12 13 41 42"));
        assert!(lines[1].ends_with("|....AB|") || lines[1].contains("AB"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dry_run_unwritable_dir_is_an_error() {
        let dir = scratch("ro");
        let file = dir.join("not-a-dir");
        std::fs::write(&file, b"x").unwrap();
        let err = submit(&Mode::DryRun(file.clone()), "L800", "x", b"1").unwrap_err();
        assert!(err.contains("Cannot create"), "{err}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn queue_state_ignores_other_queues_and_prefix_names() {
        let other = "printer L800-2 disabled since x -\nL800-2 not accepting requests since x -\nprinter L800 is idle.  enabled since x\n";
        assert_eq!(parse_queue_state("L800", other), None);
        assert_eq!(parse_queue_state("L800", ""), None);
        // Printing (busy) with jobs is fine.
        let busy = "printer L800 now printing L800-12.  enabled since x\nL800 accepting requests since x\n";
        assert_eq!(parse_queue_state("L800", busy), None);
    }

    #[test]
    fn queue_problem_idle_paused_rejecting_via_stub() {
        let dir = scratch("stub-ok");
        let idle = stub(&dir, "printer L800 is idle.  enabled since x\nL800 accepting requests since x\n", "", 0);
        assert_eq!(queue_problem_with(&idle, "L800"), None);
        let paused = stub(&dir, "printer L800 disabled since x -\n\tPaused\nL800 accepting requests since x\n", "", 0);
        assert!(queue_problem_with(&paused, "L800").unwrap().contains("paused"));
        let rej = stub(&dir, "printer L800 is idle.  enabled since x\nL800 not accepting requests since x -\n", "", 0);
        assert!(queue_problem_with(&rej, "L800").unwrap().contains("not accepting"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn queue_problem_missing_queue_and_lpstat_failure_are_errors() {
        let dir = scratch("stub-err");
        let missing = stub(&dir, "", "lpstat: Invalid destination name in list \"NOPE\".", 1);
        let msg = queue_problem_with(&missing, "NOPE").unwrap();
        assert!(msg.contains("NOPE") && msg.contains("Invalid destination"), "{msg}");
        assert!(queue_problem_with(dir.join("absent").to_str().unwrap(), "L800").unwrap().starts_with("Cannot run lpstat"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

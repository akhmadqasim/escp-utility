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
    // C locale: the lines parsed below are translated in other locales.
    let out = match Command::new("lpstat").env("LC_ALL", "C").args(["-p", queue, "-a", queue]).output() {
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
}

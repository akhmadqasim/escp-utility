//! Smoke test of the real start-up path (backend selection, app_id, the
//! sheet), which unit tests cannot reach: they use an offscreen platform.
//! It opens a window for about 3 s, so it needs a display and is ignored by
//! default: `cargo test -- --ignored`. The queue does not exist, so nothing
//! reaches a printer (no status query, no job).

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
#[ignore = "opens a window; needs WAYLAND_DISPLAY or DISPLAY"]
fn app_starts_and_stays_up() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_epson-l800-utility"))
        .args(["--dry-run", "--utility", "--queue", "NO_SUCH_QUEUE_SMOKE_TEST"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(3) {
        if child.try_wait().unwrap().is_some() {
            let out = child.wait_with_output().unwrap();
            panic!("the app exited at start: {}", String::from_utf8_lossy(&out.stderr));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    child.kill().unwrap();
    child.wait().unwrap();
}

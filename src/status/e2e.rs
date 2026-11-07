//! End to end against a fake usblp node: a pty slave stands in for
//! `/dev/usb/lpN` and goes through the real `UsbLp` open/poll/read/write;
//! a thread on the master side plays the printer's 1284.4 side.

use super::*;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::io::FromRawFd;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

type Writes = Arc<Mutex<Vec<Vec<u8>>>>;

/// Raw pty pair; returns the master and the slave path.
fn pty() -> (File, PathBuf) {
    unsafe {
        let m = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
        assert!(m >= 0);
        assert_eq!(libc::grantpt(m), 0);
        assert_eq!(libc::unlockpt(m), 0);
        let mut t: libc::termios = std::mem::zeroed();
        assert_eq!(libc::tcgetattr(m, &mut t), 0);
        libc::cfmakeraw(&mut t);
        assert_eq!(libc::tcsetattr(m, libc::TCSANOW, &t), 0);
        let mut name = [0 as libc::c_char; 128];
        assert_eq!(libc::ptsname_r(m, name.as_mut_ptr(), name.len()), 0);
        let path = std::ffi::CStr::from_ptr(name.as_ptr()).to_str().unwrap().into();
        (File::from_raw_fd(m), path)
    }
}

/// Printer side: split host bytes into 1284.4 packets (length at 2..4,
/// big endian), record them, answer the first script entry they start with.
/// Ends when the host closes the node (EIO on the master).
fn fake_printer(mut master: File, mut script: Vec<(Vec<u8>, Vec<u8>)>) -> (Writes, JoinHandle<()>) {
    let writes: Writes = Arc::default();
    let w = writes.clone();
    let h = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 512];
        while let Ok(n @ 1..) = master.read(&mut chunk) {
            buf.extend_from_slice(&chunk[..n]);
            while buf.len() >= 4 {
                let len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
                if len < 6 || buf.len() < len {
                    break;
                }
                let pkt: Vec<u8> = buf.drain(..len).collect();
                if let Some(i) = script.iter().position(|(k, _)| pkt.starts_with(k)) {
                    let (_, reply) = script.remove(i);
                    master.write_all(&reply).unwrap();
                }
                w.lock().unwrap().push(pkt);
            }
        }
    });
    (writes, h)
}

#[test]
fn recorded_l800_reply_through_fake_usblp_to_monitor_model() {
    let (master, node) = pty();
    let live = st2::tests::live();
    let (writes, h) = fake_printer(master, d4::tests::script_with(live.clone()));

    let raw = query_node(&node).unwrap();
    h.join().unwrap();
    assert_eq!(raw, live);

    // Host side: Enter first, Exit last, one data packet (the status request).
    let w = writes.lock().unwrap();
    assert_eq!(w.first().unwrap(), &d4::ENTER.to_vec());
    assert_eq!(w.last().unwrap(), &d4::EXIT.to_vec());
    let data: Vec<_> = w.iter().filter(|p| p[0] != 0).collect();
    assert_eq!(data, vec![&d4::data_packet(d4::CTRL_SID, &d4::STATUS_PAYLOAD)]);

    // Decode, pre-check and the Status Monitor model (main.rs `ink_levels`).
    let s = Status::from_reply(&raw).unwrap();
    assert_eq!(s.st, Some(0x04));
    assert_eq!(precheck(&s, true), Precheck::Proceed);
    assert_eq!(precheck(&s, false), Precheck::Proceed);
    let (title, text) = message(&s);
    assert_eq!(title, "Ink low");
    assert!(text.starts_with("Light Cyan: "), "{text}");
    let levels: Vec<(&str, i32)> =
        s.inks.iter().map(|i| (ink_name(i), i.percent().map_or(-1, i32::from))).collect();
    assert_eq!(levels.len(), 6);
    assert!(levels.iter().all(|&(_, p)| (0..=100).contains(&p)), "{levels:?}");
    let names: Vec<&str> = levels.iter().map(|l| l.0).collect();
    assert_eq!(names, ["Cyan", "Magenta", "Light Magenta", "Light Cyan", "Black", "Yellow"]);
    // The ink named low in the text is the lowest meter.
    let low = levels.iter().min_by_key(|l| l.1).unwrap();
    assert_eq!(low.0, "Light Cyan");
}

#[test]
fn fake_usblp_silent_on_enter_times_out_without_exit() {
    let (master, node) = pty();
    let (writes, h) = fake_printer(master, vec![]);

    // query_node's session with a short timeout (it waits 2 s, then 4 s).
    let t = Instant::now();
    let res = {
        let mut s = d4::Session::new(usblp::UsbLp::open(&node).unwrap());
        s.timeout = Duration::from_millis(50);
        s.query()
    };
    let took = t.elapsed();
    h.join().unwrap();

    let e = res.unwrap_err().to_string();
    assert!(e.contains("EnterIEEE"), "{e}");
    // Drain (0.2 s), Enter (50 ms), one retry (100 ms).
    assert!(took >= Duration::from_millis(350) && took < Duration::from_secs(2), "{took:?}");
    // Only Enter went out: no Init, no Exit (7 bytes of print data otherwise).
    let w = writes.lock().unwrap();
    assert!(!w.is_empty());
    assert!(w.iter().all(|p| p == &d4::ENTER.to_vec()), "{w:?}");
}

//! `/dev/usb/lpN` (kernel usblp) as a byte port.
//!
//! Going through usblp means no detach/reattach: the kernel driver stays
//! bound. usblp allows one opener at a time.

use super::d4::Port;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::time::Duration;

pub struct UsbLp {
    f: File,
}

impl UsbLp {
    pub fn open(path: &Path) -> io::Result<Self> {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(path)?;
        Ok(UsbLp { f })
    }

    fn poll(&self, events: libc::c_short, timeout: Duration) -> io::Result<bool> {
        let mut pfd = libc::pollfd { fd: self.f.as_raw_fd(), events, revents: 0 };
        let ms = timeout.as_millis().min(i32::MAX as u128) as libc::c_int;
        let r = unsafe { libc::poll(&mut pfd, 1, ms) };
        if r < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                return Ok(false);
            }
            return Err(e);
        }
        if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(io::Error::other(format!("usblp poll revents 0x{:x}", pfd.revents)));
        }
        Ok(r > 0 && pfd.revents & events != 0)
    }
}

impl Port for UsbLp {
    fn write_all(&mut self, mut data: &[u8]) -> io::Result<()> {
        let mut stalls = 0;
        while !data.is_empty() {
            match self.f.write(data) {
                Ok(0) => return Err(io::Error::other("usblp write returned 0")),
                Ok(n) => data = &data[n..],
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    stalls += 1;
                    if stalls > 10 {
                        return Err(io::Error::new(io::ErrorKind::TimedOut, "usblp write stalled"));
                    }
                    self.poll(libc::POLLOUT, Duration::from_millis(500))?;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn read_some(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        if !self.poll(libc::POLLIN, timeout)? {
            return Ok(0);
        }
        match self.f.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(0),
            Err(e) => Err(e),
        }
    }
}

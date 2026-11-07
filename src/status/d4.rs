//! Minimal IEEE 1284.4 ("D4") client that only reads the status reply.
//!
//! The packet bytes mirror ecbd's ECBT engine (`ecbteg.c`, decompiled):
//! `Tx_EpsonPacking`, `Init_Command`, `OpenChannel_Command`,
//! `CreditRequest_Command`, `DataWrite_Fnc`, `Credit_Command`,
//! `CloseChannel_Command`, `Exit_Command`. ecbd talks to socket id 2
//! directly (no GetSocketID), so this client does the same.
//!
//! Packet header (6 bytes): `psid ssid len_hi len_lo credit control`.
//! Socket 0 carries transaction commands; `len` includes the header.
//!
//! The only data ever written on socket 2 is `st 01 00 01`.

use std::io;
use std::time::{Duration, Instant};

/// EPSON-CTRL socket id as used by ecbd (`open_port_driver` → sid 2).
pub const CTRL_SID: u8 = 0x02;

/// "Epson packing" / EnterIEEE: a valid 1284.4 packet (cmd 0x45) that is
/// also the EJL string `@EJL 1284.4\n@EJL\n@EJL\n` in compatibility mode.
pub const ENTER: [u8; 27] = [
    0x00, 0x00, 0x00, 0x1B, 0x01, b'@', b'E', b'J', b'L', b' ', b'1', b'2', b'8', b'4', b'.',
    b'4', b'\n', b'@', b'E', b'J', b'L', b'\n', b'@', b'E', b'J', b'L', b'\n',
];
pub const INIT: [u8; 8] = [0x00, 0x00, 0x00, 0x08, 0x01, 0x00, 0x00, 0x10];
pub const EXIT: [u8; 7] = [0x00, 0x00, 0x00, 0x07, 0x01, 0x00, 0x08];
pub const STATUS_PAYLOAD: [u8; 5] = *b"st\x01\x00\x01";

pub fn open_channel(sid: u8) -> [u8; 17] {
    // PtS and StP packet sizes 0x0200, credit request 0, max credit 0.
    [0, 0, 0, 0x11, 0x01, 0x00, 0x01, sid, sid, 0x02, 0x00, 0x02, 0x00, 0, 0, 0, 0]
}
pub fn credit_request(sid: u8) -> [u8; 13] {
    // Ask for 0x0080 credits, max 0xFFFF (ECBT_Write).
    [0, 0, 0, 0x0D, 0x01, 0x00, 0x04, sid, sid, 0x00, 0x80, 0xFF, 0xFF]
}
pub fn credit(sid: u8, n: u16) -> [u8; 11] {
    let [h, l] = n.to_be_bytes();
    [0, 0, 0, 0x0B, 0x01, 0x00, 0x03, sid, sid, h, l]
}
pub fn close_channel(sid: u8) -> [u8; 10] {
    [0, 0, 0, 0x0A, 0x01, 0x00, 0x02, sid, sid, 0x00]
}
/// Data packet on `sid`: credit 0, control 1 (end of message).
pub fn data_packet(sid: u8, payload: &[u8]) -> Vec<u8> {
    let [h, l] = ((payload.len() + 6) as u16).to_be_bytes();
    let mut v = vec![sid, sid, h, l, 0x00, 0x01];
    v.extend_from_slice(payload);
    v
}

pub trait Port {
    fn write_all(&mut self, data: &[u8]) -> io::Result<()>;
    /// Read what is available, waiting up to `timeout`. Ok(0) = timed out.
    fn read_some(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub psid: u8,
    /// Whole packet, header included.
    pub raw: Vec<u8>,
}

impl Packet {
    pub fn payload(&self) -> &[u8] {
        &self.raw[6..]
    }
    /// Transaction command byte (socket 0 only).
    pub fn cmd(&self) -> Option<u8> {
        (self.psid == 0 && self.raw.len() > 6).then(|| self.raw[6])
    }
    pub fn result(&self) -> Option<u8> {
        (self.psid == 0 && self.raw.len() > 7).then(|| self.raw[7])
    }
}

fn err(msg: impl Into<String>) -> io::Error {
    io::Error::other(msg.into())
}

pub struct Session<P: Port> {
    port: P,
    rx: Vec<u8>,
    pending: Vec<Packet>,
    entered: bool,
    channel_open: bool,
    /// Credits the printer gave us for writing on CTRL_SID.
    tx_credit: u32,
    pub timeout: Duration,
}

impl<P: Port> Session<P> {
    pub fn new(port: P) -> Self {
        Session {
            port,
            rx: Vec::new(),
            pending: Vec::new(),
            entered: false,
            channel_open: false,
            tx_credit: 0,
            timeout: Duration::from_secs(2),
        }
    }

    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.port.write_all(bytes)
    }

    /// Read and discard stale bytes (ecbd `DummyRead_Fnc`).
    pub fn drain(&mut self) -> io::Result<()> {
        let mut total = 0;
        let mut buf = [0u8; 1024];
        loop {
            let n = self.port.read_some(&mut buf, Duration::from_millis(200))?;
            if n == 0 {
                break;
            }
            total += n;
            if total > 64 * 1024 {
                return Err(err("drain: device keeps sending"));
            }
        }
        Ok(())
    }

    /// Next whole packet from the wire, or None on timeout.
    fn next_packet(&mut self, deadline: Instant) -> io::Result<Option<Packet>> {
        loop {
            // Resync: a 1284.4 header never has length < 6. Drop junk bytes.
            let len = |rx: &[u8]| usize::from(u16::from_be_bytes([rx[2], rx[3]]));
            while self.rx.len() >= 4 && len(&self.rx) < 6 {
                self.rx.remove(0);
            }
            if self.rx.len() >= 6 {
                let n = len(&self.rx);
                if self.rx.len() >= n {
                    let raw: Vec<u8> = self.rx.drain(..n).collect();
                    return Ok(Some(Packet { psid: raw[0], raw }));
                }
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            let mut buf = [0u8; 1024];
            let n = self.port.read_some(&mut buf, left)?;
            self.rx.extend_from_slice(&buf[..n]);
        }
    }

    /// Answer printer-initiated transactions the way ecbd's
    /// `Check_CommandReply` does. Returns true if the packet was consumed.
    fn handle_unsolicited(&mut self, p: &Packet) -> io::Result<bool> {
        let Some(cmd) = p.cmd() else { return Ok(false) };
        let sid = p.raw.get(7).copied().unwrap_or(0);
        match cmd {
            0x02 => {
                // printer closes a channel
                if sid == CTRL_SID {
                    self.channel_open = false;
                }
                self.send(&[0, 0, 0, 0x0A, 0x01, 0x00, 0x82, 0x00, sid, sid])?;
                Ok(true)
            }
            0x03 => {
                // printer grants us credit
                let n = p.raw.get(9..11).map(|b| u16::from_be_bytes([b[0], b[1]])).unwrap_or(0);
                if sid == CTRL_SID {
                    self.tx_credit += n as u32;
                }
                self.send(&[0, 0, 0, 0x0A, 0x01, 0x00, 0x83, 0x00, sid, sid])?;
                Ok(true)
            }
            0x04 => {
                // printer asks us for credit: grant what it asked (ecbd does).
                let n = p.raw.get(9..11).map(|b| [b[0], b[1]]).unwrap_or([0, 0]);
                self.send(&[0, 0, 0, 0x0C, 0x01, 0x00, 0x84, 0x00, sid, sid, n[0], n[1]])?;
                Ok(true)
            }
            0x08 => {
                self.entered = false;
                self.channel_open = false;
                self.send(&[0, 0, 0, 0x08, 0x01, 0x00, 0x88, 0x00])?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Wait for the socket-0 reply `want` (command | 0x80).
    fn wait_reply(&mut self, want: u8, timeout: Duration) -> io::Result<Option<Packet>> {
        let deadline = Instant::now() + timeout;
        while let Some(p) = self.next_packet(deadline)? {
            if p.psid != 0 {
                self.pending.push(p);
                continue;
            }
            match p.cmd() {
                Some(c) if c == want => return Ok(Some(p)),
                Some(0x7F) => return Err(err(format!("printer sent 1284.4 error packet {:02X?}", p.raw))),
                _ => {
                    if !self.handle_unsolicited(&p)? {
                        return Err(err(format!("unexpected packet {:02X?}", p.raw)));
                    }
                }
            }
        }
        Ok(None)
    }

    fn transact(&mut self, bytes: &[u8], want: u8, timeout: Duration) -> io::Result<Packet> {
        self.send(bytes)?;
        self.wait_reply(want, timeout)?
            .ok_or_else(|| err(format!("no reply 0x{want:02X} within {timeout:?}")))
    }

    pub fn enter(&mut self) -> io::Result<()> {
        self.drain()?;
        self.send(&ENTER)?;
        let mut r = self.wait_reply(0xC5, self.timeout)?;
        if r.is_none() {
            // ecbd retries once and then waits 26 s.
            self.send(&ENTER)?;
            r = self.wait_reply(0xC5, self.timeout * 2)?;
        }
        let r = r.ok_or_else(|| err("no reply to EnterIEEE (printer not in D4 mode?)"))?;
        if r.result() != Some(0) {
            return Err(err(format!("EnterIEEE refused {:02X?}", r.raw)));
        }
        self.entered = true;
        Ok(())
    }

    pub fn init(&mut self) -> io::Result<()> {
        let r = self.transact(&INIT, 0x80, self.timeout)?;
        match r.result() {
            Some(0) => Ok(()),
            _ => Err(err(format!("Init refused {:02X?}", r.raw))),
        }
    }

    pub fn open(&mut self) -> io::Result<()> {
        let r = self.transact(&open_channel(CTRL_SID), 0x81, self.timeout)?;
        match r.result() {
            Some(0) | Some(6) => {
                self.channel_open = true;
                Ok(())
            }
            _ => Err(err(format!("OpenChannel refused {:02X?}", r.raw))),
        }
    }

    fn get_tx_credit(&mut self) -> io::Result<()> {
        for _ in 0..10 {
            if self.tx_credit > 0 {
                return Ok(());
            }
            let r = self.transact(&credit_request(CTRL_SID), 0x84, self.timeout)?;
            if r.result() != Some(0) {
                return Err(err(format!("CreditRequest refused {:02X?}", r.raw)));
            }
            let n = r.raw.get(10..12).map(|b| u16::from_be_bytes([b[0], b[1]])).unwrap_or(0);
            self.tx_credit += n as u32;
            if self.tx_credit == 0 {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        Err(err("printer gave no credit on the control channel"))
    }

    /// Send `st 01 00 01` and collect the reply on the control channel.
    pub fn status(&mut self) -> io::Result<Vec<u8>> {
        self.get_tx_credit()?;
        self.send(&data_packet(CTRL_SID, &STATUS_PAYLOAD))?;
        self.tx_credit -= 1;
        let mut reply = Vec::new();
        let deadline = Instant::now() + self.timeout * 2;
        for _ in 0..8 {
            // Give the printer one credit so it can send one data packet.
            let r = self.transact(&credit(CTRL_SID, 1), 0x83, self.timeout)?;
            if r.result() != Some(0) {
                return Err(err(format!("Credit refused {:02X?}", r.raw)));
            }
            let p = loop {
                if let Some(i) = self.pending.iter().position(|p| p.psid == CTRL_SID) {
                    break Some(self.pending.remove(i));
                }
                match self.next_packet(deadline)? {
                    None => break None,
                    Some(p) if p.psid == CTRL_SID => break Some(p),
                    Some(p) if p.psid == 0 => {
                        if p.cmd() == Some(0x7F) {
                            return Err(err(format!("1284.4 error packet {:02X?}", p.raw)));
                        }
                        self.handle_unsolicited(&p)?;
                    }
                    Some(p) => self.pending.push(p),
                }
            };
            let Some(p) = p else { break };
            reply.extend_from_slice(p.payload());
            if let Some(r) = super::st2::parse(&reply) {
                if r.complete() {
                    break;
                }
            } else if reply.starts_with(b"@BDC ST\r\n") && reply.ends_with(b";\x0C") {
                break;
            }
        }
        if reply.is_empty() {
            return Err(err("no data on the control channel"));
        }
        Ok(reply)
    }

    pub fn close(&mut self) -> io::Result<()> {
        if self.channel_open {
            self.channel_open = false;
            self.transact(&close_channel(CTRL_SID), 0x82, self.timeout)?;
        }
        Ok(())
    }

    /// Exit is sent only after EnterIEEE was acknowledged. When Enter gets no
    /// reply we cannot tell whether the printer switched to 1284.4 at all,
    /// and in compatibility mode the Exit packet would just be 7 bytes of
    /// print data. Gutenprint's escputil (`init_packet`) and ecbd also stop
    /// without Exit when Enter fails. If the printer did switch, it leaves
    /// 1284.4 again on the next job: every job (CUPS driver and our own
    /// maintenance jobs) starts with the "exit packet mode" preamble.
    pub fn exit(&mut self) -> io::Result<()> {
        if self.entered {
            self.entered = false;
            self.send(&EXIT)?;
            // ecbd ignores the result of Exit; so do we, but wait for it.
            let _ = self.wait_reply(0x88, self.timeout)?;
        }
        Ok(())
    }

    /// Whole status query: Enter, Init, OpenChannel, status, Close, Exit.
    /// Close and Exit also run when an earlier step fails.
    pub fn query(&mut self) -> io::Result<Vec<u8>> {
        let res = (|| {
            self.enter()?;
            self.init()?;
            self.open()?;
            self.status()
        })();
        let c = self.close();
        let e = self.exit();
        let reply = res?;
        // The reply is good even if closing failed; keep it and log.
        for err in [c.err(), e.err()].into_iter().flatten() {
            eprintln!("escp-utility: closing the 1284.4 session failed after a good reply: {err}");
        }
        Ok(reply)
    }
}

impl<P: Port> Drop for Session<P> {
    fn drop(&mut self) {
        // Never leave the printer in 1284.4 mode: CUPS sends raw data.
        self.timeout = Duration::from_secs(2);
        let _ = self.close();
        let _ = self.exit();
    }
}

#[cfg(test)]
pub mod mock {
    use super::*;
    use std::collections::VecDeque;

    /// Scripted printer: for each host write that starts with a key prefix,
    /// queue the given reply bytes.
    pub struct MockPrinter {
        pub script: Vec<(Vec<u8>, Vec<u8>)>,
        pub out: VecDeque<u8>,
        pub writes: Vec<Vec<u8>>,
    }

    impl MockPrinter {
        pub fn new(script: Vec<(Vec<u8>, Vec<u8>)>) -> Self {
            MockPrinter { script, out: VecDeque::new(), writes: vec![] }
        }
    }

    impl Port for MockPrinter {
        fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
            self.writes.push(data.to_vec());
            if let Some(i) = self.script.iter().position(|(k, _)| data.starts_with(k)) {
                let (_, reply) = self.script.remove(i);
                self.out.extend(reply);
            }
            Ok(())
        }
        fn read_some(&mut self, buf: &mut [u8], _t: Duration) -> io::Result<usize> {
            // Deliver in small pieces to exercise reassembly.
            let n = self.out.len().min(buf.len()).min(7);
            for b in buf.iter_mut().take(n) {
                *b = self.out.pop_front().unwrap();
            }
            Ok(n)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::st2;
    use super::mock::MockPrinter;
    use super::*;

    fn st2_sample() -> Vec<u8> {
        st2::tests::sample(0x04, &[0x0F, 0x04, 0x03, 0x0B, 0x00, 0x2A, 0x40, 0x02, b'Z', b'Z'])
    }

    /// Printer replies exactly as captured with usbmon on the live L800
    /// (ecbd session and this client's session were byte-identical).
    fn script_with(st2: Vec<u8>) -> Vec<(Vec<u8>, Vec<u8>)> {
        let mut data = vec![0x02, 0x02, 0x00, (st2.len() + 6) as u8, 0x00, 0x01];
        data.extend(&st2);
        vec![
            (ENTER.to_vec(), vec![0, 0, 0, 8, 1, 0, 0xC5, 0]),
            (INIT.to_vec(), vec![0, 0, 0, 9, 1, 0, 0x80, 0, 0x10]),
            (
                open_channel(2).to_vec(),
                vec![0, 0, 0, 0x10, 1, 0, 0x81, 0, 2, 2, 0x00, 0x40, 0x02, 0x00, 0, 0],
            ),
            (credit_request(2).to_vec(), vec![0, 0, 0, 0x0C, 1, 0, 0x84, 0, 2, 2, 0, 1]),
            (credit(2, 1).to_vec(), {
                let mut v = vec![0, 0, 0, 0x0A, 1, 0, 0x83, 0, 2, 2];
                v.extend(data);
                v
            }),
            (close_channel(2).to_vec(), vec![0, 0, 0, 0x0A, 1, 0, 0x82, 0, 2, 2]),
            (EXIT.to_vec(), vec![0, 0, 0, 8, 1, 0, 0x88, 0]),
        ]
    }

    fn script() -> Vec<(Vec<u8>, Vec<u8>)> {
        script_with(st2_sample())
    }

    #[test]
    fn replays_live_l800_session() {
        let live = st2::tests::live();
        let mut s = Session::new(MockPrinter::new(script_with(live.clone())));
        let reply = s.query().unwrap();
        assert_eq!(reply, live);
        // Host side of the live capture, in order.
        let expected: Vec<Vec<u8>> = vec![
            ENTER.to_vec(),
            INIT.to_vec(),
            open_channel(2).to_vec(),
            credit_request(2).to_vec(),
            vec![0x02, 0x02, 0x00, 0x0B, 0x00, 0x01, 0x73, 0x74, 0x01, 0x00, 0x01],
            vec![0x00, 0x00, 0x00, 0x0B, 0x01, 0x00, 0x03, 0x02, 0x02, 0x00, 0x01],
            vec![0x00, 0x00, 0x00, 0x0A, 0x01, 0x00, 0x02, 0x02, 0x02, 0x00],
            vec![0x00, 0x00, 0x00, 0x07, 0x01, 0x00, 0x08],
        ];
        assert_eq!(s.port.writes, expected);
    }

    #[test]
    fn full_status_query_against_mock() {
        let mut s = Session::new(MockPrinter::new(script()));
        let reply = s.query().unwrap();
        assert_eq!(reply, st2_sample());
        let r = st2::parse(&reply).unwrap();
        assert_eq!(r.get(st2::id::ST), Some(&[4][..]));
        let writes = &s.port.writes;
        assert_eq!(writes[0], ENTER.to_vec());
        assert_eq!(writes[1], INIT.to_vec());
        assert_eq!(writes[4], data_packet(2, b"st\x01\x00\x01"));
        assert_eq!(writes.last().unwrap(), &EXIT.to_vec());
        // Only one data packet is ever written, and it is the status request.
        let data_writes: Vec<_> = writes.iter().filter(|w| w[0] != 0).collect();
        assert_eq!(data_writes, vec![&data_packet(2, b"st\x01\x00\x01")]);
    }

    #[test]
    fn exit_runs_after_failure() {
        let mut sc = script();
        sc.retain(|(k, _)| k.as_slice() != open_channel(2).as_slice());
        let mut s = Session::new(MockPrinter::new(sc));
        s.timeout = Duration::from_millis(50);
        assert!(s.query().is_err());
        assert_eq!(s.port.writes.last().unwrap(), &EXIT.to_vec());
    }

    #[test]
    fn reply_kept_when_close_fails() {
        let mut sc = script();
        sc.retain(|(k, _)| k.as_slice() != close_channel(2).as_slice());
        let mut s = Session::new(MockPrinter::new(sc));
        s.timeout = Duration::from_millis(50);
        assert!(s.query().is_ok());
        assert_eq!(s.port.writes.last().unwrap(), &EXIT.to_vec());
    }

    #[test]
    fn no_exit_without_enter_reply() {
        let mut s = Session::new(MockPrinter::new(vec![]));
        s.timeout = Duration::from_millis(20);
        assert!(s.query().is_err());
        assert!(s.port.writes.iter().all(|w| w == &ENTER.to_vec()), "{:?}", s.port.writes);
    }

    #[test]
    fn packet_bytes_match_ecbd() {
        assert_eq!(&ENTER[5..], b"@EJL 1284.4\n@EJL\n@EJL\n");
        assert_eq!(data_packet(2, b"st\x01\x00\x01"), vec![2, 2, 0, 0x0B, 0, 1, 0x73, 0x74, 1, 0, 1]);
        assert_eq!(credit(2, 1), [0, 0, 0, 0x0B, 1, 0, 3, 2, 2, 0, 1]);
    }
}

#[cfg(test)]
mod session_tests {
    use super::mock::MockPrinter;
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    const ENTER_OK: [u8; 8] = [0, 0, 0, 8, 1, 0, 0xC5, 0];
    const INIT_OK: [u8; 9] = [0, 0, 0, 9, 1, 0, 0x80, 0, 0x10];

    fn quick(script: Vec<(Vec<u8>, Vec<u8>)>) -> Session<MockPrinter> {
        let mut s = Session::new(MockPrinter::new(script));
        s.timeout = Duration::from_millis(30);
        s
    }

    #[test]
    fn frame_length_fields_match_sizes() {
        let frames: Vec<Vec<u8>> = vec![
            ENTER.to_vec(),
            INIT.to_vec(),
            EXIT.to_vec(),
            open_channel(2).to_vec(),
            credit_request(2).to_vec(),
            credit(2, 0x1234).to_vec(),
            close_channel(2).to_vec(),
            data_packet(2, &STATUS_PAYLOAD),
        ];
        for f in frames {
            assert_eq!(usize::from(u16::from_be_bytes([f[2], f[3]])), f.len(), "{f:02X?}");
        }
        assert_eq!(credit(2, 0x1234)[9..], [0x12, 0x34]);
        assert_eq!(open_channel(5)[7..9], [5, 5]);
        assert_eq!(close_channel(5)[6..9], [0x02, 5, 5]);
    }

    #[test]
    fn packet_accessors() {
        let p = Packet { psid: 0, raw: vec![0, 0, 0, 8, 1, 0, 0xC5, 0] };
        assert_eq!((p.cmd(), p.result()), (Some(0xC5), Some(0)));
        let d = Packet { psid: 2, raw: vec![2, 2, 0, 7, 0, 1, b'x'] };
        assert_eq!((d.cmd(), d.result(), d.payload()), (None, None, &b"x"[..]));
        let short = Packet { psid: 0, raw: vec![0, 0, 0, 6, 1, 0] };
        assert_eq!((short.cmd(), short.result()), (None, None));
    }

    #[test]
    fn enter_resyncs_past_junk_bytes() {
        let mut reply = vec![0, 0, 0, 0, 0];
        reply.extend(ENTER_OK);
        let mut s = quick(vec![(ENTER.to_vec(), reply), (EXIT.to_vec(), vec![0, 0, 0, 8, 1, 0, 0x88, 0])]);
        s.enter().unwrap();
        s.exit().unwrap();
        assert_eq!(s.port.writes, vec![ENTER.to_vec(), EXIT.to_vec()]);
    }

    #[test]
    fn enter_retries_once_then_succeeds() {
        let mut s = quick(vec![(ENTER.to_vec(), vec![]), (ENTER.to_vec(), ENTER_OK.to_vec())]);
        s.enter().unwrap();
        assert_eq!(s.port.writes, vec![ENTER.to_vec(), ENTER.to_vec()]);
    }

    #[test]
    fn enter_refused_sends_no_exit() {
        let mut s = quick(vec![(ENTER.to_vec(), vec![0, 0, 0, 8, 1, 0, 0xC5, 1])]);
        assert!(s.query().is_err());
        assert_eq!(s.port.writes, vec![ENTER.to_vec()]);
    }

    #[test]
    fn error_packet_after_enter_still_exits() {
        let mut s = quick(vec![
            (ENTER.to_vec(), ENTER_OK.to_vec()),
            (INIT.to_vec(), vec![0, 0, 0, 0x0A, 1, 0, 0x7F, 0, 0, 0]),
        ]);
        let e = s.query().unwrap_err();
        assert!(e.to_string().contains("error packet"), "{e}");
        assert_eq!(s.port.writes.last().unwrap(), &EXIT.to_vec());
    }

    #[test]
    fn init_refused_is_error() {
        let mut s = quick(vec![(INIT.to_vec(), vec![0, 0, 0, 9, 1, 0, 0x80, 1, 0x10])]);
        s.entered = true;
        assert!(s.init().is_err());
    }

    #[test]
    fn open_accepts_result_6_and_close_sends_frame() {
        let mut s = quick(vec![
            (open_channel(2).to_vec(), vec![0, 0, 0, 0x10, 1, 0, 0x81, 6, 2, 2, 0, 0x40, 2, 0, 0, 0]),
            (close_channel(2).to_vec(), vec![0, 0, 0, 0x0A, 1, 0, 0x82, 0, 2, 2]),
        ]);
        s.open().unwrap();
        s.close().unwrap();
        // Second close is a no-op.
        s.close().unwrap();
        assert_eq!(s.port.writes, vec![open_channel(2).to_vec(), close_channel(2).to_vec()]);
    }

    #[test]
    fn unsolicited_credit_request_is_answered() {
        let mut reply = vec![0, 0, 0, 0x0D, 1, 0, 0x04, 2, 2, 0, 5, 0xFF, 0xFF];
        reply.extend(INIT_OK);
        let mut s = quick(vec![(INIT.to_vec(), reply)]);
        s.init().unwrap();
        assert_eq!(s.port.writes[1], vec![0, 0, 0, 0x0C, 1, 0, 0x84, 0, 2, 2, 0, 5]);
    }

    #[test]
    fn unsolicited_credit_grant_skips_credit_request() {
        let mut reply = vec![0, 0, 0, 0x0B, 1, 0, 0x03, 2, 2, 0, 1];
        reply.extend([0, 0, 0, 0x10, 1, 0, 0x81, 0, 2, 2, 0, 0x40, 2, 0, 0, 0]);
        // A complete ST2 reply in one data packet ends the status read.
        let st2 = super::super::st2::tests::sample(0x04, &[]);
        let mut cr = vec![0, 0, 0, 0x0A, 1, 0, 0x83, 0, 2, 2];
        cr.extend(data_packet(2, &st2));
        let mut s = quick(vec![(open_channel(2).to_vec(), reply), (credit(2, 1).to_vec(), cr)]);
        s.open().unwrap();
        assert_eq!(s.tx_credit, 1);
        assert_eq!(s.status().unwrap(), st2);
        assert!(!s.port.writes.contains(&credit_request(2).to_vec()));
        assert!(s.port.writes.contains(&vec![0, 0, 0, 0x0A, 1, 0, 0x83, 0, 2, 2]));
    }

    #[test]
    fn printer_exit_suppresses_host_exit() {
        let mut s = quick(vec![
            (ENTER.to_vec(), ENTER_OK.to_vec()),
            (INIT.to_vec(), vec![0, 0, 0, 7, 1, 0, 0x08]),
        ]);
        assert!(s.query().is_err());
        assert!(s.port.writes.contains(&vec![0, 0, 0, 8, 1, 0, 0x88, 0]));
        assert!(!s.port.writes.contains(&EXIT.to_vec()));
    }

    #[test]
    fn credit_request_zero_forever_fails() {
        let zero = vec![0, 0, 0, 0x0C, 1, 0, 0x84, 0, 2, 2, 0, 0];
        let mut s = quick((0..10).map(|_| (credit_request(2).to_vec(), zero.clone())).collect());
        s.timeout = Duration::from_millis(5);
        let e = s.status().unwrap_err();
        assert!(e.to_string().contains("no credit"), "{e}");
        assert_eq!(s.port.writes.len(), 10);
    }

    #[test]
    fn status_without_data_is_error() {
        let mut s = quick(vec![(credit(2, 1).to_vec(), vec![0, 0, 0, 0x0A, 1, 0, 0x83, 0, 2, 2])]);
        s.tx_credit = 1;
        assert!(s.status().is_err());
        assert_eq!(s.port.writes[0], data_packet(2, &STATUS_PAYLOAD));
    }

    #[test]
    fn exit_without_reply_is_ok() {
        let mut s = quick(vec![]);
        s.entered = true;
        s.exit().unwrap();
        assert_eq!(s.port.writes, vec![EXIT.to_vec()]);
    }

    struct Shared(Rc<RefCell<MockPrinter>>);
    impl Port for Shared {
        fn write_all(&mut self, d: &[u8]) -> io::Result<()> {
            self.0.borrow_mut().write_all(d)
        }
        fn read_some(&mut self, b: &mut [u8], t: Duration) -> io::Result<usize> {
            self.0.borrow_mut().read_some(b, t)
        }
    }

    #[test]
    fn drop_closes_and_exits_open_session() {
        let m = Rc::new(RefCell::new(MockPrinter::new(vec![
            (ENTER.to_vec(), ENTER_OK.to_vec()),
            (INIT.to_vec(), INIT_OK.to_vec()),
            (open_channel(2).to_vec(), vec![0, 0, 0, 0x10, 1, 0, 0x81, 0, 2, 2, 0, 0x40, 2, 0, 0, 0]),
            (close_channel(2).to_vec(), vec![0, 0, 0, 0x0A, 1, 0, 0x82, 0, 2, 2]),
            (EXIT.to_vec(), vec![0, 0, 0, 8, 1, 0, 0x88, 0]),
        ])));
        {
            let mut s = Session::new(Shared(m.clone()));
            s.timeout = Duration::from_millis(30);
            s.enter().unwrap();
            s.init().unwrap();
            s.open().unwrap();
        }
        let w = &m.borrow().writes;
        assert_eq!(w[w.len() - 2..], [close_channel(2).to_vec(), EXIT.to_vec()]);
    }
}

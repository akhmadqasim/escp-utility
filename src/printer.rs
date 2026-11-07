//! Which printer: the CUPS queues of Epson printers, and each one's
//! IEEE 1284 device ID.
//!
//! The ID decides what the app may do (phase2/MULTI-MODEL-PLAN.md §1):
//! maintenance jobs need `MFG:EPSON` and an ESC/P command set (`CMD`
//! ESCPL2 or ESCPR*), the way Epson's Linux utility accepts a printer
//! (rea/MULTI-MODEL.md §1.1). The status request needs the D4 (IEEE 1284.4)
//! channel, so `CMD` must list D4.
//!
//! Where the ID comes from, best first:
//! 1. USB: the kernel's cached copy in `/sys/class/usbmisc/lpN/device/ieee1284_id`
//!    (nothing is sent to the printer), for the `lpN` whose USB device
//!    serial (`device/../serial`) equals the queue URI's `?serial=`.
//! 2. The queue URI `usb://MFG/MDL?...` (printer switched off or unplugged).
//! 3. The queue PPD's `*1284DeviceID` (MFG and MDL only), from cupsd.
//!
//! Only read-only `lpstat`/`lpoptions` calls and one PPD download from
//! cupsd are made here; nothing reaches the printer.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the kernel lists the usblp devices.
const USBMISC: &str = "/sys/class/usbmisc";

/// A parsed IEEE 1284 device ID: `KEY:value;` fields in any order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceId {
    pub raw: String,
    fields: Vec<(String, String)>,
}

impl DeviceId {
    pub fn parse(raw: &str) -> DeviceId {
        let fields = raw
            .split(';')
            .filter_map(|f| f.split_once(':'))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect();
        DeviceId { raw: raw.trim().to_string(), fields }
    }

    /// The value of the first field with one of these keys.
    fn get(&self, keys: &[&str]) -> Option<&str> {
        self.fields.iter().find(|(k, _)| keys.contains(&k.as_str())).map(|(_, v)| v.as_str())
    }

    pub fn mfg(&self) -> Option<&str> {
        self.get(&["MFG", "MANUFACTURER"])
    }

    pub fn mdl(&self) -> Option<&str> {
        self.get(&["MDL", "MODEL"])
    }

    /// The command sets (`CMD`), e.g. ESCPL2, BDC, D4, D4PX on the L800.
    pub fn cmd(&self) -> Vec<&str> {
        self.get(&["CMD", "COMMAND SET"]).map_or(vec![], |c| c.split(',').map(str::trim).collect())
    }

    /// An Epson that speaks ESC/P: Epson's Linux utility gives such a
    /// printer Nozzle Check and Head Cleaning (CMD ESCPL2 or ESCPR*).
    pub fn maintenance(&self) -> bool {
        self.mfg().is_some_and(|m| m.eq_ignore_ascii_case("EPSON"))
            && self.cmd().iter().any(|c| *c == "ESCPL2" || c.starts_with("ESCPR"))
    }

    /// The printer has the IEEE 1284.4 (D4) channel that the status request uses.
    pub fn status(&self) -> bool {
        self.cmd().contains(&"D4")
    }
}

/// A CUPS queue: `device for NAME: URI` from `lpstat -v`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queue {
    pub name: String,
    pub uri: String,
}

/// Parse `LC_ALL=C lpstat -v` output.
pub fn parse_lpstat_v(text: &str) -> Vec<Queue> {
    text.lines()
        .filter_map(|l| l.strip_prefix("device for ")?.split_once(": "))
        .map(|(name, uri)| Queue { name: name.to_string(), uri: uri.trim().to_string() })
        .collect()
}

/// The CUPS queues (`lpstat -v`, or only `queue`'s line). Empty when cupsd
/// cannot be asked.
pub fn queues(queue: Option<&str>) -> Vec<Queue> {
    let mut cmd = Command::new("lpstat");
    // C locale: "device for" is translated in other locales.
    cmd.env("LC_ALL", "C").arg("-v");
    cmd.args(queue);
    match cmd.output() {
        Ok(o) => parse_lpstat_v(&String::from_utf8_lossy(&o.stdout)),
        Err(e) => {
            eprintln!("epson-l800-utility: cannot run lpstat: {e}");
            vec![]
        }
    }
}

/// `printer-make-and-model` from `lpoptions -p QUEUE`, or "".
pub fn make_model(queue: &str) -> String {
    let out = Command::new("lpoptions").env("LC_ALL", "C").args(["-p", queue]).output();
    let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    lpoptions_value(&text, "printer-make-and-model").unwrap_or_default()
}

/// One `key=value` of `lpoptions -p` output; values with spaces are quoted
/// ('...' or "..."), and a backslash escapes the next character.
pub fn lpoptions_value(text: &str, key: &str) -> Option<String> {
    let mut chars = text.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        chars.peek()?;
        let mut token = String::new();
        let mut quote = None;
        while let Some(c) = chars.next() {
            match (c, quote) {
                ('\\', _) => token.extend(chars.next()),
                (c, None) if c.is_whitespace() => break,
                ('\'' | '"', None) => quote = Some(c),
                (c, Some(q)) if c == q => quote = None,
                (c, _) => token.push(c),
            }
        }
        if let Some(v) = token.strip_prefix(key).and_then(|r| r.strip_prefix('=')) {
            return Some(v.to_string());
        }
    }
}

/// An Epson printer: by its device URI or its make and model.
pub fn is_epson(q: &Queue, make_model: &str) -> bool {
    q.uri.to_ascii_lowercase().contains("epson") || make_model.to_ascii_lowercase().starts_with("epson")
}

/// The Epson queues, with their make and model.
pub fn epson_queues() -> Vec<(Queue, String)> {
    queues(None)
        .into_iter()
        .map(|q| {
            let mm = make_model(&q.name);
            (q, mm)
        })
        .filter(|(q, mm)| is_epson(q, mm))
        .collect()
}

/// `%XX` decoding (URIs).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = b.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok());
        match (b[i], hex) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A query parameter of a URI, decoded.
fn uri_param(uri: &str, key: &str) -> Option<String> {
    let query = uri.split_once('?')?.1;
    query.split('&').find_map(|p| p.strip_prefix(key)?.strip_prefix('=')).map(percent_decode)
}

/// `usb://EPSON/L800?serial=...` gives `MFG:EPSON;MDL:L800;`.
fn uri_device_id(uri: &str) -> Option<DeviceId> {
    let rest = uri.strip_prefix("usb://")?;
    let rest = rest.split('?').next()?;
    let (mfg, mdl) = rest.split_once('/')?;
    Some(DeviceId::parse(&format!("MFG:{};MDL:{};", percent_decode(mfg), percent_decode(mdl))))
}

/// The usblp device whose USB serial is `serial`: its node `/dev/usb/lpN`
/// and its cached IEEE 1284 ID. `root` is `/sys/class/usbmisc` (tests use
/// a fake tree).
pub fn usb_match(root: &Path, serial: &str) -> Option<(PathBuf, String)> {
    for e in fs::read_dir(root).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("lp") {
            continue;
        }
        let dev = e.path().join("device");
        let found = fs::read_to_string(dev.join("../serial")).unwrap_or_default();
        if found.trim() == serial {
            let id = fs::read_to_string(dev.join("ieee1284_id")).ok()?;
            return Some((PathBuf::from("/dev/usb").join(name), id));
        }
    }
    None
}

/// The PPD of a queue, as cupsd serves it (`GET /printers/QUEUE.ppd` on
/// localhost:631; /etc/cups/ppd is not readable for users).
pub fn cups_ppd(queue: &str) -> Option<String> {
    use std::io::{Read, Write};
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;
    let path: String = queue
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    let addrs = ("localhost", 631).to_socket_addrs().ok()?;
    let mut s = addrs.into_iter().find_map(|a| TcpStream::connect_timeout(&a, Duration::from_secs(2)).ok())?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    write!(s, "GET /printers/{path}.ppd HTTP/1.0\r\nHost: localhost\r\n\r\n").ok()?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let (head, body) = text.split_once("\r\n\r\n")?;
    (head.split_whitespace().nth(1) == Some("200")).then(|| body.to_string())
}

/// A quoted PPD main keyword value, such as `*NickName: "..."`.
pub fn ppd_value(ppd: &str, keyword: &str) -> Option<String> {
    let prefix = format!("*{keyword}:");
    let line = ppd.lines().find_map(|l| l.strip_prefix(&prefix))?;
    let v = line.trim();
    Some(v.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(v).to_string())
}

/// Where the device ID came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    Usb,
    Uri,
    Ppd,
    #[default]
    None,
}

/// The printer behind a queue.
#[derive(Debug, Clone, Default)]
pub struct Identity {
    pub queue: String,
    pub uri: String,
    pub make_model: String,
    pub id: DeviceId,
    pub source: Source,
    /// USB serial from the URI: finds the usblp node again at each query
    /// (its number changes when the printer is plugged in again).
    pub serial: Option<String>,
}

impl Identity {
    /// Identify the printer of `queue` from the sources in the module comment.
    pub fn of(queue: &str) -> Identity {
        let uri = queues(Some(queue)).into_iter().find(|q| q.name == queue).map(|q| q.uri).unwrap_or_default();
        Identity::from_parts(queue, uri, make_model(queue), Path::new(USBMISC), || cups_ppd(queue))
    }

    fn from_parts(queue: &str, uri: String, make_model: String, usbmisc: &Path, ppd: impl FnOnce() -> Option<String>) -> Identity {
        let serial = uri.starts_with("usb://").then(|| uri_param(&uri, "serial")).flatten();
        let (id, source) = if let Some((_, raw)) = serial.as_deref().and_then(|s| usb_match(usbmisc, s)) {
            (DeviceId::parse(&raw), Source::Usb)
        } else if let Some(id) = uri_device_id(&uri) {
            (id, Source::Uri)
        } else if let Some(raw) = ppd().and_then(|p| ppd_value(&p, "1284DeviceID")) {
            (DeviceId::parse(&raw), Source::Ppd)
        } else {
            (DeviceId::default(), Source::None)
        };
        Identity { queue: queue.to_string(), uri, make_model, id, source, serial }
    }

    /// What the app knows about the printer, one item per line.
    pub fn summary(&self) -> String {
        let source = match self.source {
            Source::Usb => "USB (kernel cache)",
            Source::Uri => "device URI",
            Source::Ppd => "PPD",
            Source::None => "none",
        };
        let yes = |b: bool| if b { "yes" } else { "no" };
        format!(
            "Make and model: {}\nDevice URI: {}\nIEEE 1284 device ID (from {source}): {}\nEpson ESC/P (CMD ESCPL2 or ESCPR): {}\nStatus channel (CMD D4): {}\n",
            self.make_model,
            self.uri,
            self.id.raw,
            yes(self.id.maintenance()),
            yes(self.id.status()),
        )
    }

    /// The usblp node for the status request: the printer is connected now
    /// and its live ID lists D4.
    pub fn status_node(&self) -> Option<PathBuf> {
        self.status_node_in(Path::new(USBMISC))
    }

    fn status_node_in(&self, usbmisc: &Path) -> Option<PathBuf> {
        let (node, raw) = usb_match(usbmisc, self.serial.as_deref()?)?;
        DeviceId::parse(&raw).status().then_some(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const L800_ID: &str = "MFG:EPSON;CMD:ESCPL2,BDC,D4,D4PX;MDL:L800;CLS:PRINTER;DES:EPSON L800;CID:EpsonStd2;";

    #[test]
    fn l800_id() {
        let id = DeviceId::parse(L800_ID);
        assert_eq!(id.mfg(), Some("EPSON"));
        assert_eq!(id.mdl(), Some("L800"));
        assert_eq!(id.cmd(), vec!["ESCPL2", "BDC", "D4", "D4PX"]);
        assert!(id.maintenance());
        assert!(id.status());
    }

    #[test]
    fn reordered_and_long_keys() {
        let id = DeviceId::parse("CLS:PRINTER;MODEL:ET-2750 Series;COMMAND SET:ESCPR1,BDC,D4; MANUFACTURER:EPSON;");
        assert_eq!(id.mdl(), Some("ET-2750 Series"));
        assert_eq!(id.mfg(), Some("EPSON"));
        assert!(id.maintenance());
        assert!(id.status());
    }

    #[test]
    fn gates() {
        // No CMD (URI or PPD): neither gate.
        let id = DeviceId::parse("MFG:EPSON;MDL:L800 Series;DES:EPSON L800 Series;");
        assert!(!id.maintenance());
        assert!(!id.status());
        // ESC/P-R without D4: maintenance, no status.
        assert!(DeviceId::parse("MFG:EPSON;CMD:ESCPR1;MDL:X;").maintenance());
        assert!(!DeviceId::parse("MFG:EPSON;CMD:ESCPR1;MDL:X;").status());
        // A page printer, and another maker.
        assert!(!DeviceId::parse("MFG:EPSON;CMD:ESCPAGE,D4;MDL:AL-M200;").maintenance());
        assert!(!DeviceId::parse("MFG:HP;CMD:ESCPL2,D4;MDL:X;").maintenance());
        // D4PX alone is not D4.
        assert!(!DeviceId::parse("MFG:EPSON;CMD:ESCPL2,D4PX;").status());
    }

    #[test]
    fn lpstat_samples() {
        let text = "device for L800: usb://EPSON/L800?serial=1234567X8901234567\n\
                    device for ET2750: dnssd://EPSON%20ET-2750%20Series._ipp._tcp.local/?uuid=cfe92100-67c4-11d4-a45f-64eb8c3d5a9e\n\
                    device for Office: ipp://192.168.1.20/ipp/print\n\
                    device for WF: socket://192.168.1.30:9100\n";
        let q = parse_lpstat_v(text);
        assert_eq!(q.len(), 4);
        assert_eq!(q[0], Queue { name: "L800".into(), uri: "usb://EPSON/L800?serial=1234567X8901234567".into() });
        assert_eq!(q[2].uri, "ipp://192.168.1.20/ipp/print");
        assert!(is_epson(&q[0], ""));
        assert!(is_epson(&q[1], ""));
        assert!(!is_epson(&q[2], "HP OfficeJet"));
        assert!(is_epson(&q[3], "Epson WF-4630 Series"));
        assert_eq!(uri_param(&q[0].uri, "serial").as_deref(), Some("1234567X8901234567"));
        assert_eq!(uri_device_id(&q[0].uri).unwrap().mdl(), Some("L800"));
        assert_eq!(uri_device_id("usb://EPSON/ET-2750%20Series?serial=X&interface=1").unwrap().mdl(), Some("ET-2750 Series"));
        assert_eq!(uri_device_id(&q[1].uri), None);
    }

    #[test]
    fn lpoptions_values() {
        let text = "copies=1 device-uri=usb://EPSON/L800?serial=1234567X8901234567 printer-info=L800 printer-location printer-make-and-model='Epson L800 Series - epson-inkjet-printer 1.0.2-1 (Seiko Epson Corporation)' printer-state=3\n";
        assert_eq!(
            lpoptions_value(text, "printer-make-and-model").as_deref(),
            Some("Epson L800 Series - epson-inkjet-printer 1.0.2-1 (Seiko Epson Corporation)")
        );
        assert_eq!(lpoptions_value(text, "printer-state").as_deref(), Some("3"));
        assert_eq!(lpoptions_value(text, "printer"), None);
        assert_eq!(lpoptions_value(r"a=it\'s b='x y'", "a").as_deref(), Some("it's"));
    }

    #[test]
    fn ppd_values() {
        let ppd = "*PPD-Adobe: \"4.3\"\n*NickName:      \"Epson L800 Series - epson-inkjet-printer 1.0.2-1 (Seiko Epson Corporation)\"\n*1284DeviceID: \"MFG:EPSON;MDL:L800 Series;DES:EPSON L800 Series;\"\n";
        assert_eq!(ppd_value(ppd, "1284DeviceID").as_deref(), Some("MFG:EPSON;MDL:L800 Series;DES:EPSON L800 Series;"));
        assert!(ppd_value(ppd, "NickName").unwrap().starts_with("Epson L800 Series"));
        assert_eq!(ppd_value(ppd, "ModelName"), None);
    }

    /// A fake /sys/class/usbmisc: lp0 is another printer, lp3 the L800.
    fn fake_sysfs() -> PathBuf {
        let root = std::env::temp_dir().join(format!("usbmisc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (lp, serial, id) in [("lp0", "OTHER", "MFG:HP;MDL:X;"), ("lp3", "1234567X8901234567", L800_ID)] {
            // lpN/device is the USB interface; its parent is the USB device.
            let usbdev = root.join(format!("{lp}-usbdev"));
            fs::create_dir_all(usbdev.join("iface")).unwrap();
            fs::write(usbdev.join("serial"), format!("{serial}\n")).unwrap();
            fs::write(usbdev.join("iface/ieee1284_id"), id).unwrap();
            fs::create_dir_all(root.join(lp)).unwrap();
            std::os::unix::fs::symlink(usbdev.join("iface"), root.join(lp).join("device")).unwrap();
        }
        root
    }

    #[test]
    fn usb_identity_from_sysfs() {
        let root = fake_sysfs();
        let uri = "usb://EPSON/L800?serial=1234567X8901234567".to_string();
        let i = Identity::from_parts("L800", uri, "Epson L800 Series".into(), &root, || panic!("no PPD needed"));
        assert_eq!(i.source, Source::Usb);
        assert_eq!(i.id.raw, L800_ID);
        assert!(i.id.maintenance());
        assert_eq!(i.status_node_in(&root), Some(PathBuf::from("/dev/usb/lp3")));
        // Unplugged: the URI still names the model, but there is no node.
        let i = Identity::from_parts("L800", "usb://EPSON/L800?serial=GONE".into(), String::new(), &root, || None);
        assert_eq!((i.source, i.id.mdl()), (Source::Uri, Some("L800")));
        assert!(!i.id.maintenance(), "no CMD: only a profile allows maintenance");
        assert_eq!(i.status_node_in(&root), None);
        // A network queue: the PPD's ID, no node.
        let ppd = || Some("*1284DeviceID: \"MFG:EPSON;MDL:ET-2750 Series;\"\n".to_string());
        let i = Identity::from_parts("ET", "ipp://10.0.0.5/ipp/print".into(), String::new(), &root, ppd);
        assert_eq!((i.source, i.id.mdl()), (Source::Ppd, Some("ET-2750 Series")));
        assert!(!i.id.maintenance());
        assert_eq!(i.status_node_in(&root), None);
        fs::remove_dir_all(root).unwrap();
    }
}

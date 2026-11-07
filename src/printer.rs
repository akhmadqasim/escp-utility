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
//! 3. The queue PPD's `*1284DeviceID` (MFG and MDL only), from cupsd
//!    (`ppd::fetch`, done by the caller).
//!
//! Only read-only `lpstat`/`lpoptions` calls are made here; nothing reaches
//! the printer.

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

    /// The value of the first field with one of these keys (any case, as
    /// CUPS reads the keys).
    fn get(&self, keys: &[&str]) -> Option<&str> {
        self.fields.iter().find(|(k, _)| keys.iter().any(|w| k.eq_ignore_ascii_case(w))).map(|(_, v)| v.as_str())
    }

    /// The serial number field, which CUPS puts in a usb:// URI before the
    /// USB descriptor's (backend/usb-libusb.c `make_device_uri`).
    fn serial(&self) -> Option<&str> {
        self.get(&["SERIALNUMBER", "SERN", "SN"]).filter(|s| !s.is_empty())
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
    /// printer Nozzle Check and Head Cleaning (CMD ESCPL2* or ESCPR*; older
    /// models report `ESCPL2-00`).
    pub fn maintenance(&self) -> bool {
        self.mfg().is_some_and(|m| m.eq_ignore_ascii_case("EPSON"))
            && self.cmd().iter().any(|c| c.starts_with("ESCPL2") || c.starts_with("ESCPR"))
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
            eprintln!("escp-utility: cannot run lpstat: {e}");
            vec![]
        }
    }
}

/// `printer-make-and-model` from `lpoptions -p QUEUE`, or "".
pub fn make_model(queue: &str) -> String {
    let text = crate::settings::lpoptions(&["-p", queue]).unwrap_or_default();
    crate::settings::parse_dest_options(&text).remove("printer-make-and-model").unwrap_or_default()
}

/// An Epson printer: by its device URI or its make and model.
pub fn is_epson(q: &Queue, make_model: &str) -> bool {
    q.uri.to_ascii_lowercase().contains("epson") || make_model.to_ascii_lowercase().starts_with("epson")
}

/// The Epson queues, with their make and model. One `lpoptions` per queue,
/// run side by side (each can take a second).
pub fn epson_queues() -> Vec<(Queue, String)> {
    let qs = queues(None);
    let mms: Vec<String> = std::thread::scope(|s| {
        let handles: Vec<_> = qs.iter().map(|q| s.spawn(|| make_model(&q.name))).collect();
        handles.into_iter().map(|h| h.join().unwrap_or_default()).collect()
    });
    qs.into_iter().zip(mms).filter(|(q, mm)| is_epson(q, mm)).collect()
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

/// The usblp device whose serial is `serial` (the USB descriptor's, or the
/// SN field of its IEEE 1284 ID, which CUPS prefers for the URI): its node
/// `/dev/usb/lpN` and its cached IEEE 1284 ID. `root` is
/// `/sys/class/usbmisc` (tests use a fake tree).
pub fn usb_match(root: &Path, serial: &str) -> Option<(PathBuf, String)> {
    // An empty serial would match every device that has none.
    if serial.is_empty() {
        return None;
    }
    for e in fs::read_dir(root).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("lp") {
            continue;
        }
        let dev = e.path().join("device");
        let Ok(id) = fs::read_to_string(dev.join("ieee1284_id")) else { continue };
        let found = fs::read_to_string(dev.join("../serial")).unwrap_or_default();
        if found.trim() == serial || DeviceId::parse(&id).serial() == Some(serial) {
            return Some((PathBuf::from("/dev/usb").join(name), id));
        }
    }
    None
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
    /// Identify the printer of `queue` from the sources in the module
    /// comment; `ppd_id` is the queue PPD's `*1284DeviceID`, if any, and
    /// `nickname` its `*NickName`, which cupsd reports as
    /// printer-make-and-model (without one, `lpoptions -p` is asked).
    pub fn of(queue: &str, ppd_id: Option<&str>, nickname: Option<&str>) -> Identity {
        let uri = queues(Some(queue)).into_iter().find(|q| q.name == queue).map(|q| q.uri).unwrap_or_default();
        let make_model = nickname.map_or_else(|| make_model(queue), str::to_string);
        Identity::from_parts(queue, uri, make_model, Path::new(USBMISC), ppd_id)
    }

    fn from_parts(queue: &str, uri: String, make_model: String, usbmisc: &Path, ppd_id: Option<&str>) -> Identity {
        let serial = uri.starts_with("usb://").then(|| uri_param(&uri, "serial")).flatten();
        let (id, source) = if let Some((_, raw)) = serial.as_deref().and_then(|s| usb_match(usbmisc, s)) {
            (DeviceId::parse(&raw), Source::Usb)
        } else if let Some(id) = uri_device_id(&uri) {
            (id, Source::Uri)
        } else if let Some(raw) = ppd_id {
            (DeviceId::parse(raw), Source::Ppd)
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
        // Older models' ESCPL2-00, and keys in another case.
        assert!(DeviceId::parse("MFG:EPSON;CMD:ESC,ESCPL2-00,BDC,D4;MDL:Stylus Photo R220;").maintenance());
        assert!(DeviceId::parse("Mfg:EPSON;Cmd:ESCPR1,D4;Mdl:X;").status());
    }

    #[test]
    fn lower_case_keys_and_aliases() {
        let id = DeviceId::parse("manufacturer:EPSON;command set:ESCPL2,D4;model:L800;");
        assert_eq!(id.mfg(), Some("EPSON"));
        assert_eq!(id.mdl(), Some("L800"));
        assert!(id.maintenance());
        assert!(id.status());
        // MFG value compared in any case.
        assert!(DeviceId::parse("mfg:Epson;cmd:ESCPR1;").maintenance());
    }

    #[test]
    fn whitespace_and_semicolons() {
        let id = DeviceId::parse("  MFG : EPSON ;; CMD: ESCPL2 , BDC , D4 ;MDL:L800 Series;;;\n");
        assert_eq!(id.raw, "MFG : EPSON ;; CMD: ESCPL2 , BDC , D4 ;MDL:L800 Series;;;");
        assert_eq!(id.mfg(), Some("EPSON"));
        assert_eq!(id.mdl(), Some("L800 Series"));
        assert_eq!(id.cmd(), vec!["ESCPL2", "BDC", "D4"]);
        assert!(id.maintenance() && id.status());
        // No trailing semicolon at all.
        assert_eq!(DeviceId::parse("MFG:EPSON;CMD:ESCPL2,D4").cmd(), vec!["ESCPL2", "D4"]);
    }

    #[test]
    fn empty_and_garbage() {
        for raw in ["", "   ", ";;;", "no colon here"] {
            let id = DeviceId::parse(raw);
            assert_eq!(id.mfg(), None);
            assert_eq!(id.mdl(), None);
            assert!(id.cmd().is_empty());
            assert!(id.serial().is_none());
            assert!(!id.maintenance());
            assert!(!id.status());
        }
    }

    #[test]
    fn value_keeps_inner_colons_and_first_key_wins() {
        let id = DeviceId::parse("DES:EPSON: L800;MFG:EPSON;MFG:HP;");
        assert_eq!(id.get(&["DES"]), Some("EPSON: L800"));
        assert_eq!(id.mfg(), Some("EPSON"));
    }

    #[test]
    fn serial_aliases() {
        assert_eq!(DeviceId::parse("MFG:EPSON;SN:X3Y4012345;").serial(), Some("X3Y4012345"));
        assert_eq!(DeviceId::parse("SERN:ABC;MFG:EPSON;").serial(), Some("ABC"));
        assert_eq!(DeviceId::parse("serialnumber: S1 ;").serial(), Some("S1"));
        // An empty serial is no serial.
        assert_eq!(DeviceId::parse("MFG:EPSON;SN:;").serial(), None);
        assert_eq!(DeviceId::parse(L800_ID).serial(), None);
    }

    #[test]
    fn status_gate_needs_exact_d4() {
        assert!(DeviceId::parse("MFG:EPSON;CMD:D4;").status());
        assert!(!DeviceId::parse("MFG:EPSON;CMD:ESCPL2,BDC;").status());
        assert!(!DeviceId::parse("MFG:EPSON;CMD:ESCPL2,D4X;").status());
        // Status does not need MFG:EPSON; maintenance does.
        let hp = DeviceId::parse("MFG:HP;CMD:PCL,D4;");
        assert!(hp.status());
        assert!(!hp.maintenance());
        // Missing MFG blocks maintenance.
        assert!(!DeviceId::parse("CMD:ESCPL2,D4;MDL:L800;").maintenance());
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

    /// A fake /sys/class/usbmisc: lp0 is another printer, lp3 the L800.
    fn fake_sysfs() -> PathBuf {
        sysfs_with("l800", &[("lp0", Some("OTHER\n"), "MFG:HP;MDL:X;"), ("lp3", Some("1234567X8901234567\n"), L800_ID)])
    }

    #[test]
    fn usb_identity_from_sysfs() {
        let root = fake_sysfs();
        let uri = "usb://EPSON/L800?serial=1234567X8901234567".to_string();
        let i = Identity::from_parts("L800", uri, "Epson L800 Series".into(), &root, Some("MFG:EPSON;MDL:wrong;"));
        assert_eq!(i.source, Source::Usb);
        assert_eq!(i.id.raw, L800_ID);
        assert!(i.id.maintenance());
        assert_eq!(i.status_node_in(&root), Some(PathBuf::from("/dev/usb/lp3")));
        // Unplugged: the URI still names the model, but there is no node.
        let i = Identity::from_parts("L800", "usb://EPSON/L800?serial=GONE".into(), String::new(), &root, None);
        assert_eq!((i.source, i.id.mdl()), (Source::Uri, Some("L800")));
        assert!(!i.id.maintenance(), "no CMD: only a profile allows maintenance");
        assert_eq!(i.status_node_in(&root), None);
        // A network queue: the PPD's ID, no node.
        let i = Identity::from_parts("ET", "ipp://10.0.0.5/ipp/print".into(), String::new(), &root, Some("MFG:EPSON;MDL:ET-2750 Series;"));
        assert_eq!((i.source, i.id.mdl()), (Source::Ppd, Some("ET-2750 Series")));
        assert!(!i.id.maintenance());
        assert_eq!(i.status_node_in(&root), None);
        // An empty serial matches nothing (lp0 has no serial file below).
        fs::remove_file(root.join("lp0-usbdev/serial")).unwrap();
        assert_eq!(usb_match(&root, ""), None);
        // CUPS takes the URI serial from the ID's SN field when it has one.
        fs::write(root.join("lp0-usbdev/iface/ieee1284_id"), "MFG:EPSON;CMD:ESCPR1,D4;MDL:X;SN:IDSERIAL1;").unwrap();
        assert_eq!(usb_match(&root, "IDSERIAL1").map(|m| m.0), Some(PathBuf::from("/dev/usb/lp0")));
        fs::remove_dir_all(root).unwrap();
    }

    /// A fake /sys/class/usbmisc under its own name: (lpN, USB serial or None, ID).
    fn sysfs_with(tag: &str, devs: &[(&str, Option<&str>, &str)]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("usbmisc-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        for (lp, serial, id) in devs {
            // lpN/device is the USB interface; its parent is the USB device.
            let usbdev = root.join(format!("{lp}-usbdev"));
            fs::create_dir_all(usbdev.join("iface")).unwrap();
            if let Some(s) = serial {
                fs::write(usbdev.join("serial"), s).unwrap();
            }
            fs::write(usbdev.join("iface/ieee1284_id"), id).unwrap();
            fs::create_dir_all(root.join(lp)).unwrap();
            std::os::unix::fs::symlink(usbdev.join("iface"), root.join(lp).join("device")).unwrap();
        }
        root
    }

    #[test]
    fn lpstat_queue_names_and_uri_forms() {
        let text = "device for EPSON_L800-Series: usb://EPSON/L800%20Series?serial=ABC&interface=1\r\n\
                    device for my-office_printer2: ipp://cups.local:631/printers/x\n\
                    device for Dummy: file:///dev/null  \n\
                    device for L805: dnssd://EPSON%20L805._pdl-datastream._tcp.local/\n";
        let q = parse_lpstat_v(text);
        let names: Vec<&str> = q.iter().map(|q| q.name.as_str()).collect();
        assert_eq!(names, ["EPSON_L800-Series", "my-office_printer2", "Dummy", "L805"]);
        // CR and trailing blanks are trimmed; the port's ':' stays in the URI.
        assert_eq!(q[0].uri, "usb://EPSON/L800%20Series?serial=ABC&interface=1");
        assert_eq!(q[1].uri, "ipp://cups.local:631/printers/x");
        assert_eq!(q[2].uri, "file:///dev/null");
        assert_eq!(uri_device_id(&q[0].uri).unwrap().mdl(), Some("L800 Series"));
        assert!(is_epson(&q[3], ""));
        assert!(!is_epson(&q[2], ""));
    }

    #[test]
    fn lpstat_ignores_other_lines_and_locales() {
        // Localized output (why queues() sets LC_ALL=C) and noise give nothing.
        let text = "Gerät für L800: usb://EPSON/L800?serial=X\n\
                    périphérique pour L800 : usb://EPSON/L800\n\
                    system default destination: L800\n\
                    device for NoSep:usb://EPSON/L800\n\
                    \n";
        assert_eq!(parse_lpstat_v(text), vec![]);
        assert_eq!(parse_lpstat_v(""), vec![]);
        // Indented lines are not lpstat's "device for" lines either.
        assert_eq!(parse_lpstat_v("  device for L800: usb://EPSON/L800\n"), vec![]);
    }

    #[test]
    fn uri_serial_param() {
        assert_eq!(uri_param("usb://EPSON/L800?interface=1&serial=S1", "serial").as_deref(), Some("S1"));
        // Look-alike keys are not "serial".
        assert_eq!(uri_param("usb://EPSON/L800?xserial=A&serialx=B&serial=C", "serial").as_deref(), Some("C"));
        assert_eq!(uri_param("usb://EPSON/L800?serial=S%2F1", "serial").as_deref(), Some("S/1"));
        assert_eq!(uri_param("usb://EPSON/L800", "serial"), None);
        assert_eq!(uri_param("usb://EPSON/L800?serial=", "serial").as_deref(), Some(""));
        // Only usb:// URIs carry MFG/MDL.
        assert_eq!(uri_device_id("ipp://EPSON/L800"), None);
        assert_eq!(uri_device_id("usb://EPSON"), None);
    }

    #[test]
    fn usb_match_serial_rules() {
        let root = sysfs_with(
            "rules",
            &[
                ("lp0", None, "MFG:EPSON;CMD:ESCPL2,D4;MDL:NoSerial;"),
                ("lp1", Some("  SER1 \n"), L800_ID),
                ("hiddev0", Some("SER2"), L800_ID),
            ],
        );
        // An empty serial never matches, not even lp0 with no serial at all.
        assert_eq!(usb_match(&root, ""), None);
        // The sysfs serial is trimmed; the match is exact (no prefix).
        assert_eq!(usb_match(&root, "SER1").map(|m| m.0), Some(PathBuf::from("/dev/usb/lp1")));
        assert_eq!(usb_match(&root, "SER"), None);
        assert_eq!(usb_match(&root, "ser1"), None);
        // Only lpN entries count.
        assert_eq!(usb_match(&root, "SER2"), None);
        // No usbmisc directory at all.
        assert_eq!(usb_match(&root.join("missing"), "SER1"), None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn identity_fallbacks() {
        let root = sysfs_with("fallback", &[("lp2", Some("S9"), L800_ID)]);
        // A serial on a non-USB URI does not pick the USB device.
        let i = Identity::from_parts("N", "ipp://10.0.0.5/?serial=S9".into(), String::new(), &root, None);
        assert_eq!((i.source, i.serial.clone()), (Source::None, None));
        // usb:// with an empty serial: the URI's MDL, never a sysfs device.
        let i = Identity::from_parts("E", "usb://EPSON/L800?serial=".into(), String::new(), &root, None);
        assert_eq!((i.source, i.id.mdl()), (Source::Uri, Some("L800")));
        assert_eq!(i.status_node_in(&root), None);
        // usb:// without MDL falls through to the PPD.
        let i = Identity::from_parts("P", "usb://EPSON?serial=GONE".into(), String::new(), &root, Some("MFG:EPSON;MDL:L800;"));
        assert_eq!((i.source, i.id.mdl()), (Source::Ppd, Some("L800")));
        // Nothing at all.
        let i = Identity::from_parts("X", String::new(), String::new(), &root, None);
        assert_eq!((i.source, i.id.clone()), (Source::None, DeviceId::default()));
        // Status node needs D4 in the live ID.
        fs::write(root.join("lp2-usbdev/iface/ieee1284_id"), "MFG:EPSON;CMD:ESCPR1;MDL:X;").unwrap();
        let i = Identity::from_parts("U", "usb://EPSON/X?serial=S9".into(), String::new(), &root, None);
        assert_eq!(i.source, Source::Usb);
        assert_eq!(i.status_node_in(&root), None);
        fs::remove_dir_all(root).unwrap();
    }
}

//! `--report`: what a contributor sends to add a printer model (README,
//! Profiles): the identity, the PPD NickName, the queue's options, and one
//! status reply as hex, with the serial number masked. The status request
//! is the only thing sent to the printer, under the usual rules (not while
//! CUPS has jobs, Exit always sent).

use crate::printer::{self, Identity};
use crate::profile::Profile;
use crate::status::{self, QueryError, Status, st2};

/// Collect and format the report for `queue`.
pub fn run(queue: &str, profiles: &[Profile]) -> String {
    let p = Identity::of(queue);
    let nickname = printer::cups_ppd(queue).and_then(|ppd| printer::ppd_value(&ppd, "NickName"));
    let options = match std::process::Command::new("lpoptions").env("LC_ALL", "C").args(["-p", queue, "-l"]).output() {
        Ok(o) => format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)),
        Err(e) => format!("cannot run lpoptions: {e}\n"),
    };
    let reply = status::query_raw(queue, p.status_node().as_deref());
    format(&p, printer_profile(profiles, &p), nickname.as_deref(), &options, reply)
}

/// The matching profile, if any.
fn printer_profile<'a>(profiles: &'a [Profile], p: &Identity) -> Option<&'a Profile> {
    crate::profile::find(profiles, p.id.mdl())
}

fn format(p: &Identity, profile: Option<&Profile>, nickname: Option<&str>, options: &str, reply: Result<Vec<u8>, QueryError>) -> String {
    let mut s = format!("epson-l800-utility {} report\n\nQueue: {}\n", env!("CARGO_PKG_VERSION"), p.queue);
    s.push_str(&p.summary());
    s.push_str(&profile.cloned().unwrap_or_else(Profile::generic).summary(profile.is_some()));
    s.push_str(&format!("PPD NickName: {}\n\n$ lpoptions -p {} -l\n{options}\n", nickname.unwrap_or("(none)"), p.queue));
    match reply {
        Ok(mut raw) => {
            st2::mask_serial(&mut raw);
            s.push_str("Status reply (@BDC ST2, serial number masked):\n");
            for line in raw.chunks(16) {
                s.push_str(&line.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "));
                s.push('\n');
            }
            match Status::from_reply(&raw) {
                Some(st) => {
                    s.push_str(&format!(
                        "ST={:02X?} ER={:02X?} TP={} WR={:02X?} TC={:?}\n",
                        st.st, st.er, st.tp, st.warnings, st.cleaning_secs
                    ));
                    for i in &st.inks {
                        let level = i.percent().map_or(format!("code {:02X}", i.raw_level), |l| format!("{l}%"));
                        s.push_str(&format!("  ink {:02X} colour {:02X} {}: {level}\n", i.ink_id, i.color, status::ink_name(i)));
                    }
                }
                None => s.push_str("(not a complete @BDC ST2 reply)\n"),
            }
        }
        Err(QueryError::Busy) => s.push_str("Status: not read, the CUPS queue has jobs.\n"),
        Err(QueryError::Failed(e)) => s.push_str(&format!("Status: no reply ({e}).\n")),
    }
    // The USB serial number is in the device URI (and may be in the ID).
    match p.serial.as_deref().filter(|v| !v.is_empty()) {
        Some(serial) => s.replace(serial, &"X".repeat(serial.len())),
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_masks_the_serial() {
        let p = Identity {
            queue: "L800".into(),
            uri: "usb://EPSON/L800?serial=1234567X8901234567".into(),
            make_model: "Epson L800 Series".into(),
            id: printer::DeviceId::parse("MFG:EPSON;CMD:ESCPL2,BDC,D4,D4PX;MDL:L800;CLS:PRINTER;DES:EPSON L800;"),
            source: printer::Source::Usb,
            serial: Some("1234567X8901234567".into()),
        };
        let mut raw = st2::tests::live();
        let at = raw.windows(2).position(|w| w == [st2::id::SN, 0x0A]).unwrap() + 2;
        raw[at..at + 10].copy_from_slice(b"S3RIAL0123");
        let profiles = crate::profile::builtin();
        let r = format(&p, printer_profile(&profiles, &p), Some("Epson L800 Series"), "PageSize/Size: *A4 Letter\n", Ok(raw));
        assert!(!r.contains("1234567X8901234567"), "{r}");
        assert!(!r.contains("S3RIAL"), "{r}");
        assert!(r.contains("usb://EPSON/L800?serial=XXXXXXXXXXXXXXXXXX"), "{r}");
        assert!(r.contains("Profile: L800"), "{r}");
        assert!(r.contains("ST=Some(04) ER=None TP=false WR=[74]"), "{r}");
        assert!(r.contains("ink 06 colour 04 Light Cyan: 3%"), "{r}");
        assert!(r.contains("PageSize/Size: *A4 Letter"), "{r}");
        let none = format(&p, None, None, "", Err(QueryError::Busy));
        assert!(none.contains("Profile: generic") && none.contains("the CUPS queue has jobs"), "{none}");
    }
}

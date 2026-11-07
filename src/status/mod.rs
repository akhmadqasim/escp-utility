//! Printer status, read the way the Windows L800 driver reads it: the
//! `st 01 00 01` request on the IEEE 1284.4 EPSON-CTRL channel, answered
//! with a binary `@BDC ST2` reply.
//!
//! Transport: 1284.4 directly over the kernel usblp node `/dev/usb/lpN`.
//! The driver stays bound and a query takes about 60 ms. Each query opens
//! the node, runs Enter/Init/OpenChannel, sends the status request, then
//! CloseChannel and Exit. Exit is sent even on error (also from `Drop`).
//! The only data ever written is the status request.
//!
//! We query only when the CUPS queue has no jobs, so we never interleave
//! with a print job. Epson's ecbd daemon is not used: it answers with a
//! shortened text that lacks the warning list and the cleaning time.
//!
//! Evidence: phase2/status-spike/NOTES.md (live usbmon captures) and
//! phase2/rea/NOTES.md §1.2, §1.4, §2 (Windows E_IHUTG5P / E_IAPRG5P).

pub mod d4;
pub mod st2;
mod usblp;

use std::path::Path;
use std::process::Command;
use std::time::Duration;

pub use st2::Ink;

/// What the app uses from one ST2 reply.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    /// Field 0x01: 0x00 error, 0x01 self-printing, 0x02/0x03 printing,
    /// 0x04 ready, 0x07 cleaning, 0x08 initial setup, 0x0A shutting down,
    /// 0x11 ink charging.
    pub st: Option<u8>,
    /// Field 0x02, present only with an error.
    pub er: Option<u8>,
    /// Field 0x03 is present.
    pub tp: bool,
    /// Field 0x04.
    pub warnings: Vec<u8>,
    /// Field 0x0C: expected cleaning time in seconds.
    pub cleaning_secs: Option<u16>,
    /// Field 0x0F, in printer order (L800: C, M, LM, LC, BK, Y).
    pub inks: Vec<Ink>,
}

impl Status {
    pub fn from_reply(raw: &[u8]) -> Option<Status> {
        // A reply cut short (timeout, lost packet) may lack ER or WR: reject it.
        let r = st2::parse(raw).filter(st2::Reply::complete)?;
        let byte = |id| r.get(id).and_then(|d| d.first().copied());
        Some(Status {
            st: byte(st2::id::ST),
            er: byte(st2::id::ER),
            tp: r.get(st2::id::TP).is_some(),
            warnings: r.get(st2::id::WR).map(<[u8]>::to_vec).unwrap_or_default(),
            cleaning_secs: r
                .get(st2::id::TC)
                .filter(|d| d.len() >= 2)
                .map(|d| u16::from_le_bytes([d[0], d[1]])),
            inks: r.get(st2::id::IC).map(st2::inks).unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError {
    /// The CUPS queue has jobs, so we did not touch the printer.
    Busy,
    /// No reply (no device, no access, timeout, bad reply).
    Failed(String),
}

/// Whether `lpstat -o <queue>` lists jobs. Err when lpstat cannot be run or
/// fails (cupsd down, unknown queue): the CUPS state is then unknown.
pub fn jobs_queued(queue: &str) -> Result<bool, String> {
    let o = Command::new("lpstat").args(["-o", queue]).output().map_err(|e| format!("Cannot run lpstat: {e}"))?;
    if !o.status.success() {
        return Err(format!("lpstat -o {queue}: {}", String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(!o.stdout.iter().all(u8::is_ascii_whitespace))
}

static DEVICE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Exclusive use of the printer: held by every status query (usblp allows
/// only one opener), while a job is handed to CUPS, and at exit so no
/// session is cut off before Close/Exit.
pub fn exclusive() -> std::sync::MutexGuard<'static, ()> {
    DEVICE.lock().unwrap_or_else(|e| e.into_inner())
}

/// One status query on the usblp `node` of the queue's printer (printer.rs
/// `Identity::status_node`; None: not connected, or no D4 channel). Never
/// touches the printer while CUPS has jobs. Queries are serialised through
/// [`exclusive`].
pub fn query(queue: &str, node: Option<&Path>) -> Result<Status, QueryError> {
    let raw = query_raw(queue, node)?;
    Status::from_reply(&raw).ok_or_else(|| QueryError::Failed("reply is not a complete @BDC ST2".into()))
}

/// The reply bytes of one status query (see [`query`]).
pub fn query_raw(queue: &str, node: Option<&Path>) -> Result<Vec<u8>, QueryError> {
    let _guard = exclusive();
    let fail = QueryError::Failed;
    if jobs_queued(queue).map_err(fail)? {
        return Err(QueryError::Busy);
    }
    let path = node.ok_or_else(|| fail("no /dev/usb/lp* with a status channel for this printer".into()))?;
    let port = usblp::UsbLp::open(path).map_err(|e| fail(format!("{}: {e}", path.display())))?;
    d4::Session::new(port).query().map_err(|e| fail(e.to_string()))
}

/// Result of the Windows pre-check (E_IHUTG5P `FUN_00419768` + `FUN_0041a2f8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precheck {
    Proceed,
    /// Dialog 270: "A problem has occurred." [OK] [Details].
    Problem,
    /// Dialog 271: "Cannot complete Nozzle Check." / "... Head Cleaning."
    CannotComplete,
    /// Dialog 271: "Head Cleaning can not be completed." (ink-tank warning)
    CleaningBlocked,
    /// Dialog 271: "Now cleaning!"
    NowCleaning,
    /// Dialog 271: an ink is used up (generic pre-check only).
    InkOut,
}

pub fn precheck(s: &Status, cleaning: bool) -> Precheck {
    match s.st {
        Some(0x00) | Some(0x11) => return Precheck::Problem,
        Some(0x01) => return Precheck::CannotComplete,
        _ => {}
    }
    if s.tp {
        return Precheck::CannotComplete;
    }
    // E_IHBRG5P string 5052 lists WR codes 0x51-0x56. Each clears bit
    // (code - 0x51) of the cleaning-type mask. Both radios need mask 3, so
    // 0x51 or 0x52 leaves no cleaning type (M: FUN_0041965c).
    if cleaning && s.warnings.iter().any(|&w| w == 0x51 || w == 0x52) {
        return Precheck::CleaningBlocked;
    }
    if cleaning && s.st == Some(0x07) {
        return Precheck::NowCleaning;
    }
    Precheck::Proceed
}

/// The pre-check for printers without the Windows L800 tables, as Epson's
/// Linux utility gates its buttons (`updateCurrenInkJetPrinterStaus`,
/// rea/MULTI-MODEL.md §1.2) plus escpr's cleaning warning: an error stops
/// both (ST 00, an ER field, or an ST that escpr `serAnalyzeStatus` does
/// not know and so takes as an error, such as 11; `message_generic` shows
/// those as "Printer error"); ST 01 stops both; an ink at 0 stops both; an
/// ink at 1-4 %, or a warning 0x51-0x5A (`EPS_PRNWARN_DISABLE_CLEAN`),
/// stops cleaning; ST 07 is "Now cleaning!".
pub fn precheck_generic(s: &Status, cleaning: bool) -> Precheck {
    match s.st {
        Some(0x01) => return Precheck::CannotComplete,
        Some(0x02..=0x04 | 0x07 | 0x08 | 0x0A | 0x0D) | None if s.er.is_none() => {}
        _ => return Precheck::Problem,
    }
    if s.inks.iter().any(|i| i.percent() == Some(0)) {
        return Precheck::InkOut;
    }
    let low = s.inks.iter().any(|i| matches!(i.percent(), Some(1..=4)));
    if cleaning && (low || s.warnings.iter().any(|w| (0x51..=0x5A).contains(w))) {
        return Precheck::CleaningBlocked;
    }
    if cleaning && s.st == Some(0x07) {
        return Precheck::NowCleaning;
    }
    Precheck::Proceed
}

/// Ink names by colour code (field 0x0F, escpr `parseCartridge`); codes
/// 0-5 are E_IASRG5P strings 4150-4155. Cartridge type 0x0B is matte black.
pub fn ink_name(ink: &Ink) -> &'static str {
    if ink.ink_id == 0x0B {
        return "Matte Black";
    }
    match ink.color {
        0x00 => "Black",
        0x01 => "Cyan",
        0x02 => "Magenta",
        0x03 => "Yellow",
        0x04 => "Light Cyan",
        0x05 => "Light Magenta",
        0x06 => "Light Yellow",
        0x07 => "Dark Yellow",
        0x08 => "Light Black",
        0x09 => "Red",
        0x0A => "Violet",
        0x0B => "Clear",
        0x0C => "Light Light Black",
        0x0D => "Orange",
        0x0E => "Green",
        0x0F => "White",
        0x40 => "Composite",
        _ => "Ink",
    }
}

/// Status Monitor 3 message for printers without the Windows L800 tables:
/// neutral texts for the generic ST2 meanings (escpr `serAnalyzeStatus`).
/// Error codes mean different things on different models (A3 and egID
/// overrides there), so they are shown as numbers.
pub fn message_generic(s: &Status) -> (String, String) {
    let t = |a: &str, b: &str| (a.to_string(), b.to_string());
    match s.st {
        Some(0x00) => {
            let title = s.er.map_or("Printer error".to_string(), |er| format!("Printer error (code {er:02X})"));
            return (title, GENERIC_ERROR.to_string());
        }
        Some(0x08) => return t("Initial setup", "The printer's initial setup has not finished. Complete it on the printer."),
        Some(0x0A) => return t("Printer is shutting down", "Please wait."),
        _ => {}
    }
    if s.warnings.iter().any(|w| matches!(w, 0x10..=0x1A | 0x71..=0x7A)) {
        return t("Ink low", &format!("{}You can keep printing, but get new ink ready soon. {GENERIC_INK_NOTE}", low_inks(s)));
    }
    match s.st {
        Some(0x01..=0x03) | Some(0x0D) => t("Printing", "Please wait."),
        Some(0x04) => t("Ready", "Ready to print."),
        Some(0x07) => t("Cleaning", "The printer is cleaning the print head. Please wait."),
        Some(st) => (format!("Printer error (status {st:02X})"), GENERIC_ERROR.to_string()),
        None => t("Communication error", COMM),
    }
}

const GENERIC_ERROR: &str = "Look at the printer's display or lights. If the error stays, switch the printer off and on again.";
/// Without the "ink tanks" sentence of [`INK_NOTE`].
pub const GENERIC_INK_NOTE: &str = "The ink levels shown are estimates and can differ from the real levels.";

/// Status Monitor 3 message: (title, text). Matched in the order of the
/// Windows table `GetStatusCode` (E_IABRG5P DAT_1800027e0, 33 rows of
/// (ST, ER, WR)): rows 0-11 need ST=00 (ER absent, 00, 01, 04, 05, 06, 10,
/// 12, 29, 37, 38, any), then ST=08, ST=0A, then WR 10-15, 20, 71-76 with
/// any ST, then ST=02/03, 04, 07, 11, and "Communication error" for the
/// rest. Titles are the Windows strings; the texts are our own wording.
pub fn message(s: &Status) -> (&'static str, String) {
    let t = |a: &'static str, b: &str| (a, b.to_string());
    match s.st {
        Some(0x00) => {
            return match s.er {
                None => t("Communication error", COMM),
                Some(0x00) => t("General error", RESTART),
                Some(0x01) => t("Printer busy", "Please wait."),
                Some(0x04) => t("Paper jam or CD/DVD tray jam", "For paper, switch the printer off and pull the jammed sheet out by hand. For a CD/DVD, take the tray out. Then press the Paper button on the printer."),
                Some(0x05) => t("Check Ink Levels", &format!("{}Check how much ink is left in the printer's ink tanks. {INK_NOTE}", low_inks(s))),
                Some(0x06) => t("Paper out or not loaded correctly", "Load the paper again and set the front tray to the lower (paper) position. Then press the Paper button on the printer."),
                Some(0x10) => t("Service required", "The printer's ink pads have reached the end of their service life. Contact Epson support."),
                Some(0x12) => t("Page not printed or multi-page error", "A page was not printed, several sheets were fed at once, or the wrong paper size was fed. Take the paper out, load it again, and press the Paper button if needed."),
                Some(0x29) => t("CD/DVD tray not set correctly", "Load the tray again, then press the Paper button on the printer."),
                Some(0x37) => t("Printer cover open", "Close the printer cover."),
                Some(0x38) => t("Front tray position error", "For paper, set the front tray to the lower (paper) position. For a CD/DVD, set it to the upper (CD/DVD) position."),
                Some(_) => t("Unknown error", RESTART),
            };
        }
        Some(0x08) => return t("Initial setup error", "The printer's initial setup has not finished. Complete it on the printer."),
        Some(0x0A) => return t("Printer is shutting down", "Please wait."),
        _ => {}
    }
    match top_warning(&s.warnings) {
        Some(0x10..=0x15) => return t("Ink low", &format!("{}You can keep printing, but get a new ink bottle ready soon. {INK_NOTE}", low_inks(s))),
        Some(0x20) => return t("Service required", "The printer's ink pads are nearing the end of their service life. Contact Epson support."),
        Some(0x71..=0x76) => return t("Ink low", &format!("{}You can keep printing. Refill the ink tank soon. {INK_NOTE}", low_inks(s))),
        _ => {}
    }
    match s.st {
        Some(0x02) | Some(0x03) => t("Printing", "Please wait."),
        Some(0x04) => t("Ready", "Ready to print."),
        Some(0x07) => t("Printer busy", "The printer is cleaning the print head or charging ink. Please wait."),
        Some(0x11) => t("Ink charging in progress", "Please wait."),
        _ => t("Communication error", COMM),
    }
}

pub const COMM: &str = "Check that all cables are connected and that the printer is switched on. If the power went off while printing, cancel the print job.";
const RESTART: &str = "Open the printer cover and remove any paper inside. Switch the printer off, delete all print jobs, then switch it on again.";
/// Meaning of E_IASRG5P 30699.
pub const INK_NOTE: &str = "The ink levels shown are estimates and can differ from the real levels. To be sure, look at the ink tanks on the printer.";

/// The one warning Windows matches against the table (E_IAPRG5P
/// `FUN_18007b850`): the first whose high nibble is 7, else 2, 4, 3, 1, else
/// the first warning. The table has rows only for 0x10-0x15, 0x20 and
/// 0x71-0x76, so a higher-ranked 0x4x/0x3x warning hides "Ink low" there
/// too and the window falls through to the ST rows (e.g. "Ready").
fn top_warning(w: &[u8]) -> Option<u8> {
    [0x70, 0x20, 0x40, 0x30, 0x10]
        .iter()
        .find_map(|hi| w.iter().copied().find(|&c| c & 0xF0 == *hi))
        .or_else(|| w.first().copied())
}

/// "Light Cyan: " style prefix for warnings 0x1n / 0x7n. The slot n is read
/// as the index into field 0x0F (M: it fits the live levels).
fn low_inks(s: &Status) -> String {
    let names: Vec<&str> = s
        .warnings
        .iter()
        .filter(|&&w| matches!(w, 0x10..=0x1A | 0x71..=0x7A))
        .filter_map(|&w| s.inks.get(((w & 0x0F) as usize).checked_sub(usize::from(w >= 0x71))?))
        .map(ink_name)
        .collect();
    if names.is_empty() { String::new() } else { format!("{}: ", names.join(", ")) }
}

/// Head-cleaning progress, as the Windows worker `FUN_00411e5c` tracks it:
/// poll every 2 s; cleaning has started when ST=0x07 and field 0x0C > 0;
/// it is done after 3 polls in a row with ST != 0x07; give up after 16 polls
/// without a start (about 32 s) or 3 failed reads in a row.
#[derive(Debug, Default)]
pub struct CleaningMonitor {
    started_secs: Option<u16>,
    polls_without_start: u32,
    not_cleaning: u32,
    failures: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Waiting,
    /// Cleaning started; it takes about this many seconds.
    Started(u16),
    Running,
    Done,
    GaveUp,
}

pub const POLL: Duration = Duration::from_secs(2);

impl CleaningMonitor {
    pub fn on_poll(&mut self, res: &Result<Status, QueryError>) -> Step {
        let s = match res {
            Ok(s) => {
                self.failures = 0;
                s
            }
            Err(QueryError::Busy) => {
                // Our cleaning job is still in the CUPS queue: not a failure.
                return self.no_start();
            }
            Err(QueryError::Failed(_)) => {
                self.failures += 1;
                if self.failures >= 3 {
                    return Step::GaveUp;
                }
                return self.no_start();
            }
        };
        let cleaning = s.st == Some(0x07);
        match self.started_secs {
            None => match s.cleaning_secs.filter(|&t| t > 0) {
                Some(t) if cleaning => {
                    self.started_secs = Some(t);
                    Step::Started(t)
                }
                _ => self.no_start(),
            },
            Some(_) => {
                self.not_cleaning = if cleaning { 0 } else { self.not_cleaning + 1 };
                if self.not_cleaning >= 3 { Step::Done } else { Step::Running }
            }
        }
    }

    fn no_start(&mut self) -> Step {
        if self.started_secs.is_some() {
            return Step::Running;
        }
        self.polls_without_start += 1;
        if self.polls_without_start >= 16 { Step::GaveUp } else { Step::Waiting }
    }
}

/// Progress bar value: elapsed / expected, held at 97% until done.
pub fn progress(elapsed: Duration, secs: u16) -> f32 {
    (elapsed.as_secs_f32() / f32::from(secs.max(1))).min(0.97)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(st: u8) -> Status {
        Status { st: Some(st), ..Default::default() }
    }

    #[test]
    fn live_reply_decodes() {
        let s = Status::from_reply(&st2::tests::live()).unwrap();
        assert_eq!(s.st, Some(0x04));
        assert_eq!(s.warnings, vec![0x74]);
        assert_eq!(s.inks.len(), 6);
        let names: Vec<&str> = s.inks.iter().map(ink_name).collect();
        assert_eq!(names, ["Cyan", "Magenta", "Light Magenta", "Light Cyan", "Black", "Yellow"]);
        let (title, text) = message(&s);
        assert_eq!(title, "Ink low");
        assert!(text.starts_with("Light Cyan: "), "{text}");
        assert_eq!(precheck(&s, true), Precheck::Proceed);
    }

    #[test]
    fn incomplete_reply_is_rejected() {
        let mut raw = st2::tests::live();
        raw.truncate(raw.len() - 1);
        assert_eq!(Status::from_reply(&raw), None);
    }

    #[test]
    fn prechecks_follow_windows_table() {
        assert_eq!(precheck(&st(0x00), false), Precheck::Problem);
        assert_eq!(precheck(&st(0x11), true), Precheck::Problem);
        assert_eq!(precheck(&st(0x01), false), Precheck::CannotComplete);
        assert_eq!(precheck(&Status { tp: true, ..st(4) }, false), Precheck::CannotComplete);
        assert_eq!(precheck(&st(0x07), true), Precheck::NowCleaning);
        assert_eq!(precheck(&st(0x07), false), Precheck::Proceed);
        assert_eq!(precheck(&Status { warnings: vec![0x52], ..st(4) }, true), Precheck::CleaningBlocked);
        assert_eq!(precheck(&Status { warnings: vec![0x52], ..st(4) }, false), Precheck::Proceed);
    }

    #[test]
    fn messages() {
        assert_eq!(message(&st(4)).0, "Ready");
        assert_eq!(message(&Status { er: Some(0x37), ..st(0) }).0, "Printer cover open");
        assert_eq!(message(&st(0)).0, "Communication error");
        assert_eq!(message(&Status { warnings: vec![0x20, 0x71], ..st(4) }).0, "Ink low");
        assert_eq!(message(&Status { warnings: vec![0x20], ..st(4) }).0, "Service required");
        // ER rows need ST=00 (Windows table rows 1-11).
        assert_eq!(message(&Status { er: Some(0x37), ..st(4) }).0, "Ready");
        // A 0x4x warning outranks 0x1x and has no row: Windows shows the ST row.
        assert_eq!(message(&Status { warnings: vec![0x12, 0x41], ..st(4) }).0, "Ready");
        assert_eq!(message(&Status { warnings: vec![0x51, 0x74], ..st(4) }).0, "Ink low");
    }

    /// A synthetic ST2 reply: ST, optional ER, warnings, and inks given as
    /// (cartridge type, colour, level) in 3-byte entries.
    fn synthetic(st_byte: u8, er: Option<u8>, wr: &[u8], inks: &[(u8, u8, u8)]) -> Status {
        let mut extra = vec![];
        if let Some(er) = er {
            extra.extend([0x02, 0x01, er]);
        }
        extra.extend([0x04, wr.len() as u8]);
        extra.extend(wr);
        extra.extend([0x0F, 1 + 3 * inks.len() as u8, 3]);
        for &(t, c, l) in inks {
            extra.extend([t, c, l]);
        }
        Status::from_reply(&st2::tests::sample(st_byte, &extra)).unwrap()
    }

    #[test]
    fn generic_nine_inks() {
        let inks = [(0x0B, 0x00, 50), (0x01, 0x00, 60), (0, 1, 70), (0, 2, 3), (0, 3, 80), (0, 4, 90), (0, 5, 55), (0, 8, 40), (0, 0x0C, 30)];
        let s = synthetic(0x04, None, &[], &inks);
        let names: Vec<&str> = s.inks.iter().map(ink_name).collect();
        assert_eq!(
            names,
            ["Matte Black", "Black", "Cyan", "Magenta", "Yellow", "Light Cyan", "Light Magenta", "Light Black", "Light Light Black"]
        );
        assert_eq!(message_generic(&s).0, "Ready");
        // Magenta at 3 %: cleaning off, nozzle check on.
        assert_eq!(precheck_generic(&s, true), Precheck::CleaningBlocked);
        assert_eq!(precheck_generic(&s, false), Precheck::Proceed);
        let s = synthetic(0x04, None, &[0x12], &inks);
        assert_eq!(message_generic(&s), ("Ink low".into(), format!("Cyan: You can keep printing, but get new ink ready soon. {GENERIC_INK_NOTE}")));
    }

    #[test]
    fn generic_prechecks() {
        let ok = [(0, 0, 50), (0, 1, 60)];
        assert_eq!(precheck_generic(&synthetic(0x00, Some(0x04), &[], &ok), false), Precheck::Problem);
        assert_eq!(precheck_generic(&synthetic(0x01, None, &[], &ok), false), Precheck::CannotComplete);
        // An ST escpr does not know is an error there (and "Printer error" here).
        assert_eq!(precheck_generic(&synthetic(0x11, None, &[], &ok), false), Precheck::Problem);
        assert_eq!(precheck_generic(&synthetic(0x0E, None, &[], &ok), true), Precheck::Problem);
        assert_eq!(precheck_generic(&synthetic(0x04, Some(0x06), &[], &ok), false), Precheck::Problem);
        let out = synthetic(0x04, None, &[], &[(0, 0, 0), (0, 1, 60)]);
        assert_eq!(precheck_generic(&out, false), Precheck::InkOut);
        assert_eq!(precheck_generic(&out, true), Precheck::InkOut);
        let wr = synthetic(0x04, None, &[0x55], &ok);
        assert_eq!(precheck_generic(&wr, true), Precheck::CleaningBlocked);
        assert_eq!(precheck_generic(&wr, false), Precheck::Proceed);
        assert_eq!(precheck_generic(&synthetic(0x07, None, &[], &ok), true), Precheck::NowCleaning);
        assert_eq!(precheck_generic(&synthetic(0x07, None, &[], &ok), false), Precheck::Proceed);
        assert_eq!(precheck_generic(&synthetic(0x04, None, &[], &ok), true), Precheck::Proceed);
    }

    #[test]
    fn generic_messages() {
        let ok = [(0, 0, 50)];
        assert_eq!(message_generic(&synthetic(0x00, Some(0x6A), &[], &ok)).0, "Printer error (code 6A)");
        assert_eq!(message_generic(&synthetic(0x00, None, &[], &ok)).0, "Printer error");
        assert_eq!(message_generic(&synthetic(0x07, None, &[], &ok)).0, "Cleaning");
        assert_eq!(message_generic(&synthetic(0x02, None, &[], &ok)).0, "Printing");
        assert_eq!(message_generic(&synthetic(0x0A, None, &[], &ok)).0, "Printer is shutting down");
        assert_eq!(message_generic(&synthetic(0x0E, None, &[], &ok)).0, "Printer error (status 0E)");
        // The L800 live reply: the same "Ink low" for Light Cyan, neutral wording.
        let s = Status::from_reply(&st2::tests::live()).unwrap();
        let (title, text) = message_generic(&s);
        assert_eq!(title, "Ink low");
        assert!(text.starts_with("Light Cyan: "), "{text}");
    }

    fn ink(level: u8) -> Ink {
        Ink { ink_id: 0, color: 0, raw_level: level }
    }

    fn with(st_byte: Option<u8>, er: Option<u8>, wr: &[u8], levels: &[u8]) -> Status {
        Status { st: st_byte, er, warnings: wr.to_vec(), inks: levels.iter().map(|&l| ink(l)).collect(), ..Default::default() }
    }

    #[test]
    fn generic_precheck_table() {
        use Precheck::*;
        // (ST, ER, WR, ink levels, cleaning, expected)
        type Case = (Option<u8>, Option<u8>, &'static [u8], &'static [u8], bool, Precheck);
        let cases: &[Case] = &[
            // ST 00 and ST 01 stop both buttons.
            (Some(0x00), None, &[], &[50], false, Problem),
            (Some(0x00), None, &[], &[50], true, Problem),
            (Some(0x01), None, &[], &[50], true, CannotComplete),
            // ST 01 wins over an ER field.
            (Some(0x01), Some(0x04), &[], &[50], false, CannotComplete),
            // Unknown ST or any ER blocks.
            (Some(0x05), None, &[], &[50], false, Problem),
            (Some(0xFF), None, &[], &[50], true, Problem),
            (Some(0x07), Some(0x00), &[], &[50], true, Problem),
            (None, Some(0x10), &[], &[50], false, Problem),
            // Known non-error STs proceed.
            (Some(0x02), None, &[], &[50], false, Proceed),
            (Some(0x03), None, &[], &[50], true, Proceed),
            (Some(0x08), None, &[], &[50], true, Proceed),
            (Some(0x0A), None, &[], &[50], false, Proceed),
            (Some(0x0D), None, &[], &[50], true, Proceed),
            // Ink 0 stops both, ahead of the cleaning rules and ST 07.
            (Some(0x04), None, &[], &[50, 0], false, InkOut),
            (Some(0x07), None, &[0x51], &[0, 3], true, InkOut),
            // Ink 1-4 % stops cleaning only; 5 % and special codes do not.
            (Some(0x04), None, &[], &[1], true, CleaningBlocked),
            (Some(0x04), None, &[], &[4], true, CleaningBlocked),
            (Some(0x04), None, &[], &[4], false, Proceed),
            (Some(0x04), None, &[], &[5], true, Proceed),
            (Some(0x04), None, &[], &[0x6E, 0xFF], true, Proceed),
            // WR 0x51-0x5A stops cleaning; the neighbours do not.
            (Some(0x04), None, &[0x51], &[50], true, CleaningBlocked),
            (Some(0x04), None, &[0x5A], &[50], true, CleaningBlocked),
            (Some(0x04), None, &[0x50], &[50], true, Proceed),
            (Some(0x04), None, &[0x5B], &[50], true, Proceed),
            (Some(0x04), None, &[0x5A], &[50], false, Proceed),
            // Cleaning blocked outranks "Now cleaning!".
            (Some(0x07), None, &[0x55], &[50], true, CleaningBlocked),
            (Some(0x07), None, &[], &[2], true, CleaningBlocked),
            (Some(0x07), None, &[], &[50], true, NowCleaning),
            (Some(0x07), None, &[], &[50], false, Proceed),
        ];
        for (i, &(st_b, er, wr, lv, cl, want)) in cases.iter().enumerate() {
            assert_eq!(precheck_generic(&with(st_b, er, wr, lv), cl), want, "case {i}");
        }
    }

    #[test]
    fn l800_precheck_table() {
        use Precheck::*;
        // (ST, TP, WR, ink levels, cleaning, expected)
        type Case = (Option<u8>, bool, &'static [u8], &'static [u8], bool, Precheck);
        let cases: &[Case] = &[
            // ST 00 / 11 are a problem even with TP or cleaning warnings.
            (Some(0x00), true, &[0x51], &[50], true, Problem),
            (Some(0x11), false, &[], &[50], false, Problem),
            (Some(0x01), true, &[], &[50], true, CannotComplete),
            // TP outranks the cleaning warning.
            (Some(0x04), true, &[0x51], &[50], true, CannotComplete),
            (Some(0x07), true, &[], &[50], true, CannotComplete),
            // Only 0x51/0x52 block cleaning on the L800 table.
            (Some(0x04), false, &[0x51], &[50], true, CleaningBlocked),
            (Some(0x04), false, &[0x53], &[50], true, Proceed),
            (Some(0x04), false, &[0x56], &[50], true, Proceed),
            (Some(0x07), false, &[0x52], &[50], true, CleaningBlocked),
            // Ink levels are not checked there.
            (Some(0x04), false, &[], &[0, 2], true, Proceed),
        ];
        for (i, &(st_b, tp, wr, lv, cl, want)) in cases.iter().enumerate() {
            let s = Status { tp, ..with(st_b, None, wr, lv) };
            assert_eq!(precheck(&s, cl), want, "case {i}");
        }
    }

    #[test]
    fn l800_error_rows_need_st_00() {
        let rows = [
            (Some(0x00), "General error"),
            (Some(0x01), "Printer busy"),
            (Some(0x04), "Paper jam or CD/DVD tray jam"),
            (Some(0x05), "Check Ink Levels"),
            (Some(0x06), "Paper out or not loaded correctly"),
            (Some(0x10), "Service required"),
            (Some(0x12), "Page not printed or multi-page error"),
            (Some(0x29), "CD/DVD tray not set correctly"),
            (Some(0x37), "Printer cover open"),
            (Some(0x38), "Front tray position error"),
            (Some(0x99), "Unknown error"),
            (None, "Communication error"),
        ];
        for (er, title) in rows {
            // Error rows win over any warning when ST=00.
            assert_eq!(message(&with(Some(0x00), er, &[0x71], &[50])).0, title, "ER {er:?}");
            // With any other ST the ER field is ignored.
            assert_eq!(message(&with(Some(0x04), er, &[], &[50])).0, "Ready", "ER {er:?}");
        }
    }

    #[test]
    fn l800_message_priority() {
        let cases: &[(u8, &[u8], &str)] = &[
            // ST 08 / 0A come before the warning rows.
            (0x08, &[0x71], "Initial setup error"),
            (0x0A, &[0x10], "Printer is shutting down"),
            // Warning rank: 7x, then 2x, 4x, 3x, 1x.
            (0x04, &[0x10, 0x20], "Service required"),
            (0x04, &[0x20, 0x75], "Ink low"),
            (0x04, &[0x15], "Ink low"),
            (0x04, &[0x16], "Ready"),
            // 0x4x / 0x3x outrank 0x1x and have no row: "Ink low" is hidden.
            (0x04, &[0x13, 0x40], "Ready"),
            (0x04, &[0x31, 0x11], "Ready"),
            (0x07, &[0x41, 0x10], "Printer busy"),
            // ...but not 0x7x.
            (0x04, &[0x41, 0x72], "Ink low"),
            // Fallback to the first warning when no ranked nibble matches.
            (0x02, &[0x51], "Printing"),
            // ST rows.
            (0x03, &[], "Printing"),
            (0x07, &[], "Printer busy"),
            (0x11, &[], "Ink charging in progress"),
            (0x01, &[], "Communication error"),
            (0x0D, &[], "Communication error"),
        ];
        for &(st_b, wr, title) in cases {
            assert_eq!(message(&with(Some(st_b), None, wr, &[50; 6])).0, title, "ST {st_b:02X} WR {wr:02X?}");
        }
        assert_eq!(message(&Status::default()).0, "Communication error");
    }

    #[test]
    fn low_ink_names_follow_slot() {
        let inks: Vec<Ink> = (0..6).map(|c| Ink { ink_id: 0, color: c, raw_level: 50 }).collect();
        // 0x1n is slot n, 0x7n is slot n-1.
        let s = Status { warnings: vec![0x12, 0x71], inks: inks.clone(), ..st(4) };
        assert_eq!(low_inks(&s), "Magenta, Black: ");
        // A slot past field 0x0F, or 0x70, gives no name.
        let s = Status { warnings: vec![0x19, 0x70], inks, ..st(4) };
        assert_eq!(low_inks(&s), "");
        let (title, text) = message(&with(Some(4), None, &[0x14], &[]));
        assert_eq!(title, "Ink low");
        assert!(text.starts_with("You can keep printing"), "{text}");
    }

    #[test]
    fn generic_message_priority() {
        // Errors and ST 08/0A come before "Ink low"; other STs after it.
        assert_eq!(message_generic(&with(Some(0x00), Some(0x04), &[0x71], &[50])).0, "Printer error (code 04)");
        assert_eq!(message_generic(&with(Some(0x08), None, &[0x71], &[50])).0, "Initial setup");
        assert_eq!(message_generic(&with(Some(0x07), None, &[0x1A], &[50])).0, "Ink low");
        assert_eq!(message_generic(&with(Some(0x04), None, &[0x7A], &[50])).0, "Ink low");
        // Unlike the L800 table, a 0x4x warning does not hide "Ink low".
        assert_eq!(message_generic(&with(Some(0x04), None, &[0x41, 0x10], &[50])).0, "Ink low");
        assert_eq!(message_generic(&with(Some(0x04), None, &[0x1B], &[50])).0, "Ready");
        assert_eq!(message_generic(&with(Some(0x01), None, &[], &[50])).0, "Printing");
        assert_eq!(message_generic(&with(Some(0x0D), None, &[], &[50])).0, "Printing");
        assert_eq!(message_generic(&with(None, None, &[], &[])).0, "Communication error");
    }

    fn cleaning(secs: Option<u16>) -> Result<Status, QueryError> {
        Ok(Status { cleaning_secs: secs, ..st(0x07) })
    }

    #[test]
    fn monitor_runs_to_done() {
        let mut m = CleaningMonitor::default();
        assert_eq!(m.on_poll(&Err(QueryError::Busy)), Step::Waiting);
        assert_eq!(m.on_poll(&Ok(st(4))), Step::Waiting);
        assert_eq!(m.on_poll(&cleaning(Some(60))), Step::Started(60));
        assert_eq!(m.on_poll(&cleaning(Some(58))), Step::Running);
        assert_eq!(m.on_poll(&Err(QueryError::Failed("x".into()))), Step::Running);
        assert_eq!(m.on_poll(&Ok(st(4))), Step::Running);
        assert_eq!(m.on_poll(&Ok(st(4))), Step::Running);
        assert_eq!(m.on_poll(&Ok(st(4))), Step::Done);
    }

    #[test]
    fn monitor_gives_up() {
        let mut m = CleaningMonitor::default();
        for _ in 0..15 {
            assert_eq!(m.on_poll(&cleaning(None)), Step::Waiting);
        }
        assert_eq!(m.on_poll(&Ok(st(4))), Step::GaveUp);
        let mut m = CleaningMonitor::default();
        let f = Err(QueryError::Failed("x".into()));
        assert_eq!(m.on_poll(&f), Step::Waiting);
        assert_eq!(m.on_poll(&f), Step::Waiting);
        assert_eq!(m.on_poll(&f), Step::GaveUp);
    }

    #[test]
    fn progress_caps_at_97() {
        assert_eq!(progress(Duration::from_secs(30), 60), 0.5);
        assert_eq!(progress(Duration::from_secs(90), 60), 0.97);
    }
}

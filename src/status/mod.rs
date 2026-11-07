//! Printer status for the L800, read the way Windows reads it: the
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
        let r = st2::parse(raw)?;
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

/// True when `lpstat -o <queue>` lists jobs (or cannot be run).
pub fn jobs_queued(queue: &str) -> bool {
    match Command::new("lpstat").args(["-o", queue]).output() {
        Ok(o) => !o.stdout.iter().all(u8::is_ascii_whitespace),
        Err(_) => true,
    }
}

/// One status query. Never touches the printer while CUPS has jobs.
/// Queries are serialised: usblp allows only one opener.
pub fn query(queue: &str) -> Result<Status, QueryError> {
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    if jobs_queued(queue) {
        return Err(QueryError::Busy);
    }
    let fail = QueryError::Failed;
    let path = usblp::find_lp("L800").ok_or_else(|| fail("no /dev/usb/lp* for the L800".into()))?;
    let port = usblp::UsbLp::open(&path).map_err(|e| fail(format!("{}: {e}", path.display())))?;
    let raw = d4::Session::new(port).query().map_err(|e| fail(e.to_string()))?;
    Status::from_reply(&raw).ok_or_else(|| fail("reply is not @BDC ST2".into()))
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

/// Ink names, E_IASRG5P strings 4150-4155, by colour code.
pub fn ink_name(color: u8) -> &'static str {
    match color {
        0 => "Black",
        1 => "Cyan",
        2 => "Magenta",
        3 => "Yellow",
        4 => "Light Cyan",
        5 => "Light Magenta",
        _ => "Ink",
    }
}

/// Status Monitor 3 message: (title, text). Matched in the order of the
/// Windows table `GetStatusCode` (E_IABRG5P, rea/NOTES.md §2.3). Titles are
/// the Windows strings; the texts are our own wording with the same meaning.
pub fn message(s: &Status) -> (&'static str, String) {
    let t = |a: &'static str, b: &str| (a, b.to_string());
    match (s.st, s.er) {
        (Some(0x00), None) => return t("Communication error", COMM),
        (Some(0x00), Some(0x00)) => return t("General error", RESTART),
        (_, Some(0x01)) => return t("Printer busy", "Please wait."),
        (_, Some(0x04)) => return t("Paper jam or CD/DVD tray jam", "For paper, switch the printer off and pull the jammed sheet out by hand. For a CD/DVD, take the tray out. Then press the Paper button on the printer."),
        (_, Some(0x05)) => return t("Check Ink Levels", &format!("{}Check how much ink is left in the printer's ink tanks. {INK_NOTE}", low_inks(s))),
        (_, Some(0x06)) => return t("Paper out or not loaded correctly", "Load the paper again and set the front tray to the lower (paper) position. Then press the Paper button on the printer."),
        (_, Some(0x10)) => return t("Service required", "The printer's ink pads have reached the end of their service life. Contact Epson support."),
        (_, Some(0x12)) => return t("Page not printed or multi-page error", "A page was not printed, several sheets were fed at once, or the wrong paper size was fed. Take the paper out, load it again, and press the Paper button if needed."),
        (_, Some(0x29)) => return t("CD/DVD tray not set correctly", "Load the tray again, then press the Paper button on the printer."),
        (_, Some(0x37)) => return t("Printer cover open", "Close the printer cover."),
        (_, Some(0x38)) => return t("Front tray position error", "For paper, set the front tray to the lower (paper) position. For a CD/DVD, set it to the upper (CD/DVD) position."),
        (Some(0x00), Some(_)) => return t("Unknown error", RESTART),
        (Some(0x08), _) => return t("Initial setup error", "The printer's initial setup has not finished. Complete it on the printer."),
        (Some(0x0A), _) => return t("Printer is shutting down", "Please wait."),
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

/// Highest-priority warning: nibble priority 7, 2, 4, 3, 1 (rea/NOTES.md §2.3).
fn top_warning(w: &[u8]) -> Option<u8> {
    [0x70, 0x20, 0x40, 0x30, 0x10]
        .iter()
        .find_map(|hi| w.iter().copied().find(|&c| c & 0xF0 == *hi))
}

/// "Light Cyan: " style prefix for warnings 0x1n / 0x7n. The slot n is read
/// as the index into field 0x0F (M: it fits the live levels).
fn low_inks(s: &Status) -> String {
    let names: Vec<&str> = s
        .warnings
        .iter()
        .filter(|&&w| matches!(w, 0x10..=0x15 | 0x71..=0x76))
        .filter_map(|&w| s.inks.get((w & 0x0F) as usize - usize::from(w >= 0x71)))
        .map(|i| ink_name(i.color))
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
        let (title, text) = message(&s);
        assert_eq!(title, "Ink low");
        assert!(text.starts_with("Light Cyan: "), "{text}");
        assert_eq!(precheck(&s, true), Precheck::Proceed);
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

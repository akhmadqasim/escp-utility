//! Linux clone of the Windows Epson L800 driver's Maintenance tab.
//!
//! The flows follow the Windows utility (E_IHUTG5P.DLL) on its no-status
//! path, because this version has no bidirectional status channel.
//! See README.md for the evidence.

mod commands;
mod transport;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use slint::ComponentHandle;

slint::include_modules!();

use commands::{Cleaning, Timestamp};
use transport::{Mode, Sent};

// Our own wording, with the meaning of the Windows string named in each comment.

/// Meaning of E_IHSRG5P 4210.
const NOZZLE_INTRO: &str = "Before you print the nozzle check pattern, make sure the printer is switched on and connected to this computer.\n\nTake out the CD/DVD tray and set the front tray to the paper position; otherwise the nozzle check cannot run. Load A4 or Letter plain paper in the sheet feeder.\n\nClick [Print] to print the pattern, or [Cancel] to close.";
/// Meaning of E_IHSRG5P 4211.
const NOZZLE_RESULT: &str = "Compare your printout with the samples above. If lines are missing or broken, click [Clean] to clean the print head. If the pattern is complete, click [Finish].";
/// Meaning of E_IHSRG5P 4201 (shown when there is no status reply; 4200 has the same text).
const CLEAN_INTRO: &str = "First take out the CD/DVD tray and set the front tray to the paper position; otherwise head cleaning cannot run. Select [Cleaning], then click [Start] to clean the print head nozzles.\n\nCleaning uses ink, so clean only when printouts show faint areas or gaps. Run a Nozzle Check first to see whether cleaning is needed.\n\nNote: select [Power Cleaning] only if print quality has not improved after three normal cleanings. Power Cleaning uses more ink than a normal cleaning.\n\nCaution: keep the CD/DVD tray out until cleaning has finished.";
/// Meaning of E_IHSRG5P 10553.
const POWER_CONFIRM: &str = "Do you really want to run Power Cleaning? It uses more ink than a normal cleaning.\n\nBefore you start, check that the transportation lock is in the unlocked (Printing) position, as in the picture below.";
/// Meaning of E_IHSRG5P 4205 (finish text when the utility cannot read status).
const CLEAN_DONE: &str = "Cleaning is finished when the power light on the printer stops flashing.\n\nThen click [Print Nozzle Check Pattern] to check that the cleaning worked.";
/// Meaning of E_IHSRG5P 3000 (shown when the queue is offline or paused).
const COMM_ERROR: &str = "The printer cannot be reached. Check that it is switched on and that the cable is connected. If a nozzle check pattern is printing, switch the printer off and on again to reset it.";
const DRY_RUN_NOTE: &str = "Dry run: no job was sent to the printer.\n\n";

/// The "Please wait..." popup stays up at least this long, so it does not just flicker.
const MIN_WAIT: Duration = Duration::from_millis(700);

#[derive(Clone, Copy)]
enum Utility {
    NozzleCheck,
    HeadCleaning,
}

impl Utility {
    /// Window captions: E_IHSRG5P 1102 / 1101.
    fn title(self) -> &'static str {
        match self {
            Utility::NozzleCheck => "Nozzle Check",
            Utility::HeadCleaning => "Head Cleaning",
        }
    }
    /// E_IHSRG5P 1128 / 1129.
    fn failed(self) -> &'static str {
        match self {
            Utility::NozzleCheck => "Nozzle Check can not be completed.",
            Utility::HeadCleaning => "Head Cleaning can not be completed.",
        }
    }
    fn icon(self) -> slint::Image {
        let svg: &[u8] = match self {
            Utility::NozzleCheck => include_bytes!("../ui/icons/nozzle-check.svg"),
            Utility::HeadCleaning => include_bytes!("../ui/icons/head-cleaning.svg"),
        };
        slint::Image::load_from_svg_data(svg).unwrap_or_default()
    }
}

type JobResult = Result<Sent, String>;
type Continuation = Box<dyn FnOnce(&Rc<App>, JobResult)>;

struct App {
    mode: Mode,
    queue: String,
    main: MainWindow,
    wait: WaitDialog,
    nozzle: NozzleCheckDialog,
    nozzle_result: NozzleResultDialog,
    cleaning: HeadCleaningDialog,
    power: PowerCleaningDialog,
    cleaning_done: CleaningDoneDialog,
    message: MessageDialog,
    info: TextDialog,
    dump: TextDialog,
    /// Result handed from the worker thread to the UI thread.
    job_result: Arc<Mutex<Option<JobResult>>>,
    /// What to do with it once the popup closes.
    on_job_done: RefCell<Option<Continuation>>,
}

impl App {
    fn dry_run(&self) -> bool {
        matches!(self.mode, Mode::DryRun(_))
    }

    fn with_note(&self, text: &str) -> String {
        if self.dry_run() { format!("{DRY_RUN_NOTE}{text}") } else { text.to_string() }
    }

    // Nozzle Check: dialog 210, then popup 201 while spooling, then dialog 211.

    fn start_nozzle_check(&self) {
        self.nozzle.set_body(NOZZLE_INTRO.into());
        let _ = self.nozzle.show();
    }

    fn nozzle_print(self: &Rc<Self>) {
        let _ = self.nozzle.hide();
        let bytes = commands::nozzle_check_job(Timestamp::now());
        self.run_job(Utility::NozzleCheck, "nozzle-check", bytes, |app, res| match res {
            Ok(sent) => {
                app.show_dump(&sent);
                app.nozzle_result.set_body(app.with_note(NOZZLE_RESULT).into());
                let _ = app.nozzle_result.show();
            }
            Err(e) => app.show_error(Utility::NozzleCheck, e),
        });
    }

    // Head Cleaning: dialog 226, [dialog 360 for Power Cleaning], popup 201,
    // then dialog 227. Without status there is no progress dialog (223).

    fn start_head_cleaning(&self) {
        self.cleaning.set_body(CLEAN_INTRO.into());
        self.cleaning.set_power(false);
        let _ = self.cleaning.show();
    }

    fn cleaning_start(self: &Rc<Self>, power: bool) {
        let _ = self.cleaning.hide();
        if power {
            // FUN_004118dc shows dialog 360 when the selected value is not 0
            // and E_IHBRG5P string 5060 exists (it does on the L800).
            self.power.set_body(POWER_CONFIRM.into());
            let _ = self.power.show();
        } else {
            self.run_cleaning(Cleaning::Normal);
        }
    }

    fn run_cleaning(self: &Rc<Self>, kind: Cleaning) {
        let name = match kind {
            Cleaning::Normal => "head-cleaning",
            Cleaning::Power => "power-cleaning",
        };
        let bytes = commands::head_cleaning_job(kind, Timestamp::now());
        self.run_job(Utility::HeadCleaning, name, bytes, |app, res| match res {
            Ok(sent) => {
                app.show_dump(&sent);
                app.cleaning_done.set_body(app.with_note(CLEAN_DONE).into());
                let _ = app.cleaning_done.show();
            }
            Err(e) => app.show_error(Utility::HeadCleaning, e),
        });
    }

    /// Like the Windows worker (FUN_00428d14): show "Please wait...", check
    /// the queue (offline/paused gives message 3000), spool the job, close the
    /// popup, then continue.
    fn run_job(self: &Rc<Self>, u: Utility, name: &'static str, bytes: Vec<u8>, then: impl FnOnce(&Rc<App>, JobResult) + 'static) {
        self.wait.set_utility(u.title().into());
        self.wait.set_glyph(u.icon());
        let _ = self.wait.show();
        *self.on_job_done.borrow_mut() = Some(Box::new(then));

        let mode = self.mode.clone();
        let queue = self.queue.clone();
        let slot = self.job_result.clone();
        let main = self.main.as_weak();
        let started = Instant::now();
        std::thread::spawn(move || {
            let res = match transport::queue_problem(&queue) {
                Some(problem) => Err(format!("{COMM_ERROR}\n\n{problem}")),
                None => transport::submit(&mode, &queue, name, &bytes),
            };
            if let Some(rest) = MIN_WAIT.checked_sub(started.elapsed()) {
                std::thread::sleep(rest);
            }
            *slot.lock().unwrap() = Some(res);
            let _ = main.upgrade_in_event_loop(|m| m.invoke_job_done());
        });
    }

    fn job_done(self: &Rc<Self>) {
        let _ = self.wait.hide();
        let res = self.job_result.lock().unwrap().take();
        let then = self.on_job_done.borrow_mut().take();
        if let (Some(res), Some(then)) = (res, then) {
            then(self, res);
        }
    }

    /// Dialog 271 with one OK button.
    fn show_error(&self, u: Utility, detail: String) {
        self.message.set_utility(u.title().into());
        self.message.set_glyph(u.icon());
        self.message.set_heading(u.failed().into());
        self.message.set_body(detail.into());
        let _ = self.message.show();
    }

    fn show_dump(&self, sent: &Sent) {
        if let Some((_, dump)) = &sent.dump {
            self.dump.set_caption("Dry run".into());
            self.dump.set_heading(sent.summary.clone().into());
            self.dump.set_content(dump.clone().into());
            let _ = self.dump.show();
        }
    }

    fn show_printer_info(&self) {
        self.info.set_caption("Printer and Option Information".into());
        self.info.set_heading(
            format!("The printer is used through the CUPS queue \"{}\". This is what CUPS reports for it (read-only).", self.queue).into(),
        );
        self.info.set_content(transport::queue_info(&self.queue).into());
        let _ = self.info.show();
    }
}

struct Args {
    queue: String,
    dump_dir: Option<PathBuf>,
}

const USAGE: &str = "Usage: epson-l800-utility [--dry-run] [--dump-dir DIR] [--queue NAME]

  --dry-run       write each job to a file and show a hexdump; never calls lp
  --dump-dir DIR  folder for dry-run files (default: ~/.cache/epson-l800-utility)
  --queue NAME    CUPS queue of the printer (default: L800)
";

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut queue = "L800".to_string();
    let mut dry = false;
    let mut dir = None;
    let mut it = args;
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dry-run" => dry = true,
            "--dump-dir" => dir = Some(PathBuf::from(it.next().ok_or("--dump-dir needs a value")?)),
            "--queue" => queue = it.next().ok_or("--queue needs a value")?,
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("epson-l800-utility {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}\n\n{USAGE}")),
        }
    }
    if dir.is_some() && !dry {
        return Err("--dump-dir needs --dry-run".into());
    }
    Ok(Args { queue, dump_dir: dry.then(|| dir.unwrap_or_else(transport::default_dump_dir)) })
}

fn main() -> Result<(), slint::PlatformError> {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let mode = match args.dump_dir {
        Some(dir) => Mode::DryRun(dir),
        None => Mode::Live,
    };

    let main = MainWindow::new()?;
    main.set_queue(args.queue.clone().into());
    main.set_dry_run(matches!(mode, Mode::DryRun(_)));
    main.set_version_text(env!("CARGO_PKG_VERSION").into());

    let app = Rc::new(App {
        mode,
        queue: args.queue,
        main,
        wait: WaitDialog::new()?,
        nozzle: NozzleCheckDialog::new()?,
        nozzle_result: NozzleResultDialog::new()?,
        cleaning: HeadCleaningDialog::new()?,
        power: PowerCleaningDialog::new()?,
        cleaning_done: CleaningDoneDialog::new()?,
        message: MessageDialog::new()?,
        info: TextDialog::new()?,
        dump: TextDialog::new()?,
        job_result: Arc::new(Mutex::new(None)),
        on_job_done: RefCell::new(None),
    });
    app.dump.set_monospace(true);
    wire(&app);

    app.main.show()?;
    slint::run_event_loop()
}

/// Connect every callback. Handlers hold a weak reference to the app.
fn wire(app: &Rc<App>) {
    let on = |f: fn(&Rc<App>)| {
        let w = Rc::downgrade(app);
        move || {
            if let Some(a) = w.upgrade() {
                f(&a)
            }
        }
    };

    let w = Rc::downgrade(app);
    app.main.on_utility_clicked(move |id| {
        let Some(a) = w.upgrade() else { return };
        match id.as_str() {
            "nozzle-check" => a.start_nozzle_check(),
            "head-cleaning" => a.start_head_cleaning(),
            "printer-info" => a.show_printer_info(),
            _ => {}
        }
    });
    app.main.on_close_window(|| {
        let _ = slint::quit_event_loop();
    });
    app.main.window().on_close_requested(|| {
        let _ = slint::quit_event_loop();
        slint::CloseRequestResponse::HideWindow
    });
    app.main.on_job_done(on(|a| a.job_done()));

    // Dialog 201 has no close box on Windows (no WS_SYSMENU); it closes itself.
    app.wait.window().on_close_requested(|| slint::CloseRequestResponse::KeepWindowShown);

    app.nozzle.on_print(on(|a| a.nozzle_print()));
    app.nozzle.on_cancel(on(|a| {
        let _ = a.nozzle.hide();
    }));
    // [Clean] goes to the Head Cleaning utility (state 2 in FUN_00413b60).
    app.nozzle_result.on_clean(on(|a| {
        let _ = a.nozzle_result.hide();
        a.start_head_cleaning();
    }));
    app.nozzle_result.on_finish(on(|a| {
        let _ = a.nozzle_result.hide();
    }));

    let w = Rc::downgrade(app);
    app.cleaning.on_start(move |power| {
        if let Some(a) = w.upgrade() {
            a.cleaning_start(power)
        }
    });
    app.cleaning.on_cancel(on(|a| {
        let _ = a.cleaning.hide();
    }));
    app.power.on_ok(on(|a| {
        let _ = a.power.hide();
        a.run_cleaning(Cleaning::Power);
    }));
    // [Cancel] returns to the Head Cleaning dialog.
    app.power.on_cancel(on(|a| {
        let _ = a.power.hide();
        let _ = a.cleaning.show();
    }));
    // [Print Nozzle Check Pattern] starts the Nozzle Check (state 1).
    app.cleaning_done.on_nozzle_check(on(|a| {
        let _ = a.cleaning_done.hide();
        a.start_nozzle_check();
    }));
    app.cleaning_done.on_finish(on(|a| {
        let _ = a.cleaning_done.hide();
    }));
    app.message.on_ok(on(|a| {
        let _ = a.message.hide();
    }));
    app.info.on_ok(on(|a| {
        let _ = a.info.hide();
    }));
    app.dump.on_ok(on(|a| {
        let _ = a.dump.hide();
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Result<Args, String> {
        parse_args(v.iter().map(|s| s.to_string()))
    }

    #[test]
    fn default_is_live_on_l800() {
        let a = args(&[]).unwrap();
        assert_eq!(a.queue, "L800");
        assert!(a.dump_dir.is_none());
    }

    #[test]
    fn dry_run_flags() {
        let a = args(&["--dry-run", "--dump-dir", "/tmp/x", "--queue", "Q"]).unwrap();
        assert_eq!(a.dump_dir.unwrap(), PathBuf::from("/tmp/x"));
        assert_eq!(a.queue, "Q");
        assert!(args(&["--dump-dir", "/tmp/x"]).is_err());
        assert!(args(&["--bogus"]).is_err());
    }
}

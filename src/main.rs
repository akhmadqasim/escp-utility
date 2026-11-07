//! Linux clone of the Windows Epson L800 driver's Maintenance tab.
//!
//! The flows follow the Windows utility (E_IHUTG5P.DLL). With a status reply
//! it takes the status path (pre-check, dialog 220, progress 223, finish 224).
//! Without one it takes the no-status path (226, then 227). See README.md.

mod commands;
mod status;
mod transport;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Timer, TimerMode};

slint::include_modules!();

use commands::{Cleaning, Timestamp};
use status::{CleaningMonitor, Precheck, QueryError, Status, Step};
use transport::{Mode, Sent};

// Our own wording, with the meaning of the Windows string named in each comment.

/// Meaning of E_IHSRG5P 4210.
const NOZZLE_INTRO: &str = "Before you print the nozzle check pattern, make sure the printer is switched on and connected to this computer.\n\nTake out the CD/DVD tray and set the front tray to the paper position; otherwise the nozzle check cannot run. Load A4 or Letter plain paper in the sheet feeder.\n\nClick [Print] to print the pattern, or [Cancel] to close.";
/// Meaning of E_IHSRG5P 4211.
const NOZZLE_RESULT: &str = "Compare your printout with the samples above. If lines are missing or broken, click [Clean] to clean the print head. If the pattern is complete, click [Finish].";
/// Meaning of E_IHSRG5P 4200 / 4201 (same text).
const CLEAN_INTRO: &str = "First take out the CD/DVD tray and set the front tray to the paper position; otherwise head cleaning cannot run. Select [Cleaning], then click [Start] to clean the print head nozzles.\n\nCleaning uses ink, so clean only when printouts show faint areas or gaps. Run a Nozzle Check first to see whether cleaning is needed.\n\nNote: select [Power Cleaning] only if print quality has not improved after three normal cleanings. Power Cleaning uses more ink than a normal cleaning.\n\nCaution: keep the CD/DVD tray out until cleaning has finished.";
/// Meaning of E_IHSRG5P 10553.
const POWER_CONFIRM: &str = "Do you really want to run Power Cleaning? It uses more ink than a normal cleaning.\n\nBefore you start, check that the transportation lock is in the unlocked (Printing) position, as in the picture below.";
/// Meaning of E_IHSRG5P 4203.
const CLEANING_NOW: &str = "The print head is being cleaned.\nPlease wait.\nCaution: keep the CD/DVD tray out until cleaning has finished.";
/// Meaning of E_IHSRG5P 4204 (status path).
const CLEAN_DONE_STATUS: &str = "The cleaning cycle has finished.\n\nTo check that it worked, click [Print Nozzle Check Pattern].";
/// Meaning of E_IHSRG5P 4205 (no-status path).
const CLEAN_DONE: &str = "Cleaning is finished when the power light on the printer stops flashing.\n\nThen click [Print Nozzle Check Pattern] to check that the cleaning worked.";
/// Meaning of E_IHSRG5P 3000.
const COMM_ERROR: &str = "The printer cannot be reached. Check that it is switched on and that the cable is connected. If a nozzle check pattern is printing, switch the printer off and on again to reset it.";
/// Meaning of E_IHSRG5P 1133.
const SEE_DETAILS: &str = "Click [Details] to see more.";
const DRY_RUN_NOTE: &str = "Dry run: no job was sent to the printer.\n\n";

/// The "Please wait..." popup stays up at least this long, so it does not just flicker.
const MIN_WAIT: Duration = Duration::from_millis(700);
/// Status Monitor 3 polls every 5 s (E_IAIFG5P.DAT LocalMonitoringTimer).
const MONITOR_POLL: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, PartialEq)]
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
    fn icon(self) -> slint::Image {
        let svg: &[u8] = match self {
            Utility::NozzleCheck => include_bytes!("../ui/icons/nozzle-check.svg"),
            Utility::HeadCleaning => include_bytes!("../ui/icons/head-cleaning.svg"),
        };
        slint::Image::load_from_svg_data(svg).unwrap_or_default()
    }
}

/// Why a job was not sent.
enum Fail {
    /// Dialog 270 with [Details] (status said ST=00).
    Problem(String),
    /// Dialog 271.
    Plain(String),
}

type QueryResult = Result<Status, QueryError>;
/// What to do on the UI thread once a job was spooled (or not).
type JobDone = fn(&Rc<App>, Result<Sent, Fail>);

thread_local! {
    static APP: RefCell<Weak<App>> = const { RefCell::new(Weak::new()) };
}

/// Run `work` on a thread, then `then` on the UI thread with its result.
fn bg<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, then: fn(&Rc<App>, T)) {
    std::thread::spawn(move || {
        let value = work();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = APP.with(|a| a.borrow().upgrade()) {
                then(&app, value);
            }
        });
    });
}

/// Run `f` and make it last at least `MIN_WAIT`.
fn at_least_min_wait<T>(f: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let v = f();
    if let Some(rest) = MIN_WAIT.checked_sub(started.elapsed()) {
        std::thread::sleep(rest);
    }
    v
}

/// State of the cleaning progress dialog (223).
#[derive(Default)]
struct Progress {
    monitor: CleaningMonitor,
    started_at: Option<Instant>,
    in_flight: bool,
}

struct App {
    mode: Mode,
    queue: String,
    main: MainWindow,
    wait: WaitDialog,
    nozzle: NozzleCheckDialog,
    nozzle_result: NozzleResultDialog,
    cleaning: HeadCleaningDialog,
    power: PowerCleaningDialog,
    progress: ProgressDialog,
    cleaning_done: CleaningDoneDialog,
    message: MessageDialog,
    monitor: StatusMonitorDialog,
    info: TextDialog,
    dump: TextDialog,
    /// The last pre-check got a status reply (Windows +0x51C / +0x6C).
    have_status: Cell<bool>,
    progress_state: RefCell<Progress>,
    poll_timer: Timer,
    bar_timer: Timer,
    monitor_timer: Timer,
    monitor_in_flight: Cell<bool>,
}

impl App {
    fn dry_run(&self) -> bool {
        matches!(self.mode, Mode::DryRun(_))
    }

    fn with_note(&self, text: &str) -> String {
        if self.dry_run() { format!("{DRY_RUN_NOTE}{text}") } else { text.to_string() }
    }

    fn show_wait(&self, u: Utility) {
        self.wait.set_utility(u.title().into());
        self.wait.set_glyph(u.icon());
        let _ = self.wait.show();
    }

    // Pre-check (FUN_004189c8): popup 201 while the status is read, then
    // either an error message or the first dialog of the utility.

    fn start(&self, u: Utility) {
        self.show_wait(u);
        let queue = self.queue.clone();
        bg(move || (u, at_least_min_wait(|| status::query(&queue))), |app, (u, res)| app.after_precheck(u, res));
    }

    fn after_precheck(&self, u: Utility, res: QueryResult) {
        let _ = self.wait.hide();
        // No reply (or CUPS has jobs, so we did not ask): the no-status path.
        let Ok(s) = res else {
            self.have_status.set(false);
            return self.show_intro(u);
        };
        self.have_status.set(true);
        let cleaning = u == Utility::HeadCleaning;
        match status::precheck(&s, cleaning) {
            Precheck::Proceed => self.show_intro(u),
            // Dialog 270: strings 1127 + 1133.
            Precheck::Problem => self.show_message(u, "A problem has occurred.", SEE_DETAILS, true),
            // Dialog 271: strings 1124 / 1125.
            Precheck::CannotComplete => self.show_message(
                u,
                if cleaning { "Cannot complete Head Cleaning." } else { "Cannot complete Nozzle Check." },
                "",
                false,
            ),
            // Dialog 271: strings 1132 and 1131.
            Precheck::CleaningBlocked => self.show_message(u, "Head Cleaning can not be completed.", "", false),
            Precheck::NowCleaning => self.show_message(u, "Now cleaning!", "", false),
        }
    }

    fn show_intro(&self, u: Utility) {
        match u {
            Utility::NozzleCheck => {
                self.nozzle.set_body(NOZZLE_INTRO.into());
                let _ = self.nozzle.show();
            }
            // Dialog 220 with status, 226 without: same layout and text.
            Utility::HeadCleaning => {
                self.cleaning.set_body(CLEAN_INTRO.into());
                self.cleaning.set_power(false);
                let _ = self.cleaning.show();
            }
        }
    }

    fn nozzle_print(&self) {
        let _ = self.nozzle.hide();
        self.run_job(Utility::NozzleCheck, "nozzle-check", commands::nozzle_check_job(Timestamp::now()), |app, res| {
            match res {
                Ok(sent) => {
                    app.show_dump(&sent);
                    app.nozzle_result.set_body(app.with_note(NOZZLE_RESULT).into());
                    let _ = app.nozzle_result.show();
                }
                Err(f) => app.show_fail(Utility::NozzleCheck, f),
            }
        });
    }

    fn cleaning_start(&self, power: bool) {
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

    fn run_cleaning(&self, kind: Cleaning) {
        let name = match kind {
            Cleaning::Normal => "head-cleaning",
            Cleaning::Power => "power-cleaning",
        };
        self.run_job(Utility::HeadCleaning, name, commands::head_cleaning_job(kind, Timestamp::now()), |app, res| {
            match res {
                Ok(sent) => {
                    app.show_dump(&sent);
                    if app.have_status.get() && !app.dry_run() {
                        app.start_progress();
                    } else {
                        app.show_done(false);
                    }
                }
                Err(f) => app.show_fail(Utility::HeadCleaning, f),
            }
        });
    }

    /// The Windows worker (FUN_00428d14): popup 201, re-read status (ST=00
    /// stops with message 3000), check the spooler (offline/paused stops with
    /// 3000), spool the job. In dry-run the queue check is only a note.
    fn run_job(&self, u: Utility, name: &'static str, bytes: Vec<u8>, then: JobDone) {
        self.show_wait(u);
        let (mode, queue, have_status) = (self.mode.clone(), self.queue.clone(), self.have_status.get());
        bg(
            move || {
                let res = at_least_min_wait(|| {
                    if have_status && matches!(status::query(&queue), Ok(Status { st: Some(0x00), .. })) {
                        return Err(Fail::Problem(COMM_ERROR.into()));
                    }
                    match (transport::queue_problem(&queue), &mode) {
                        (Some(p), Mode::Live) => Err(Fail::Plain(format!("{COMM_ERROR}\n\n{p}"))),
                        (Some(p), Mode::DryRun(_)) => transport::submit(&mode, &queue, name, &bytes)
                            .map(|mut s| {
                                s.summary.push_str(&format!("\nA real run would stop here with a communication error: {p}"));
                                s
                            })
                            .map_err(Fail::Plain),
                        (None, _) => transport::submit(&mode, &queue, name, &bytes).map_err(Fail::Plain),
                    }
                });
                (then, res)
            },
            |app, (then, v)| {
                let _ = app.wait.hide();
                then(app, v)
            },
        );
    }

    // Progress dialog 223 (FUN_004123b8, FUN_00411e5c, FUN_00412ae8).

    fn start_progress(&self) {
        *self.progress_state.borrow_mut() = Progress::default();
        self.progress.set_body("Please wait...".into());
        self.progress.set_bar_visible(false);
        self.progress.set_value(0.0);
        let _ = self.progress.show();
        self.poll_timer.start(TimerMode::Repeated, status::POLL, || {
            if let Some(app) = APP.with(|a| a.borrow().upgrade()) {
                app.poll_cleaning();
            }
        });
    }

    fn poll_cleaning(&self) {
        let mut p = self.progress_state.borrow_mut();
        if p.in_flight {
            return;
        }
        p.in_flight = true;
        let queue = self.queue.clone();
        bg(move || status::query(&queue), |app, res| app.on_cleaning_poll(res));
    }

    fn on_cleaning_poll(&self, res: QueryResult) {
        let step = {
            let mut p = self.progress_state.borrow_mut();
            p.in_flight = false;
            p.monitor.on_poll(&res)
        };
        match step {
            Step::Waiting | Step::Running => {}
            Step::Started(_) => {
                self.progress_state.borrow_mut().started_at = Some(Instant::now());
                self.progress.set_body(CLEANING_NOW.into());
                self.progress.set_bar_visible(true);
                self.bar_timer.start(TimerMode::Repeated, Duration::from_secs(1), || {
                    if let Some(app) = APP.with(|a| a.borrow().upgrade()) {
                        let p = app.progress_state.borrow();
                        if let (Some(at), Some(secs)) = (p.started_at, p.monitor.started_secs) {
                            app.progress.set_value(status::progress(at.elapsed(), secs));
                        }
                    }
                });
            }
            Step::Done => {
                self.poll_timer.stop();
                self.bar_timer.stop();
                self.progress.set_value(1.0);
                Timer::single_shot(Duration::from_millis(500), || {
                    if let Some(app) = APP.with(|a| a.borrow().upgrade()) {
                        let _ = app.progress.hide();
                        app.show_done(true);
                    }
                });
            }
            Step::GaveUp => {
                self.poll_timer.stop();
                self.bar_timer.stop();
                let _ = self.progress.hide();
                self.show_done(false);
            }
        }
    }

    /// Dialog 224 (status: string 4204) or 227 (no status: 4205).
    fn show_done(&self, finished_seen: bool) {
        let text = if finished_seen { CLEAN_DONE_STATUS } else { CLEAN_DONE };
        self.cleaning_done.set_body(self.with_note(text).into());
        let _ = self.cleaning_done.show();
    }

    fn show_fail(&self, u: Utility, f: Fail) {
        match f {
            Fail::Problem(text) => self.show_message(u, &text, SEE_DETAILS, true),
            // Headings: E_IHSRG5P 1128 / 1129.
            Fail::Plain(detail) => self.show_message(
                u,
                match u {
                    Utility::NozzleCheck => "Nozzle Check can not be completed.",
                    Utility::HeadCleaning => "Head Cleaning can not be completed.",
                },
                &detail,
                false,
            ),
        }
    }

    /// Dialog 270 (with [Details], which opens Status Monitor 3) or 271.
    fn show_message(&self, u: Utility, heading: &str, body: &str, details: bool) {
        self.message.set_utility(u.title().into());
        self.message.set_glyph(u.icon());
        self.message.set_heading(heading.into());
        self.message.set_body(body.into());
        self.message.set_show_details(details);
        let _ = self.message.show();
    }

    // Status Monitor 3 window.

    fn open_monitor(&self) {
        self.monitor.set_printer(self.queue.clone().into());
        self.monitor.set_title_text("Please wait...".into());
        self.monitor.set_body("".into());
        let _ = self.monitor.show();
        self.poll_monitor();
        self.monitor_timer.start(TimerMode::Repeated, MONITOR_POLL, || {
            if let Some(app) = APP.with(|a| a.borrow().upgrade()) {
                app.poll_monitor();
            }
        });
    }

    fn poll_monitor(&self) {
        if self.monitor_in_flight.replace(true) {
            return;
        }
        let queue = self.queue.clone();
        bg(move || status::query(&queue), |app, res| {
            app.monitor_in_flight.set(false);
            app.show_monitor_status(res);
        });
    }

    fn show_monitor_status(&self, res: QueryResult) {
        match res {
            Ok(s) => {
                let (title, text) = status::message(&s);
                self.monitor.set_title_text(title.into());
                self.monitor.set_body(text.into());
                let inks: Vec<InkLevel> = s
                    .inks
                    .iter()
                    .map(|i| InkLevel {
                        name: status::ink_name(i.color).into(),
                        color: ink_color(i.color),
                        level: i.percent().map_or(-1, i32::from),
                    })
                    .collect();
                self.monitor.set_inks(std::rc::Rc::new(slint::VecModel::from(inks)).into());
            }
            // CUPS has jobs: keep the last reading, say it is printing.
            Err(QueryError::Busy) => {
                self.monitor.set_title_text("Printing".into());
                self.monitor.set_body("Please wait.".into());
            }
            Err(QueryError::Failed(e)) => {
                self.monitor.set_title_text("Communication error".into());
                self.monitor.set_body(format!("{}\n\n({e})", status::COMM).into());
                self.monitor.set_inks(slint::ModelRc::default());
            }
        }
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

/// Bar colour for each ink (colour code from field 0x0F).
fn ink_color(code: u8) -> slint::Color {
    let rgb = match code {
        0 => 0x202020,
        1 => 0x00a0e0,
        2 => 0xd0168c,
        3 => 0xf0c800,
        4 => 0x7fd0f0,
        5 => 0xf08cc8,
        _ => 0x808080,
    };
    slint::Color::from_argb_encoded(0xff00_0000 | rgb)
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
        progress: ProgressDialog::new()?,
        cleaning_done: CleaningDoneDialog::new()?,
        message: MessageDialog::new()?,
        monitor: StatusMonitorDialog::new()?,
        info: TextDialog::new()?,
        dump: TextDialog::new()?,
        have_status: Cell::new(false),
        progress_state: RefCell::new(Progress::default()),
        poll_timer: Timer::default(),
        bar_timer: Timer::default(),
        monitor_timer: Timer::default(),
        monitor_in_flight: Cell::new(false),
    });
    APP.with(|a| *a.borrow_mut() = Rc::downgrade(&app));
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
            "status-monitor" => a.open_monitor(),
            "nozzle-check" => a.start(Utility::NozzleCheck),
            "head-cleaning" => a.start(Utility::HeadCleaning),
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

    // Dialogs 201 and 223 have no way out on Windows; they close themselves.
    app.wait.window().on_close_requested(|| slint::CloseRequestResponse::KeepWindowShown);
    app.progress.window().on_close_requested(|| slint::CloseRequestResponse::KeepWindowShown);

    app.nozzle.on_print(on(|a| a.nozzle_print()));
    app.nozzle.on_cancel(on(|a| {
        let _ = a.nozzle.hide();
    }));
    // [Clean] goes to the Head Cleaning utility (state 2 in FUN_00413b60).
    app.nozzle_result.on_clean(on(|a| {
        let _ = a.nozzle_result.hide();
        a.start(Utility::HeadCleaning);
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
        a.start(Utility::NozzleCheck);
    }));
    app.cleaning_done.on_finish(on(|a| {
        let _ = a.cleaning_done.hide();
    }));
    app.message.on_ok(on(|a| {
        let _ = a.message.hide();
    }));
    app.message.on_details(on(|a| {
        let _ = a.message.hide();
        a.open_monitor();
    }));
    // "Print Queue": the GNOME printer settings show the CUPS queue.
    app.monitor.on_print_queue(|| {
        if let Ok(mut child) = std::process::Command::new("gnome-control-center").arg("printers").spawn() {
            std::thread::spawn(move || child.wait());
        }
    });
    app.monitor.on_ok(on(|a| {
        a.monitor_timer.stop();
        let _ = a.monitor.hide();
    }));
    let w = Rc::downgrade(app);
    app.monitor.window().on_close_requested(move || {
        if let Some(a) = w.upgrade() {
            a.monitor_timer.stop();
        }
        slint::CloseRequestResponse::HideWindow
    });
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

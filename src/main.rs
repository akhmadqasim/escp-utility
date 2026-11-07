//! Linux clone of the Windows Epson L800 driver's Maintenance tab.
//!
//! The flows follow the Windows utility (E_IHUTG5P.DLL). With a status reply
//! it takes the status path (pre-check, dialog 220, progress 223, finish 224).
//! Without one it takes the no-status path (226, then 227). See README.md.

mod commands;
mod layouts;
mod printer;
mod settings;
mod status;
mod texts;
mod transport;

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Timer, TimerMode};

slint::include_modules!();

use commands::{Cleaning, Timestamp};
use printer::Identity;
use settings::Values;
use status::{CleaningMonitor, Precheck, QueryError, Status, Step};
use texts::*;
use transport::{Mode, Sent};

/// Epson's download page for the L800 Linux drivers (Windows runs its own updater, E_GUPA30.EXE).
const DRIVER_PAGE: &str = "https://download-center.epson.com/softwares/?device_id=L800&region=ID&os=DEBX64&language=en";

/// Wayland app_id / X11 WM_CLASS; matches the desktop file and icon name.
const APP_ID: &str = "epson-l800-utility";

/// The "Please wait..." popup stays up at least this long, so it does not just flicker.
const MIN_WAIT: Duration = Duration::from_millis(700);
/// Status Monitor 3 polls every 5 s (E_IAIFG5P.DAT LocalMonitoringTimer).
const MONITOR_POLL: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, PartialEq)]
enum Utility {
    NozzleCheck,
    HeadCleaning,
    PaperGuide,
}

impl Utility {
    /// Window captions: E_IHSRG5P 1102 / 1101.
    fn title(self) -> &'static str {
        match self {
            Utility::NozzleCheck => "Nozzle Check",
            Utility::HeadCleaning => "Head Cleaning",
            // String 1112 is absent from E_IHSRG5P: Windows shows no caption.
            Utility::PaperGuide => "",
        }
    }
    fn icon(self) -> slint::Image {
        let svg: &[u8] = match self {
            Utility::NozzleCheck => include_bytes!("../ui/icons/nozzle-check.svg"),
            Utility::HeadCleaning => include_bytes!("../ui/icons/head-cleaning.svg"),
            Utility::PaperGuide => include_bytes!("../ui/icons/paper-guide.svg"),
        };
        slint::Image::load_from_svg_data(svg).unwrap_or_default()
    }
    /// Which column of the Windows message tables the utility uses. Paper
    /// Guide Cleaning uses the Nozzle Check one (FUN_0041a2f8: state 0xF ->
    /// column 0), so it is not "cleaning" here.
    fn cleaning(self) -> bool {
        self == Utility::HeadCleaning
    }
}

/// Why a job was not sent.
enum Fail {
    /// Dialog 270 with [Details] (status said ST=00).
    Problem,
    /// Dialog 271.
    Plain(String),
}

type QueryResult = Result<Status, QueryError>;

/// One status query on the printer's usblp node (none: no status path).
fn query(p: &Identity) -> QueryResult {
    status::query(&p.queue, p.status_node().as_deref())
}
/// What to do on the UI thread once a job was spooled (or not).
type JobDone = fn(&Rc<App>, Result<Sent, Fail>);

thread_local! {
    static APP: RefCell<Weak<App>> = const { RefCell::new(Weak::new()) };
}

/// The app, while it is alive (UI thread only).
fn app() -> Option<Rc<App>> {
    APP.with(|a| a.borrow().upgrade())
}

/// A callback or timer handler that runs `f` with the app.
fn on(f: fn(&Rc<App>)) -> impl Fn() + 'static {
    move || {
        if let Some(a) = app() {
            f(&a)
        }
    }
}

/// Start a desktop program and reap it on a thread when it exits.
fn launch(program: &str, arg: &str) {
    match std::process::Command::new(program).arg(arg).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(e) => eprintln!("epson-l800-utility: cannot start {program}: {e}"),
    }
}

/// Run `work` on a thread, then `then` on the UI thread with its result.
fn bg<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, then: fn(&Rc<App>, T)) {
    std::thread::spawn(move || {
        let value = work();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(app) = app() {
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

/// A dialog that is created (and wired) on first use. Slint's winit backend
/// creates a toplevel for every window component once the event loop runs,
/// shown or not, and GNOME then lists each as a separate, unknown app. On
/// Wayland a window that is hidden again is dropped by Slint (X11 unmaps
/// it), so with this only the dialogs that are open exist on screen. Check
/// a dialog with `shown`, never through its window: that creates it.
type Lazy<T> = std::cell::LazyCell<T, fn() -> T>;

/// A dialog that is open (never-created dialogs are not).
fn shown<T: ComponentHandle>(d: &Lazy<T>) -> bool {
    Lazy::get(d).is_some_and(|d| d.window().is_visible())
}

/// Create a dialog component; the platform exists once the sheet does.
/// Its close box hides it and lets the sheet take clicks again; the makers
/// of 201, 223, Status Monitor 3 and Current Settings replace that handler.
fn make<T: ComponentHandle + 'static>(new: fn() -> Result<T, slint::PlatformError>) -> T
where
    for<'a> Frame<'a>: slint::Global<'a, T>,
{
    let d = new().expect("cannot create a dialog window");
    frame(&d);
    refresh_on_close(d.window());
    d
}

/// The windows draw their own frame (ui/common.slint `Frame`): GNOME's
/// client-side frame always offers maximize. Our Close sends the window a
/// close request, as the system close box did, so each window's
/// `on_close_requested` still decides. Sent from a timer, not from inside
/// the click that is still being handled.
fn frame<T: ComponentHandle + 'static>(c: &T)
where
    for<'a> Frame<'a>: slint::Global<'a, T>,
{
    let weak = c.as_weak();
    c.global::<Frame>().on_close(move || {
        let weak = weak.clone();
        Timer::single_shot(Duration::ZERO, move || {
            if let Some(c) = weak.upgrade()
                && let Err(e) = c.window().dispatch_event_with_result(slint::platform::WindowEvent::CloseRequested)
            {
                eprintln!("epson-l800-utility: cannot close the window: {e}");
            }
        });
    });
    fixed_size(c.window());
}

/// Windows keep their fixed size. They have no maximize button (see
/// `frame`), but a compositor shortcut can still maximize or fullscreen
/// them; that resizes the window, and then we ask for the normal state back.
/// Only once per change: a compositor that keeps the state answers each
/// request with another configure (and resize), which must not loop.
fn fixed_size(w: &slint::Window) {
    use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};
    // (fullscreen, maximized) as the last resize found them.
    let mut last = (false, false);
    w.on_winit_window_event(move |w, event| {
        if let WindowEvent::Resized(_) = event {
            w.with_winit_window(|w| {
                let now = (w.fullscreen().is_some(), w.is_maximized());
                if now.0 && !last.0 {
                    w.set_fullscreen(None);
                }
                if now.1 && !last.1 {
                    w.set_maximized(false);
                }
                last = now;
            });
        }
        EventResult::Propagate
    });
}

/// The sheet takes clicks again when a dialog is closed with its close box.
fn refresh_on_close(w: &slint::Window) {
    w.on_close_requested(|| {
        Timer::single_shot(Duration::ZERO, on(|a| a.refresh_busy()));
        slint::CloseRequestResponse::HideWindow
    });
}

/// State of the cleaning progress dialog (223).
#[derive(Default)]
struct Progress {
    monitor: CleaningMonitor,
    in_flight: bool,
}

struct App {
    mode: Mode,
    /// The queue and the printer behind it.
    printer: Identity,
    main: MainWindow,
    wait: Lazy<WaitDialog>,
    nozzle: Lazy<NozzleCheckDialog>,
    nozzle_result: Lazy<NozzleResultDialog>,
    cleaning: Lazy<HeadCleaningDialog>,
    power: Lazy<PowerCleaningDialog>,
    progress: Lazy<ProgressDialog>,
    cleaning_done: Lazy<CleaningDoneDialog>,
    paper_guide: Lazy<PaperGuideDialog>,
    message: Lazy<MessageDialog>,
    monitor: Lazy<StatusMonitorDialog>,
    info: Lazy<TextDialog>,
    dump: Lazy<TextDialog>,
    current_settings: Lazy<CurrentSettingsDialog>,
    color_controls: Lazy<ColorControlsDialog>,
    layout_dialog: Lazy<LayoutDialog>,
    /// The last pre-check got a status reply (Windows +0x51C / +0x6C).
    have_status: Cell<bool>,
    progress_state: RefCell<Progress>,
    poll_timer: Timer,
    bar_timer: Timer,
    monitor_timer: Timer,
    monitor_in_flight: Cell<bool>,
    /// `lpoptions` is loading or saving the preferences: the sheet waits.
    settings_io: Cell<bool>,
}

impl App {
    /// The property sheet; the dialogs are created when first opened.
    fn new(mode: Mode, printer: Identity) -> Result<Rc<App>, slint::PlatformError> {
        let main = MainWindow::new()?;
        main.set_queue(printer.queue.clone().into());
        main.set_maintenance(printer.maintenance());
        main.set_version_text(env!("CARGO_PKG_VERSION").into());
        Ok(Rc::new(App {
            mode,
            printer,
            main,
            wait: Lazy::new(dialogs::wait),
            nozzle: Lazy::new(dialogs::nozzle),
            nozzle_result: Lazy::new(dialogs::nozzle_result),
            cleaning: Lazy::new(dialogs::cleaning),
            power: Lazy::new(dialogs::power),
            progress: Lazy::new(dialogs::progress),
            cleaning_done: Lazy::new(dialogs::cleaning_done),
            paper_guide: Lazy::new(dialogs::paper_guide),
            message: Lazy::new(dialogs::message),
            monitor: Lazy::new(dialogs::monitor),
            info: Lazy::new(dialogs::info),
            dump: Lazy::new(dialogs::dump),
            current_settings: Lazy::new(dialogs::current_settings),
            color_controls: Lazy::new(dialogs::color_controls),
            layout_dialog: Lazy::new(dialogs::layout_dialog),
            have_status: Cell::new(false),
            progress_state: RefCell::new(Progress::default()),
            poll_timer: Timer::default(),
            bar_timer: Timer::default(),
            monitor_timer: Timer::default(),
            monitor_in_flight: Cell::new(false),
            settings_io: Cell::new(false),
        }))
    }

    fn open<T: ComponentHandle>(&self, d: &Lazy<T>) {
        let _ = d.show();
        self.refresh_busy();
    }

    fn close<T: ComponentHandle>(&self, d: &Lazy<T>) {
        let _ = d.hide();
        self.refresh_busy();
    }

    /// Windows runs the utility dialogs modally: while one is open, the
    /// property sheet ignores clicks.
    fn refresh_busy(&self) {
        // Every dialog that blocks the property sheet while it is open (the
        // Current Settings window is modeless).
        let open = [
            shown(&self.wait),
            shown(&self.nozzle),
            shown(&self.nozzle_result),
            shown(&self.cleaning),
            shown(&self.power),
            shown(&self.progress),
            shown(&self.cleaning_done),
            shown(&self.paper_guide),
            shown(&self.message),
            shown(&self.monitor),
            shown(&self.info),
            shown(&self.dump),
            shown(&self.color_controls),
            shown(&self.layout_dialog),
        ];
        self.main.set_busy(self.settings_io.get() || open.contains(&true));
    }

    /// Show or hide the Current Settings window (dialog 4050).
    fn show_settings(&self, on: bool) {
        if on {
            let _ = self.current_settings.show();
            self.refresh_current_settings();
        } else if let Some(d) = Lazy::get(&self.current_settings) {
            let _ = d.hide();
        }
        self.main.global::<Prefs>().set_settings_shown(on);
    }

    // Printing Preferences (settings.rs): the four settings pages edit the
    // Prefs global; OK saves and closes, Apply saves, Cancel discards.

    fn set_prefs(&self, v: &Values) {
        // Values loaded from ~/.cups/lpoptions can break the constraints too.
        let v = &v.constrained();
        let p = self.main.global::<Prefs>();
        // The source-dependent lists and the selected row in each.
        let labels = |rows: &[usize], table: &[settings::Choice]| strings(rows.iter().map(|&i| table[i].label));
        let pos = |rows: &[usize], i: usize| rows.iter().position(|&r| r == i).unwrap_or(0) as i32;
        let (media, sizes, outputs) = (v.media_list(), v.size_list(), v.output_list());
        p.set_media_labels(labels(&media, settings::MEDIA));
        p.set_media_pos(pos(&media, v.media));
        p.set_size_labels(labels(&sizes, settings::SIZES));
        p.set_size_pos(pos(&sizes, v.size));
        p.set_output_labels(labels(&outputs, settings::output_sizes()));
        p.set_output_pos(pos(&outputs, v.output_size));
        let qualities = settings::advanced_qualities(v.media);
        p.set_adv_quality_labels(strings(qualities.iter().map(|q| q.0)));
        p.set_adv_quality_pos(settings::advanced_quality_pos(v.media, v.quality) as i32);
        p.set_quality(v.quality as i32);
        p.set_source(v.source as i32);
        p.set_media(v.media as i32);
        p.set_size(v.size as i32);
        p.set_borderless(v.borderless);
        p.set_landscape(v.landscape);
        p.set_grayscale(v.grayscale);
        p.set_reverse(v.reverse);
        p.set_color_mode(v.color_mode as i32);
        p.set_gamma(v.gamma as i32);
        p.set_reduce(v.reduce);
        p.set_by_percent(v.by_percent);
        p.set_output_size(v.output_size as i32);
        p.set_scale(v.scale as i32);
        p.set_multi_page(v.multi_page);
        p.set_poster(v.poster);
        p.set_pages_index(v.pages_index as i32);
        p.set_poster_index(v.poster_index as i32);
        p.set_page_borders(v.page_borders);
        p.set_copies(v.copies as i32);
        p.set_collate(v.collate);
        p.set_rotate180(v.rotate180);
        p.set_mirror(v.mirror);
        p.set_brightness(v.brightness);
        p.set_contrast(v.contrast);
        p.set_saturation(v.saturation);
        p.set_cyan(v.cyan);
        p.set_magenta(v.magenta);
        p.set_yellow(v.yellow);
        self.refresh_current_settings();
    }

    fn prefs(&self) -> Values {
        let p = self.main.global::<Prefs>();
        let ix = |i: i32| usize::try_from(i).unwrap_or(0);
        Values {
            quality: ix(p.get_quality()),
            source: ix(p.get_source()),
            media: ix(p.get_media()),
            size: ix(p.get_size()),
            borderless: p.get_borderless(),
            landscape: p.get_landscape(),
            grayscale: p.get_grayscale(),
            reverse: p.get_reverse(),
            color_mode: ix(p.get_color_mode()),
            gamma: ix(p.get_gamma()),
            reduce: p.get_reduce(),
            by_percent: p.get_by_percent(),
            output_size: ix(p.get_output_size()),
            scale: p.get_scale().clamp(10, 400) as u32,
            multi_page: p.get_multi_page(),
            poster: p.get_poster(),
            pages_index: ix(p.get_pages_index()),
            poster_index: ix(p.get_poster_index()),
            page_borders: p.get_page_borders(),
            copies: p.get_copies().clamp(1, 9999) as u32,
            collate: p.get_collate(),
            rotate180: p.get_rotate180(),
            mirror: p.get_mirror(),
            brightness: p.get_brightness(),
            contrast: p.get_contrast(),
            saturation: p.get_saturation(),
            cyan: p.get_cyan(),
            magenta: p.get_magenta(),
            yellow: p.get_yellow(),
        }
        .constrained()
    }

    /// The sub-dialogs of the settings pages.
    fn open_dialog(&self, id: &str) {
        let layout = match id {
            "color-controls" => {
                let v = self.prefs();
                let d = &self.color_controls;
                d.set_color_mode(v.color_mode as i32);
                d.set_gamma(v.gamma as i32);
                d.set_brightness(v.brightness);
                d.set_contrast(v.contrast);
                d.set_saturation(v.saturation);
                d.set_cyan(v.cyan);
                d.set_magenta(v.magenta);
                d.set_yellow(v.yellow);
                return self.open(d);
            }
            "custom-settings" => &layouts::CUSTOM_SETTINGS,
            "page-order" => &layouts::PAGE_ORDER,
            "poster" => &layouts::POSTER,
            "margins" => &layouts::MARGINS,
            "watermark" => &layouts::WATERMARK,
            _ => return,
        };
        let d = &self.layout_dialog;
        d.set_caption(layout.caption.into());
        d.set_reason(layout.reason.into());
        d.set_client_w(layout.size.0);
        d.set_client_h(layout.size.1);
        let controls: Vec<LayoutControl> = layout
            .controls
            .iter()
            .map(|&(kind, x, y, w, h, text)| LayoutControl { kind, x, y, w, h, text: text.into() })
            .collect();
        d.set_controls(slint::VecModel::from_slice(&controls));
        self.open(d);
    }

    /// Color Controls [OK]: take its values into the sheet (Apply/OK saves them).
    fn color_controls_ok(&self) {
        let d = &self.color_controls;
        let mut v = self.prefs();
        v.color_mode = usize::try_from(d.get_color_mode()).unwrap_or(1);
        v.gamma = usize::try_from(d.get_gamma()).unwrap_or(1);
        v.brightness = d.get_brightness();
        v.contrast = d.get_contrast();
        v.saturation = d.get_saturation();
        v.cyan = d.get_cyan();
        v.magenta = d.get_magenta();
        v.yellow = d.get_yellow();
        self.set_prefs(&v);
        let p = self.main.global::<Prefs>();
        p.set_dirty(true);
        p.set_preset(-1);
        self.close(d);
    }

    /// The Current Settings window (dialog 4050): labels are strings 4053-4079.
    fn refresh_current_settings(&self) {
        use settings::*;
        let Some(dialog) = Lazy::get(&self.current_settings) else { return };
        let v = self.prefs();
        let on_off = |b: bool| if b { "On" } else { "Off" }.to_string();
        let lines = [
            ("Paper Source", SOURCES[v.source].label.to_string()),
            ("Paper Type", MEDIA[v.media].label.to_string()),
            ("Quality", QUALITY[v.quality].label.to_string()),
            ("Document Size", SIZES[v.size].label.to_string()),
            ("Borderless", on_off(v.borderless)),
            ("Orientation", if v.landscape { "Landscape" } else { "Portrait" }.to_string()),
            ("Reverse Order", on_off(v.reverse)),
            ("Grayscale", on_off(v.grayscale)),
            ("Color Management", "Color Controls".to_string()),
            ("Color Mode", COLOR_MODES[v.color_mode].label.to_string()),
            (
                "Page Layout",
                match (v.multi_page, v.poster) {
                    (false, _) => "Off".to_string(),
                    (true, false) => format!("Pages per Sheet {}", PAGES_PER_SHEET[v.pages_index]),
                    (true, true) => format!("Poster Printing {}", POSTERS[v.poster_index].1),
                },
            ),
            (
                "Scale ratio",
                match (v.reduce_active(), v.by_percent) {
                    (false, _) => "Off".to_string(),
                    (true, true) => format!("{} %", v.scale),
                    (true, false) => output_sizes()[v.output_size].label.to_string(),
                },
            ),
        ];
        let lines: Vec<SettingLine> =
            lines.into_iter().map(|(label, value)| SettingLine { label: label.into(), value: value.into() }).collect();
        dialog.set_lines(slint::VecModel::from_slice(&lines));
    }

    /// Save for this user with `lpoptions -p <queue> -o ...` (~/.cups/lpoptions),
    /// off the UI thread; the sheet waits meanwhile. In dry-run the command is
    /// only shown. `quit` (OK) closes the sheet once saved.
    fn save_prefs(&self, quit: bool) {
        // Number edits commit: their text now shows the (clamped) value saved.
        let p = self.main.global::<Prefs>();
        p.set_commit_tick(p.get_commit_tick() + 1);
        let v = self.prefs();
        if self.dry_run() {
            let args = settings::save_args(&self.printer.queue, &v);
            self.dump.set_caption("Dry run".into());
            self.dump.set_heading("Dry run: these settings were not saved. A real run would run this command:".into());
            self.dump.set_content(format!("lpoptions {}\n", args.join(" \\\n    ")).into());
            self.open(&self.dump);
            p.set_dirty(false);
            return;
        }
        self.settings_io.set(true);
        self.refresh_busy();
        let queue = self.printer.queue.clone();
        bg(move || (quit, settings::save(&queue, &v)), |app, (quit, res)| {
            app.settings_io.set(false);
            app.refresh_busy();
            match res {
                Ok(()) => {
                    app.main.global::<Prefs>().set_dirty(false);
                    if quit {
                        let _ = slint::quit_event_loop();
                    }
                }
                Err(e) => {
                    app.message.set_utility("Printing Preferences".into());
                    app.message.set_heading("The settings could not be saved.".into());
                    app.message.set_body(e.into());
                    app.message.set_show_details(false);
                    app.open(&app.message);
                }
            }
        });
    }

    fn dry_run(&self) -> bool {
        matches!(self.mode, Mode::DryRun(_))
    }

    fn with_note(&self, text: &str) -> String {
        if self.dry_run() { format!("{DRY_RUN_NOTE}{text}") } else { text.to_string() }
    }

    fn show_wait(&self, u: Utility) {
        self.wait.set_utility(u.title().into());
        self.wait.set_glyph(u.icon());
        self.open(&self.wait);
    }

    // Pre-check (FUN_004189c8): popup 201 while the status is read, then
    // either an error message or the first dialog of the utility.

    fn start(&self, u: Utility) {
        // Windows runs these dialogs modally: no second utility while one is
        // reading status, spooling, or showing cleaning progress.
        if shown(&self.wait) || shown(&self.progress) {
            return;
        }
        self.show_wait(u);
        let p = self.printer.clone();
        bg(move || (u, at_least_min_wait(|| query(&p))), |app, (u, res)| app.after_precheck(u, res));
    }

    fn after_precheck(&self, u: Utility, res: QueryResult) {
        self.close(&self.wait);
        // No reply (or CUPS has jobs, so we did not ask): the no-status path.
        let Ok(s) = res else {
            self.have_status.set(false);
            return self.show_intro(u);
        };
        self.have_status.set(true);
        let cleaning = u.cleaning();
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
                self.open(&self.nozzle);
            }
            // Dialog 220 with status, 226 without: same layout and text.
            Utility::HeadCleaning => {
                self.cleaning.set_body(CLEAN_INTRO.into());
                self.cleaning.set_power(false);
                self.open(&self.cleaning);
            }
            Utility::PaperGuide => {
                self.paper_guide.set_body(PAPER_GUIDE.into());
                self.open(&self.paper_guide);
            }
        }
    }

    /// Dialog 265 [Cleaning]: the same worker as the other utilities (status
    /// re-check, paused/offline check), but on the L800 Windows sends no
    /// bytes: its script RCDATA 1900 does not exist in E_IHBRG5P, so
    /// FUN_00407dc4 opens and closes an empty RAW job. We send nothing.
    /// Then the utility ends (message 0x41E closes 265).
    fn paper_guide_cleaning(&self) {
        self.close(&self.paper_guide);
        self.run_job(Utility::PaperGuide, None, |app, res| match res {
            // Live: the utility just ends. Dry run: say so, with any queue note.
            Ok(sent) if app.dry_run() => app.show_message(Utility::PaperGuide, "Dry run", &app.with_note(&sent.summary), false),
            Ok(_) => {}
            Err(f) => app.show_fail(Utility::PaperGuide, f),
        });
    }

    fn nozzle_print(&self) {
        self.close(&self.nozzle);
        self.run_job(Utility::NozzleCheck, Some(("nozzle-check", commands::nozzle_check_job(Timestamp::now()))), |app, res| {
            match res {
                Ok(sent) => {
                    app.show_dump(&sent);
                    app.nozzle_result.set_body(app.with_note(NOZZLE_RESULT).into());
                    app.open(&app.nozzle_result);
                }
                Err(f) => app.show_fail(Utility::NozzleCheck, f),
            }
        });
    }

    fn cleaning_start(&self, power: bool) {
        self.close(&self.cleaning);
        if power {
            // FUN_004118dc shows dialog 360 when the selected value is not 0
            // and E_IHBRG5P string 5060 exists (it does on the L800).
            self.power.set_body(POWER_CONFIRM.into());
            self.open(&self.power);
        } else {
            self.run_cleaning(Cleaning::Normal);
        }
    }

    fn run_cleaning(&self, kind: Cleaning) {
        let name = match kind {
            Cleaning::Normal => "head-cleaning",
            Cleaning::Power => "power-cleaning",
        };
        self.run_job(Utility::HeadCleaning, Some((name, commands::head_cleaning_job(kind, Timestamp::now()))), |app, res| {
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
    fn run_job(&self, u: Utility, job: Option<(&'static str, Vec<u8>)>, then: JobDone) {
        self.show_wait(u);
        let (mode, p, have_status) = (self.mode.clone(), self.printer.clone(), self.have_status.get());
        let queue = p.queue.clone();
        bg(
            move || {
                let res = at_least_min_wait(|| {
                    if have_status && matches!(query(&p), Ok(Status { st: Some(0x00), .. })) {
                        return Err(Fail::Problem);
                    }
                    let problem = transport::queue_problem(&queue);
                    if let (Some(p), Mode::Live) = (&problem, &mode) {
                        return Err(Fail::Plain(format!("{COMM_ERROR}\n\n{p}")));
                    }
                    let mut sent = match job {
                        None => Sent { summary: "Paper Guide Cleaning sends no data on the L800 (Windows sends none either).".into(), dump: None },
                        Some((name, bytes)) => {
                            // No status session may be open while CUPS gets the job.
                            let _device = status::exclusive();
                            transport::submit(&mode, &queue, name, &bytes).map_err(Fail::Plain)?
                        }
                    };
                    if let Some(p) = problem {
                        sent.summary.push_str(&format!("\nA real run would stop here with a communication error: {p}"));
                    }
                    Ok(sent)
                });
                (then, res)
            },
            |app, (then, v)| {
                app.close(&app.wait);
                then(app, v)
            },
        );
    }

    // Progress dialog 223 (FUN_004123b8, FUN_00411e5c, FUN_00412ae8).

    fn start_progress(&self) {
        *self.progress_state.borrow_mut() = Progress::default();
        self.bar_timer.stop();
        self.progress.set_body("Please wait...".into());
        self.progress.set_bar_visible(false);
        self.progress.set_value(0.0);
        self.progress.set_frame(0);
        self.progress.set_animate(true);
        self.open(&self.progress);
        self.poll_timer.start(TimerMode::Repeated, status::POLL, on(|a| a.poll_cleaning()));
    }

    fn poll_cleaning(&self) {
        if std::mem::replace(&mut self.progress_state.borrow_mut().in_flight, true) {
            return;
        }
        let p = self.printer.clone();
        bg(move || query(&p), |app, res| app.on_cleaning_poll(res));
    }

    fn on_cleaning_poll(&self, res: QueryResult) {
        let step = {
            let mut p = self.progress_state.borrow_mut();
            p.in_flight = false;
            p.monitor.on_poll(&res)
        };
        match step {
            Step::Waiting | Step::Running => {}
            Step::Started(secs) => {
                let at = Instant::now();
                self.progress.set_body(CLEANING_NOW.into());
                self.progress.set_bar_visible(true);
                self.bar_timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
                    if let Some(app) = app() {
                        app.progress.set_value(status::progress(at.elapsed(), secs));
                    }
                });
            }
            Step::Done => {
                self.poll_timer.stop();
                self.bar_timer.stop();
                self.progress.set_value(1.0);
                Timer::single_shot(
                    Duration::from_millis(500),
                    on(|a| {
                        a.progress.set_animate(false);
                        a.close(&a.progress);
                        a.show_done(true);
                    }),
                );
            }
            Step::GaveUp => {
                self.poll_timer.stop();
                self.bar_timer.stop();
                self.progress.set_animate(false);
                self.close(&self.progress);
                self.show_done(false);
            }
        }
    }

    /// Dialog 224 (status: string 4204) or 227 (no status: 4205).
    fn show_done(&self, finished_seen: bool) {
        let text = if finished_seen { CLEAN_DONE_STATUS } else { CLEAN_DONE };
        self.cleaning_done.set_body(self.with_note(text).into());
        self.open(&self.cleaning_done);
    }

    fn show_fail(&self, u: Utility, f: Fail) {
        match f {
            Fail::Problem => self.show_message(u, COMM_ERROR, SEE_DETAILS, true),
            // Headings: E_IHSRG5P 1128 / 1129.
            Fail::Plain(detail) => self.show_message(
                u,
                if u.cleaning() { "Head Cleaning can not be completed." } else { "Nozzle Check can not be completed." },
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
        self.open(&self.message);
    }

    // Status Monitor 3 window.

    fn open_monitor(&self) {
        self.monitor.set_printer(self.printer.queue.clone().into());
        self.monitor.set_title_text("Please wait...".into());
        self.monitor.set_body("".into());
        self.open(&self.monitor);
        self.poll_monitor();
        self.monitor_timer.start(TimerMode::Repeated, MONITOR_POLL, on(|a| a.poll_monitor()));
    }

    fn poll_monitor(&self) {
        if self.monitor_in_flight.replace(true) {
            return;
        }
        let p = self.printer.clone();
        bg(move || query(&p), |app, res| {
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
                self.monitor.set_inks(slint::VecModel::from_slice(&ink_levels(&s)));
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
            self.open(&self.dump);
        }
    }

    fn show_printer_info(&self) {
        self.info.set_caption("Printer and Option Information".into());
        self.info.set_heading(
            format!("The printer is used through the CUPS queue \"{}\". This is how the app identified it, and what CUPS reports for it (read-only).", self.printer.queue).into(),
        );
        // lpstat can be slow when cupsd is: keep it off the UI thread.
        let queue = self.printer.queue.clone();
        let summary = self.printer.summary();
        bg(move || format!("{summary}\n{}", transport::queue_info(&queue)), |app, text| {
            app.info.set_content(text.into());
            app.open(&app.info);
        });
    }
}

/// The ink meters for one status reply, in printer order.
fn ink_levels(s: &Status) -> Vec<InkLevel> {
    s.inks
        .iter()
        .map(|i| InkLevel {
            name: status::ink_name(i.color).into(),
            color: ink_color(i.color),
            level: i.percent().map_or(-1, i32::from),
        })
        .collect()
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

/// A Slint string list.
fn strings<'a>(items: impl IntoIterator<Item = &'a str>) -> slint::ModelRc<slint::SharedString> {
    slint::VecModel::from_slice(&items.into_iter().map(slint::SharedString::from).collect::<Vec<_>>())
}

/// The Windows labels of each list (settings.rs tables).
fn fill_lists(p: &Prefs) {
    let flags = |list: &[settings::Choice]| -> slint::ModelRc<bool> {
        slint::VecModel::from_slice(&list.iter().map(|c| c.borderless).collect::<Vec<_>>())
    };
    let labels = |list: &[settings::Choice]| strings(list.iter().map(|c| c.label));
    p.set_quality_labels(labels(settings::QUALITY));
    p.set_source_labels(labels(settings::SOURCES));
    p.set_color_mode_labels(labels(settings::COLOR_MODES));
    p.set_gamma_labels(labels(settings::GAMMAS));
    p.set_preset_labels(strings(settings::PRESETS.iter().map(|x| x.name)));
    p.set_source_borderless(flags(settings::SOURCES));
    p.set_media_borderless(flags(settings::MEDIA));
    p.set_size_borderless(flags(settings::SIZES));
    p.set_first_document_size(settings::FIRST_DOCUMENT_SIZE as i32);
}

struct Args {
    /// --queue; otherwise the only Epson queue, or the user picks one.
    queue: Option<String>,
    dump_dir: Option<PathBuf>,
    /// Open on the Maintenance tab (Windows: launch code EPUT).
    utility: bool,
}

const USAGE: &str = "Usage: epson-l800-utility [--utility] [--dry-run] [--dump-dir DIR] [--queue NAME]

  --utility       open on the Maintenance tab (default: Main)
  --dry-run       write each job to a file and show a hexdump; never calls lp,
                  and show the lpoptions command instead of saving settings
  --dump-dir DIR  folder for dry-run files (default: ~/.cache/epson-l800-utility)
  --queue NAME    CUPS queue of the printer (default: the only Epson queue;
                  with several, a list to choose from)
";

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut queue = None;
    let mut dry = false;
    let mut dir = None;
    let mut utility = false;
    let mut it = args;
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dry-run" => dry = true,
            "--utility" => utility = true,
            "--dump-dir" => dir = Some(PathBuf::from(it.next().ok_or("--dump-dir needs a value")?)),
            "--queue" => queue = Some(it.next().ok_or("--queue needs a value")?),
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
    Ok(Args { queue, dump_dir: dry.then(|| dir.unwrap_or_else(transport::default_dump_dir)), utility })
}

fn main() -> Result<(), slint::PlatformError> {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    // The Wayland app_id (X11 WM_CLASS) of every window: GNOME matches it to
    // epson-l800-utility.desktop (StartupWMClass), so the sheet and its
    // dialogs group as one app with our icon. Must be set before any window
    // is shown (the winit windows are created on show).
    slint::set_xdg_app_id(APP_ID)?;
    let queue = match args.queue {
        Some(q) => q,
        None => match choose_queue()? {
            Some(q) => q,
            None => return Ok(()),
        },
    };
    let app = App::new(args.dump_dir.map_or(Mode::Live, Mode::DryRun), Identity::of(&queue))?;
    APP.with(|a| *a.borrow_mut() = Rc::downgrade(&app));
    app.main.set_dry_run(app.dry_run());
    app.main.set_current_tab(if args.utility { 4 } else { 1 });
    fill_lists(&app.main.global::<Prefs>());
    app.set_prefs(&Values::default());
    // The sheet waits for the load, so no early edit (or OK) is lost to it.
    app.settings_io.set(true);
    app.refresh_busy();
    let queue = app.printer.queue.clone();
    bg(move || settings::load(&queue), |app, res| {
        app.settings_io.set(false);
        app.refresh_busy();
        match res {
            Ok(v) => app.set_prefs(&v),
            Err(e) => eprintln!("epson-l800-utility: cannot read the printing preferences: {e}"),
        }
    });
    // The Ink Levels boxes on Main and Shortcuts: one status query when the
    // sheet opens (same rules as always: none while CUPS has jobs, Exit is
    // always sent). Without a reply they stay empty, like Windows.
    let p = app.printer.clone();
    bg(move || query(&p), |app, res| {
        if let Ok(s) = res {
            app.main.global::<Prefs>().set_inks(slint::VecModel::from_slice(&ink_levels(&s)));
        }
    });
    wire(&app);

    frame(&app.main);
    app.main.show()?;
    let res = slint::run_event_loop();
    // Let an in-flight status query finish Close/Exit, and keep new ones out
    // until the process is gone (the lock is never released): never leave
    // the printer in 1284.4 mode.
    std::mem::forget(status::exclusive());
    res
}

/// The queue to open when no --queue was given: the only Epson queue, or
/// the one picked in the chooser (None: cancelled). The chooser runs its own
/// event loop before the sheet's.
fn choose_queue() -> Result<Option<String>, slint::PlatformError> {
    let queues = printer::epson_queues();
    if let [(q, _)] = queues.as_slice() {
        return Ok(Some(q.name.clone()));
    }
    let d = ChooserDialog::new()?;
    frame(&d);
    let labels: Vec<String> = queues
        .iter()
        .map(|(q, mm)| if mm.is_empty() { q.name.clone() } else { format!("{}  ({mm})", q.name) })
        .collect();
    d.set_queues(strings(labels.iter().map(String::as_str)));
    let picked = Rc::new(Cell::new(None));
    let (w, chosen) = (d.as_weak(), picked.clone());
    d.on_ok(move || {
        if let Some(d) = w.upgrade() {
            chosen.set(usize::try_from(d.get_selected()).ok());
            let _ = d.hide();
        }
        let _ = slint::quit_event_loop();
    });
    d.on_cancel(|| {
        let _ = slint::quit_event_loop();
    });
    d.window().on_close_requested(|| {
        let _ = slint::quit_event_loop();
        slint::CloseRequestResponse::HideWindow
    });
    d.show()?;
    slint::run_event_loop()?;
    let _ = d.hide();
    Ok(picked.get().and_then(|i| queues.get(i)).map(|(q, _)| q.name.clone()))
}

/// Connect every callback. Handlers reach the app through `app()`.
fn wire(ui: &App) {
    ui.main.on_utility_clicked(|id| {
        let Some(a) = app() else { return };
        match id.as_str() {
            "status-monitor" => a.open_monitor(),
            "nozzle-check" => a.start(Utility::NozzleCheck),
            "head-cleaning" => a.start(Utility::HeadCleaning),
            "paper-guide-cleaning" => a.start(Utility::PaperGuide),
            "printer-info" => a.show_printer_info(),
            _ => {}
        }
    });
    // Windows runs Epson's own updater (E_GUPA30.EXE); on Linux we open
    // Epson's driver download page for the L800 instead.
    ui.main.on_driver_update(|| launch("xdg-open", DRIVER_PAGE));
    // OK = save + close, Apply = save, Cancel (or the close box) = discard.
    ui.main.on_ok(on(|a| a.save_prefs(true)));
    ui.main.on_apply(on(|a| a.save_prefs(false)));
    ui.main.on_cancel(|| {
        let _ = slint::quit_event_loop();
    });
    ui.main.window().on_close_requested(|| {
        let _ = slint::quit_event_loop();
        slint::CloseRequestResponse::HideWindow
    });
    let prefs = ui.main.global::<Prefs>();
    // After every edit: re-apply the constraints and refill the lists.
    prefs.on_edited(on(|a| a.set_prefs(&a.prefs())));
    prefs.on_pick(|list, row| {
        let Some(a) = app() else { return };
        let mut v = a.prefs();
        let row = usize::try_from(row).unwrap_or(0);
        match list.as_str() {
            "media" => v.media = v.media_list().get(row).copied().unwrap_or(v.media),
            "size" => v.size = v.size_list().get(row).copied().unwrap_or(v.size),
            "output" => v.output_size = v.output_list().get(row).copied().unwrap_or(v.output_size),
            "quality" => v.quality = settings::advanced_qualities(v.media).get(row).map_or(v.quality, |q| q.1),
            _ => return,
        }
        let p = a.main.global::<Prefs>();
        a.set_prefs(&v);
        p.set_dirty(true);
        p.set_preset(-1);
    });
    prefs.on_open_dialog(|id| {
        if let Some(a) = app() {
            a.open_dialog(&id);
        }
    });
    // Show Settings / Hide Settings toggles the Current Settings window.
    prefs.on_toggle_settings(on(|a| a.show_settings(!a.main.global::<Prefs>().get_settings_shown())));
    prefs.on_reset_defaults(on(|a| {
        a.set_prefs(&Values::default());
        let p = a.main.global::<Prefs>();
        p.set_dirty(true);
        p.set_preset(-1);
    }));
    prefs.on_preset_picked(|i| {
        let (Some(a), Some(preset)) = (app(), usize::try_from(i).ok().and_then(|i| settings::PRESETS.get(i))) else { return };
        let mut v = a.prefs();
        preset.apply(&mut v);
        a.set_prefs(&v);
        let p = a.main.global::<Prefs>();
        p.set_dirty(true);
        p.set_preset(i);
    });
}

/// One maker per dialog: create the window and connect its callbacks
/// (handlers reach the app through `app()`). Used by `Lazy`.
mod dialogs {
    use super::*;

    pub fn wait() -> WaitDialog {
        let d = make(WaitDialog::new);
        // Dialog 201 has no way out on Windows; it closes itself.
        d.window().on_close_requested(|| slint::CloseRequestResponse::KeepWindowShown);
        d
    }

    pub fn nozzle() -> NozzleCheckDialog {
        let d = make(NozzleCheckDialog::new);
        d.on_print(on(|a| a.nozzle_print()));
        d.on_cancel(on(|a| a.close(&a.nozzle)));
        d
    }

    pub fn nozzle_result() -> NozzleResultDialog {
        let d = make(NozzleResultDialog::new);
        // [Clean] goes to the Head Cleaning utility (state 2 in FUN_00413b60).
        d.on_clean(on(|a| {
            a.close(&a.nozzle_result);
            a.start(Utility::HeadCleaning);
        }));
        d.on_finish(on(|a| a.close(&a.nozzle_result)));
        d
    }

    pub fn cleaning() -> HeadCleaningDialog {
        let d = make(HeadCleaningDialog::new);
        d.on_start(|power| {
            if let Some(a) = app() {
                a.cleaning_start(power)
            }
        });
        d.on_cancel(on(|a| a.close(&a.cleaning)));
        d
    }

    pub fn power() -> PowerCleaningDialog {
        let d = make(PowerCleaningDialog::new);
        d.on_ok(on(|a| {
            a.close(&a.power);
            a.run_cleaning(Cleaning::Power);
        }));
        // [Cancel] returns to the Head Cleaning dialog.
        d.on_cancel(on(|a| {
            a.close(&a.power);
            a.open(&a.cleaning);
        }));
        d
    }

    pub fn progress() -> ProgressDialog {
        let d = make(ProgressDialog::new);
        // Dialog 223 has no way out on Windows; it closes itself.
        d.window().on_close_requested(|| slint::CloseRequestResponse::KeepWindowShown);
        d
    }

    pub fn cleaning_done() -> CleaningDoneDialog {
        let d = make(CleaningDoneDialog::new);
        // [Print Nozzle Check Pattern] starts the Nozzle Check (state 1).
        d.on_nozzle_check(on(|a| {
            a.close(&a.cleaning_done);
            a.start(Utility::NozzleCheck);
        }));
        d.on_finish(on(|a| a.close(&a.cleaning_done)));
        d
    }

    pub fn paper_guide() -> PaperGuideDialog {
        let d = make(PaperGuideDialog::new);
        d.on_cleaning(on(|a| a.paper_guide_cleaning()));
        d.on_cancel(on(|a| a.close(&a.paper_guide)));
        d
    }

    pub fn message() -> MessageDialog {
        let d = make(MessageDialog::new);
        d.on_ok(on(|a| a.close(&a.message)));
        d.on_details(on(|a| {
            a.close(&a.message);
            a.open_monitor();
        }));
        d
    }

    pub fn monitor() -> StatusMonitorDialog {
        let d = make(StatusMonitorDialog::new);
        // "Print Queue": the GNOME printer settings show the CUPS queue.
        d.on_print_queue(|| launch("gnome-control-center", "printers"));
        d.on_ok(on(|a| {
            a.monitor_timer.stop();
            a.close(&a.monitor);
        }));
        d.window().on_close_requested(|| {
            if let Some(a) = app() {
                a.monitor_timer.stop();
            }
            Timer::single_shot(Duration::ZERO, on(|a| a.refresh_busy()));
            slint::CloseRequestResponse::HideWindow
        });
        d
    }

    pub fn info() -> TextDialog {
        let d = make(TextDialog::new);
        d.on_ok(on(|a| a.close(&a.info)));
        d
    }

    pub fn dump() -> TextDialog {
        let d = make(TextDialog::new);
        d.set_monospace(true);
        d.on_ok(on(|a| a.close(&a.dump)));
        d
    }

    pub fn current_settings() -> CurrentSettingsDialog {
        let d = make(CurrentSettingsDialog::new);
        d.on_dismiss(on(|a| a.show_settings(false)));
        d.window().on_close_requested(|| {
            if let Some(a) = app() {
                a.show_settings(false);
            }
            slint::CloseRequestResponse::HideWindow
        });
        d
    }

    pub fn color_controls() -> ColorControlsDialog {
        let d = make(ColorControlsDialog::new);
        // The same fixed lists as the sheet's (fill_lists).
        d.set_color_mode_labels(strings(settings::COLOR_MODES.iter().map(|c| c.label)));
        d.set_gamma_labels(strings(settings::GAMMAS.iter().map(|c| c.label)));
        d.on_ok(on(|a| a.color_controls_ok()));
        d.on_cancel(on(|a| a.close(&a.color_controls)));
        d
    }

    pub fn layout_dialog() -> LayoutDialog {
        let d = make(LayoutDialog::new);
        d.on_dismiss(on(|a| a.close(&a.layout_dialog)));
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    /// No display: windows render into memory (and are never shown here).
    struct Headless;
    impl slint::platform::Platform for Headless {
        fn create_window_adapter(&self) -> Result<Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
            use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
            Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
        }
    }

    /// What a reload does with the values lpoptions reports: the sheet
    /// shows them (constrained, with the matching row in each list) and
    /// reads back the same values.
    #[test]
    fn reload_restores_the_sheet() {
        let _ = slint::platform::set_platform(Box::new(Headless));
        let app = App::new(Mode::DryRun(std::env::temp_dir()), Identity { queue: "L800".into(), ..Default::default() }).unwrap();
        let p = app.main.global::<Prefs>();
        fill_lists(&p);
        let d = Values::default;
        for v in [
            Values { media: 2, borderless: true, quality: 4, ..d() },
            Values { color_mode: 0, gamma: 0, brightness: 10, contrast: -5, cyan: -25, yellow: 12, ..d() },
            Values { grayscale: true, landscape: true, reverse: true, ..d() },
            Values { reduce: true, by_percent: true, scale: 150, ..d() },
            Values { multi_page: true, poster: true, poster_index: 2, copies: 3, collate: true, ..d() },
            // Not allowed together; a file edited elsewhere can still hold it.
            Values { source: 1, size: 14, output_size: 5, borderless: true, ..d() },
        ] {
            app.set_prefs(&v);
            let want = v.constrained();
            assert_eq!(app.prefs(), want);
            let shown = |labels: slint::ModelRc<slint::SharedString>, pos: i32| labels.row_data(pos as usize).unwrap().to_string();
            assert_eq!(shown(p.get_media_labels(), p.get_media_pos()), settings::MEDIA[want.media].label);
            assert_eq!(shown(p.get_size_labels(), p.get_size_pos()), settings::SIZES[want.size].label);
            assert_eq!(shown(p.get_output_labels(), p.get_output_pos()), settings::output_sizes()[want.output_size].label);
        }
    }

    /// Copies typed out of range are saved clamped to 1..9999, and the
    /// commit tick moves so the edit shows that value.
    #[test]
    fn copies_are_clamped_and_committed() {
        let _ = slint::platform::set_platform(Box::new(Headless));
        let app = App::new(Mode::DryRun(std::env::temp_dir()), Identity { queue: "L800".into(), ..Default::default() }).unwrap();
        let p = app.main.global::<Prefs>();
        fill_lists(&p);
        p.set_copies(0);
        assert_eq!(app.prefs().copies, 1);
        p.set_copies(12000);
        assert_eq!(app.prefs().copies, 9999);
        p.set_dirty(true);
        let tick = p.get_commit_tick();
        app.save_prefs(false); // dry run: shows the command, saves nothing
        assert_eq!(p.get_commit_tick(), tick + 1);
        assert!(!p.get_dirty());
    }

    fn args(v: &[&str]) -> Result<Args, String> {
        parse_args(v.iter().map(|s| s.to_string()))
    }

    #[test]
    fn default_is_live_with_no_queue() {
        let a = args(&[]).unwrap();
        assert_eq!(a.queue, None);
        assert!(a.dump_dir.is_none());
    }

    #[test]
    fn dry_run_flags() {
        let a = args(&["--dry-run", "--dump-dir", "/tmp/x", "--queue", "Q"]).unwrap();
        assert_eq!(a.dump_dir.unwrap(), PathBuf::from("/tmp/x"));
        assert_eq!(a.queue.as_deref(), Some("Q"));
        assert!(args(&["--dump-dir", "/tmp/x"]).is_err());
        assert!(args(&["--bogus"]).is_err());
    }
}

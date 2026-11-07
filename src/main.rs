//! escp-utility: Printing Preferences and maintenance for Epson inkjet
//! printers on Linux, laid out like the Windows Epson L800 driver.
//!
//! The flows follow the Windows utility (E_IHUTG5P.DLL). With a status reply
//! it takes the status path (pre-check, dialog 220, progress 223, finish 224).
//! Without one it takes the no-status path (226, then 227). See README.md.

mod commands;
mod layouts;
mod ppd;
mod ppd_sheet;
mod printer;
mod profile;
mod report;
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

use commands::Timestamp;
use printer::Identity;
use profile::Profile;
use ppd_sheet::strings;
use settings::Values;
use status::{CleaningMonitor, Precheck, QueryError, Status, Step};
use texts::*;
use transport::{Mode, Sent};

/// Epson's Linux driver download page for the printer's model (Windows runs
/// its own updater, E_GUPA30.EXE): device_id is the IEEE 1284 MDL without a
/// trailing " Series" (the L800 page is `device_id=L800`).
fn driver_page(mdl: Option<&str>) -> String {
    let device = mdl.map(|m| m.strip_suffix(" Series").unwrap_or(m).replace(' ', "%20"));
    let device = device.map_or(String::new(), |d| format!("device_id={d}&"));
    format!("https://download-center.epson.com/softwares/?{device}region=ID&os=DEBX64&language=en")
}

/// The L800 PPD keeps the Windows tables (settings.rs); any other PPD
/// drives the same pages through ppd_sheet.rs.
const L800_NICKNAME: &str = "Epson L800 Series - epson-inkjet-printer";

/// Wayland app_id / X11 WM_CLASS; matches the desktop file and icon name.
const APP_ID: &str = "escp-utility";

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
        Err(e) => eprintln!("escp-utility: cannot start {program}: {e}"),
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
                eprintln!("escp-utility: cannot close the window: {e}");
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

/// The pages of a printer other than the L800 (ppd_sheet.rs).
struct PpdState {
    sheet: ppd_sheet::Sheet,
    /// Complete and conflict-free.
    sel: RefCell<ppd::Selection>,
    /// What the pages show for `sel`.
    view: RefCell<ppd_sheet::View>,
    /// The selection when Printer Options opened: its Cancel restores it.
    before: RefCell<Option<(ppd::Selection, bool)>>,
}

struct App {
    mode: Mode,
    /// The queue and the printer behind it.
    printer: Identity,
    /// Its profile, or the generic one.
    profile: Profile,
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
    printer_options: Lazy<PrinterOptionsDialog>,
    /// Any printer but the L800: the PPD sheet (None: the L800 tables).
    ppd: Option<PpdState>,
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
    /// `profile`: the one matching the printer's MDL. Without one, the
    /// maintenance jobs need an ESC/P device ID, and use the generic profile.
    fn new(mode: Mode, printer: Identity, profile: Option<Profile>, sheet: Option<ppd_sheet::Sheet>) -> Result<Rc<App>, slint::PlatformError> {
        let main = MainWindow::new()?;
        main.set_queue(printer.queue.clone().into());
        main.set_maintenance(printer.id.maintenance() || profile.is_some());
        let profile = profile.unwrap_or_else(Profile::generic);
        main.set_experimental(profile.experimental());
        // Paper Guide Cleaning is the L800's (a no-op there, as on Windows).
        main.set_paper_guide(profile.l800_windows());
        main.set_version_text(env!("CARGO_PKG_VERSION").into());
        Ok(Rc::new(App {
            mode,
            printer,
            profile,
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
            printer_options: Lazy::new(dialogs::printer_options),
            ppd: sheet.map(|sheet| PpdState {
                sheet,
                sel: RefCell::default(),
                view: RefCell::default(),
                before: RefCell::default(),
            }),
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
            shown(&self.printer_options),
        ];
        self.main.set_busy(self.settings_io.get() || open.contains(&true));
    }

    /// An edit on the sheet: Apply is enabled and no preset is selected.
    fn touched(&self) {
        let p = self.main.global::<Prefs>();
        p.set_dirty(true);
        p.set_preset(-1);
    }

    /// The defaults on the sheet (PPD defaults, or the L800's).
    fn show_defaults(&self) {
        match &self.ppd {
            Some(st) => {
                self.ppd_set(st, st.sheet.defaults());
                ppd_sheet::show_std(&self.main.global::<Prefs>(), &ppd_sheet::Std::default());
            }
            None => self.set_prefs(&Values::default()),
        }
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

    /// Show a PPD selection on the pages.
    fn ppd_set(&self, st: &PpdState, sel: ppd::Selection) {
        let v = st.sheet.view(&sel);
        ppd_sheet::show(&self.main.global::<Prefs>(), &v);
        *st.sel.borrow_mut() = sel;
        *st.view.borrow_mut() = v;
        self.refresh_current_settings();
    }

    /// Apply an edit on the pages of the PPD sheet.
    fn ppd_edit(&self, st: &PpdState, e: &ppd_sheet::Edit) {
        let sel = st.sheet.apply(&st.sel.borrow(), e);
        self.ppd_set(st, sel);
    }

    /// The PPD sheet's sub-dialogs: Color Controls and Printer Options.
    fn open_ppd_dialog(&self, st: &PpdState, id: &str) {
        let v = st.view.borrow();
        match id {
            "color-controls" => {
                let d = &self.color_controls;
                d.set_ppd_mode(true);
                d.set_color_mode_labels(strings(v.color_mode.labels()));
                d.set_color_mode(v.color_mode.pos as i32);
                d.set_gamma_labels(strings(v.gamma.labels()));
                d.set_gamma(v.gamma.pos as i32);
                d.set_has_color_mode(v.color_mode.present);
                d.set_has_gamma(v.gamma.present);
                d.set_has_level(slint::VecModel::from_slice(&v.levels.map(|l| l.is_some())));
                let l = v.levels.map(|l| l.unwrap_or(0));
                d.set_brightness(l[0]);
                d.set_contrast(l[1]);
                d.set_saturation(l[2]);
                d.set_cyan(l[3]);
                d.set_magenta(l[4]);
                d.set_yellow(l[5]);
                self.open(d);
            }
            "printer-options" => {
                // With Apply's state, which Cancel restores too.
                *st.before.borrow_mut() = Some((st.sel.borrow().clone(), self.main.global::<Prefs>().get_dirty()));
                self.printer_options.set_rows(slint::VecModel::from_slice(&ppd_sheet::option_rows(&v)));
                self.open(&self.printer_options);
            }
            _ => {}
        }
    }

    /// Printer Options [Cancel] or close box: undo its picks, and Apply's
    /// state with them.
    fn printer_options_cancel(&self) {
        if let Some(st) = &self.ppd
            && let Some((sel, dirty)) = st.before.borrow_mut().take()
        {
            self.ppd_set(st, sel);
            self.main.global::<Prefs>().set_dirty(dirty);
        }
        self.close(&self.printer_options);
    }

    /// The sub-dialogs of the settings pages.
    fn open_dialog(&self, id: &str) {
        if let Some(st) = &self.ppd
            && matches!(id, "color-controls" | "printer-options")
        {
            return self.open_ppd_dialog(st, id);
        }
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
        if let Some(st) = &self.ppd {
            let row = |i: i32| usize::try_from(i).unwrap_or(0);
            let levels = [d.get_brightness(), d.get_contrast(), d.get_saturation(), d.get_cyan(), d.get_magenta(), d.get_yellow()];
            let edits = st.sheet.color_edits(&st.view.borrow(), row(d.get_color_mode()), row(d.get_gamma()), levels);
            for e in &edits {
                self.ppd_edit(st, e);
            }
            self.main.global::<Prefs>().set_dirty(true);
            return self.close(d);
        }
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
        self.touched();
        self.close(d);
    }

    /// The Current Settings window (dialog 4050): labels are strings 4053-4079.
    fn refresh_current_settings(&self) {
        use settings::*;
        let Some(dialog) = Lazy::get(&self.current_settings) else { return };
        let on_off = |b: bool| if b { "On" } else { "Off" }.to_string();
        if let Some(st) = &self.ppd {
            let (v, p) = (st.view.borrow(), self.main.global::<Prefs>());
            let at = |f: &ppd_sheet::Field| f.items.get(f.pos).map_or("-".to_string(), |(_, l)| l.clone());
            let s = ppd_sheet::std_of(&p);
            let lines = [
                ("Paper Source", at(&v.source)),
                ("Paper Type", at(&v.media)),
                ("Quality", v.quality.map_or("-".to_string(), |r| v.quality_labels[r].clone())),
                ("Document Size", at(&v.size)),
                ("Borderless", on_off(v.borderless)),
                ("Orientation", if s.landscape { "Landscape" } else { "Portrait" }.to_string()),
                ("Reverse Order", on_off(s.reverse)),
                ("Grayscale", on_off(v.grayscale == Some(true))),
                ("Color Mode", at(&v.color_mode)),
                ("Page Layout", if s.multi_page { format!("Pages per Sheet {}", PAGES_PER_SHEET[s.pages_index]) } else { "Off".into() }),
            ];
            let lines: Vec<SettingLine> =
                lines.into_iter().map(|(label, value)| SettingLine { label: label.into(), value: value.into() }).collect();
            return dialog.set_lines(slint::VecModel::from_slice(&lines));
        }
        let v = self.prefs();
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
        let queue = &self.printer.queue;
        let args = match &self.ppd {
            Some(st) => settings::save_args(queue, st.sheet.to_options(&st.sel.borrow()).into_iter().chain(ppd_sheet::std_of(&p).to_options())),
            None => settings::save_args(queue, self.prefs().to_options()),
        };
        if self.dry_run() {
            self.dump.set_caption("Dry run".into());
            self.dump.set_heading("Dry run: these settings were not saved. A real run would run this command:".into());
            self.dump.set_content(format!("lpoptions {}\n", args.join(" \\\n    ")).into());
            self.open(&self.dump);
            p.set_dirty(false);
            return;
        }
        self.settings_io.set(true);
        self.refresh_busy();
        bg(move || (quit, settings::save(&args)), |app, (quit, res)| {
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

    /// The Windows L800 text, or the neutral one for other printers.
    fn text(&self, l800: &'static str, generic: &'static str) -> &'static str {
        if self.profile.l800_windows() { l800 } else { generic }
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
        let check = if self.profile.l800_windows() { status::precheck(&s, cleaning) } else { status::precheck_generic(&s, cleaning) };
        match check {
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
            Precheck::InkOut => self.show_message(
                u,
                if cleaning { "Cannot complete Head Cleaning." } else { "Cannot complete Nozzle Check." },
                INK_OUT,
                false,
            ),
        }
    }

    fn show_intro(&self, u: Utility) {
        match u {
            Utility::NozzleCheck => {
                self.nozzle.set_body(self.text(NOZZLE_INTRO, NOZZLE_INTRO_GENERIC).into());
                self.open(&self.nozzle);
            }
            // Dialog 220 with status, 226 without: same layout and text.
            Utility::HeadCleaning => {
                self.cleaning.set_body(self.text(CLEAN_INTRO, CLEAN_INTRO_GENERIC).into());
                self.cleaning.set_kinds(strings(self.profile.cleaning.iter().map(|c| c.label.as_str())));
                self.cleaning.set_kind(0);
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
        let job = commands::nozzle_check_job(self.profile.nc_param, Timestamp::now());
        self.run_job(Utility::NozzleCheck, Some(("nozzle-check".into(), job)), |app, res| {
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

    /// The cleaning type picked in dialog 220/226 (a row of the profile's list).
    fn cleaning_kind(&self) -> &profile::Cleaning {
        let i = usize::try_from(self.cleaning.get_kind()).unwrap_or(0);
        self.profile.cleaning.get(i).unwrap_or(&self.profile.cleaning[0])
    }

    fn cleaning_start(&self) {
        self.close(&self.cleaning);
        if self.cleaning_kind().confirms() {
            // FUN_004118dc shows dialog 360 when the selected value is not 0
            // and E_IHBRG5P string 5060 exists (it does on the L800).
            self.power.set_body(self.text(POWER_CONFIRM, CONFIRM_GENERIC).into());
            // The transportation lock is L800 hardware.
            self.power.set_picture(self.profile.l800_windows());
            self.open(&self.power);
        } else {
            self.run_cleaning();
        }
    }

    /// Spool the cleaning type picked in dialog 220/226.
    fn run_cleaning(&self) {
        let ch = self.cleaning_kind().ch;
        let job = commands::head_cleaning_job(ch, Timestamp::now());
        self.run_job(Utility::HeadCleaning, Some((format!("head-cleaning-{ch:02x}"), job)), |app, res| {
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
    fn run_job(&self, u: Utility, job: Option<(String, Vec<u8>)>, then: JobDone) {
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
                            transport::submit(&mode, &queue, &name, &bytes).map_err(Fail::Plain)?
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
                self.progress.set_body(self.text(CLEANING_NOW, CLEANING_NOW_GENERIC).into());
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
        self.monitor.set_note(self.text(status::INK_NOTE, status::GENERIC_INK_NOTE).into());
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
                let (title, text) = if self.profile.l800_windows() {
                    let (title, text) = status::message(&s);
                    (title.to_string(), text)
                } else {
                    status::message_generic(&s)
                };
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
        let summary = format!("{}{}", self.printer.summary(), self.profile.summary());
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
            name: status::ink_name(i).into(),
            color: ink_color(i.color),
            level: i.percent().map_or(-1, i32::from),
        })
        .collect()
}

/// Bar colour for each ink (colour code from field 0x0F, see `status::ink_name`).
fn ink_color(code: u8) -> slint::Color {
    let rgb = match code {
        0x00 => 0x202020,
        0x01 => 0x00a0e0,
        0x02 => 0xd0168c,
        0x03 => 0xf0c800,
        0x04 => 0x7fd0f0,
        0x05 => 0xf08cc8,
        0x06 => 0xfff0a0,
        0x07 => 0xc8a000,
        0x08 => 0x707070,
        0x09 => 0xe02020,
        0x0A => 0x7030c0,
        0x0B => 0xe8e8e8,
        0x0C => 0xa8a8a8,
        0x0D => 0xf08020,
        0x0E => 0x20a040,
        0x0F => 0xffffff,
        _ => 0x808080,
    };
    slint::Color::from_argb_encoded(0xff00_0000 | rgb)
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
    /// --profile: more profiles, before the built-in ones.
    profile: Option<PathBuf>,
    /// --report: print the report for a new profile and exit.
    report: bool,
}

const USAGE: &str = "Usage: escp-utility [--utility] [--dry-run] [--dump-dir DIR] [--queue NAME] [--profile FILE]
       escp-utility --report [--queue NAME] [--profile FILE]

  --utility       open on the Maintenance tab (default: Main)
  --dry-run       write each job to a file and show a hexdump; never calls lp,
                  and show the lpoptions command instead of saving settings
  --dump-dir DIR  folder for dry-run files (default: ~/.cache/escp-utility)
  --queue NAME    CUPS queue of the printer (default: the only Epson queue;
                  with several, a list to choose from)
  --profile FILE  maintenance profiles in the profiles.toml format; they
                  come before the built-in ones (to try a new model)
  --report        print what a new profile needs (identity, PPD NickName,
                  lpoptions -l, one status reply with the serial masked);
                  reads the printer status once, sends no job
";

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut queue = None;
    let mut dry = false;
    let mut dir = None;
    let mut utility = false;
    let mut profile = None;
    let mut report = false;
    let mut it = args;
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dry-run" => dry = true,
            "--utility" => utility = true,
            "--report" => report = true,
            "--dump-dir" => dir = Some(PathBuf::from(it.next().ok_or("--dump-dir needs a value")?)),
            "--queue" => queue = Some(it.next().ok_or("--queue needs a value")?),
            "--profile" => profile = Some(PathBuf::from(it.next().ok_or("--profile needs a value")?)),
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("escp-utility {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument: {other}\n\n{USAGE}")),
        }
    }
    if dir.is_some() && !dry {
        return Err("--dump-dir needs --dry-run".into());
    }
    Ok(Args { queue, dump_dir: dry.then(|| dir.unwrap_or_else(transport::default_dump_dir)), utility, profile, report })
}

fn main() -> Result<(), slint::PlatformError> {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let mut profiles = match &args.profile {
        None => vec![],
        Some(f) => match std::fs::read_to_string(f).map_err(|e| e.to_string()).and_then(|t| profile::parse(&t)) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("{}: {e}", f.display());
                std::process::exit(2);
            }
        },
    };
    profiles.extend(profile::builtin());
    if args.report {
        let queue = match args.queue {
            Some(q) => q,
            None => match printer::epson_queues().as_slice() {
                [(q, _)] => q.name.clone(),
                qs => {
                    let names: Vec<&str> = qs.iter().map(|(q, _)| q.name.as_str()).collect();
                    eprintln!("--report needs --queue NAME (Epson queues: {})", names.join(", "));
                    std::process::exit(2);
                }
            },
        };
        print!("{}", report::run(&queue, &profiles));
        return Ok(());
    }
    // The platform must exist before the app_id can be set (set_xdg_app_id
    // fails with NoPlatform otherwise): select it now, as the first window
    // would (SLINT_BACKEND is honoured).
    slint::BackendSelector::new().select()?;
    // The Wayland app_id (X11 WM_CLASS) of every window: GNOME matches it to
    // escp-utility.desktop (StartupWMClass), so the sheet and its
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
    // The queue's PPD from cupsd (read-only): the sheet, and the device ID
    // when no USB one is cached.
    let ppd = ppd::fetch(&queue).and_then(|t| ppd::parse(&t)).inspect_err(|e| eprintln!("escp-utility: {e}")).ok();
    let printer = Identity::of(&queue, ppd.as_ref().and_then(|p| p.device_id.as_deref()), ppd.as_ref().and_then(|p| p.nickname.as_deref()));
    let profile = profile::find(&profiles, printer.id.mdl()).cloned();
    let sheet = match ppd {
        Some(p) if p.nickname.as_deref().is_some_and(|n| n.starts_with(L800_NICKNAME)) => None,
        Some(p) => Some(ppd_sheet::Sheet::new(p)),
        // No PPD: the L800 keeps its tables; other printers get an empty sheet.
        None if profile.as_ref().is_some_and(Profile::l800_windows) => None,
        None => Some(ppd_sheet::Sheet::new(ppd::Ppd::default())),
    };
    let app = App::new(args.dump_dir.map_or(Mode::Live, Mode::DryRun), printer, profile, sheet)?;
    APP.with(|a| *a.borrow_mut() = Rc::downgrade(&app));
    app.main.set_dry_run(app.dry_run());
    app.main.set_current_tab(if args.utility { 4 } else { 1 });
    fill_lists(&app.main.global::<Prefs>());
    app.show_defaults();
    // The sheet waits for the load, so no early edit (or OK) is lost to it.
    app.settings_io.set(true);
    app.refresh_busy();
    let queue = app.printer.queue.clone();
    bg(move || settings::load_raw(&queue), |app, res| {
        app.settings_io.set(false);
        app.refresh_busy();
        match (res, &app.ppd) {
            (Ok((listing, dest)), Some(st)) => {
                app.ppd_set(st, st.sheet.read_listing(&listing));
                ppd_sheet::show_std(&app.main.global::<Prefs>(), &ppd_sheet::Std::parse(&dest));
            }
            (Ok((listing, dest)), None) => app.set_prefs(&Values::parse(&listing, &dest)),
            (Err(e), _) => eprintln!("escp-utility: cannot read the printing preferences: {e}"),
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
    // Epson's driver download page for the model instead.
    ui.main.on_driver_update(on(|a| launch("xdg-open", &driver_page(a.printer.id.mdl()))));
    // OK = save + close, Apply = save, Cancel (or the close box) = discard.
    ui.main.on_ok(on(|a| a.save_prefs(true)));
    ui.main.on_apply(on(|a| a.save_prefs(false)));
    ui.main.on_cancel(on(|a| {
        // Nothing is saved: Apply goes off while the app shuts down.
        a.main.global::<Prefs>().set_dirty(false);
        let _ = slint::quit_event_loop();
    }));
    ui.main.window().on_close_requested(|| {
        let _ = slint::quit_event_loop();
        slint::CloseRequestResponse::HideWindow
    });
    let prefs = ui.main.global::<Prefs>();
    // After every edit: re-apply the constraints and refill the lists.
    prefs.on_edited(on(|a| match &a.ppd {
        // The PPD sheet: the control that changed, as an edit (else show
        // the selection again). No RefCell borrow may span ppd_set.
        Some(st) => {
            let e = st.sheet.edit_from(&a.main.global::<Prefs>(), &st.view.borrow());
            match e {
                Some(e) => a.ppd_edit(st, &e),
                None => {
                    let sel = st.sel.borrow().clone();
                    a.ppd_set(st, sel);
                }
            }
        }
        None => a.set_prefs(&a.prefs()),
    }));
    prefs.on_pick(|list, row| {
        let Some(a) = app() else { return };
        let row = usize::try_from(row).unwrap_or(0);
        if let Some(st) = &a.ppd {
            let e = st.sheet.pick(&st.view.borrow(), &list, row);
            if let Some(e) = e {
                a.ppd_edit(st, &e);
            }
            return a.touched();
        }
        let mut v = a.prefs();
        match list.as_str() {
            "media" => v.media = v.media_list().get(row).copied().unwrap_or(v.media),
            "size" => v.size = v.size_list().get(row).copied().unwrap_or(v.size),
            "output" => v.output_size = v.output_list().get(row).copied().unwrap_or(v.output_size),
            "quality" => v.quality = settings::advanced_qualities(v.media).get(row).map_or(v.quality, |q| q.1),
            _ => return,
        }
        a.set_prefs(&v);
        a.touched();
    });
    prefs.on_open_dialog(|id| {
        if let Some(a) = app() {
            a.open_dialog(&id);
        }
    });
    // Show Settings / Hide Settings toggles the Current Settings window.
    prefs.on_toggle_settings(on(|a| a.show_settings(!a.main.global::<Prefs>().get_settings_shown())));
    prefs.on_reset_defaults(on(|a| {
        a.show_defaults();
        a.touched();
    }));
    prefs.on_preset_picked(|i| {
        let (Some(a), Some(preset)) = (app(), usize::try_from(i).ok().and_then(|i| settings::PRESETS.get(i))) else { return };
        // The presets are the L800's (the list is off for other printers).
        if a.ppd.is_some() {
            return;
        }
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
        d.on_start(on(|a| a.cleaning_start()));
        d.on_cancel(on(|a| a.close(&a.cleaning)));
        d
    }

    pub fn power() -> PowerCleaningDialog {
        let d = make(PowerCleaningDialog::new);
        d.on_ok(on(|a| {
            a.close(&a.power);
            a.run_cleaning();
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

    pub fn printer_options() -> PrinterOptionsDialog {
        let d = make(PrinterOptionsDialog::new);
        d.on_picked(|i, row| {
            let Some(a) = app() else { return };
            let Some(st) = &a.ppd else { return };
            let e = st.sheet.option_pick(&st.view.borrow(), usize::try_from(i).unwrap_or(0), usize::try_from(row).unwrap_or(0));
            if let Some(e) = e {
                a.ppd_edit(st, &e);
                a.printer_options.set_rows(slint::VecModel::from_slice(&ppd_sheet::option_rows(&st.view.borrow())));
                a.touched();
            }
        });
        d.on_ok(on(|a| a.close(&a.printer_options)));
        d.on_cancel(on(|a| a.printer_options_cancel()));
        d.window().on_close_requested(|| {
            Timer::single_shot(Duration::ZERO, on(|a| a.printer_options_cancel()));
            slint::CloseRequestResponse::KeepWindowShown
        });
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
        let app = App::new(Mode::DryRun(std::env::temp_dir()), Identity { queue: "L800".into(), ..Default::default() }, None, None).unwrap();
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
        let app = App::new(Mode::DryRun(std::env::temp_dir()), Identity { queue: "L800".into(), ..Default::default() }, None, None).unwrap();
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

    /// The old fixed L800 link stays the L800's; other models get theirs.
    #[test]
    fn driver_pages() {
        assert_eq!(driver_page(Some("L800")), "https://download-center.epson.com/softwares/?device_id=L800&region=ID&os=DEBX64&language=en");
        assert!(driver_page(Some("ET-2750 Series")).contains("?device_id=ET-2750&"));
        assert!(driver_page(Some("WF-C5790 Series")).contains("device_id=WF-C5790&"));
        assert!(driver_page(None).starts_with("https://download-center.epson.com/softwares/?region=ID&"));
    }

    /// PPD_FIXTURES: the L800 PPD keeps the Windows tables; an escpr L805
    /// does not (ESCPR_PPD_DIR).
    #[test]
    fn l800_nickname_picks_the_windows_tables() {
        let Ok(dir) = std::env::var("PPD_FIXTURES") else { return eprintln!("PPD_FIXTURES unset: skipped") };
        let p = ppd::parse(&std::fs::read_to_string(PathBuf::from(dir).join("L800.ppd")).unwrap()).unwrap();
        assert!(p.nickname.unwrap().starts_with(L800_NICKNAME));
        assert!(!"EPSON L805 Series , Epson Inkjet Printer Driver (ESC/P-R) for Linux".starts_with(L800_NICKNAME));
    }

    /// The PPD sheet on the pages: an edit there comes back as the right
    /// PPD choice, and Reset Defaults restores the PPD's.
    #[test]
    fn ppd_sheet_on_the_pages() {
        let Ok(dir) = std::env::var("ESCPR_PPD_DIR") else { return eprintln!("ESCPR_PPD_DIR unset: skipped") };
        let text = std::fs::read_to_string(PathBuf::from(dir).join("Epson-L805_Series-epson-escpr-en.ppd")).unwrap();
        let sheet = ppd_sheet::Sheet::new(ppd::parse(&text).unwrap());
        let _ = slint::platform::set_platform(Box::new(Headless));
        let app = App::new(Mode::DryRun(std::env::temp_dir()), Identity { queue: "L805".into(), ..Default::default() }, None, Some(sheet)).unwrap();
        let p = app.main.global::<Prefs>();
        fill_lists(&p);
        let st = app.ppd.as_ref().unwrap();
        app.ppd_set(st, st.sheet.defaults());
        assert!(p.get_ppd_mode() && !p.get_has_source() && p.get_has_media());
        // The High radio, as the page sets it.
        p.set_quality(3);
        let e = st.sheet.edit_from(&p, &st.view.borrow()).unwrap();
        app.ppd_edit(st, &e);
        assert_eq!(st.sel.borrow()["MediaType"], "PLAIN_HIGH");
        // Grayscale on: Ink=MONO.
        p.set_grayscale(true);
        let e = st.sheet.edit_from(&p, &st.view.borrow()).unwrap();
        app.ppd_edit(st, &e);
        assert_eq!(st.sel.borrow()["Ink"], "MONO");
        p.set_copies(2);
        app.save_prefs(false); // dry run: shows the command
        let shown = app.dump.get_content().to_string();
        assert!(shown.contains("MediaType=PLAIN_HIGH") && shown.contains("Ink=MONO") && shown.contains("copies=2"), "{shown}");
        assert!(!shown.contains("PageRegion"));
    }

    /// Printer Options [Cancel] undoes its picks and Apply's state.
    #[test]
    fn printer_options_cancel_restores_apply() {
        let Ok(dir) = std::env::var("ESCPR_PPD_DIR") else { return eprintln!("ESCPR_PPD_DIR unset: skipped") };
        let text = std::fs::read_to_string(PathBuf::from(dir).join("Epson-ET-2750_Series-epson-escpr-en.ppd")).unwrap();
        let _ = slint::platform::set_platform(Box::new(Headless));
        let sheet = Some(ppd_sheet::Sheet::new(ppd::parse(&text).unwrap()));
        let app = App::new(Mode::DryRun(std::env::temp_dir()), Identity { queue: "ET".into(), ..Default::default() }, None, sheet).unwrap();
        let p = app.main.global::<Prefs>();
        let st = app.ppd.as_ref().unwrap();
        app.ppd_set(st, st.sheet.defaults());
        assert!(!p.get_dirty());
        app.open_dialog("printer-options");
        // What the dialog's pick does: Duplex, long edge.
        let e = st.sheet.option_pick(&st.view.borrow(), 0, 1).unwrap();
        app.ppd_edit(st, &e);
        app.touched();
        assert_eq!((st.sel.borrow()["Duplex"].as_str(), p.get_dirty()), ("DuplexNoTumble", true));
        app.printer_options_cancel();
        assert_eq!((st.sel.borrow()["Duplex"].as_str(), p.get_dirty()), ("None", false));
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
        let a = args(&["--report", "--profile", "p.toml"]).unwrap();
        assert!(a.report);
        assert_eq!(a.profile, Some(PathBuf::from("p.toml")));
    }
    #[test]
    fn utility_flag_alone() {
        let a = args(&["--utility"]).unwrap();
        assert!(a.utility && !a.report && a.dump_dir.is_none());
        assert!(!args(&[]).unwrap().utility);
    }

    #[test]
    fn dry_run_without_dir_uses_default() {
        let a = args(&["--dry-run"]).unwrap();
        assert_eq!(a.dump_dir, Some(transport::default_dump_dir()));
    }

    /// --dump-dir may come before --dry-run; the check runs after parsing.
    #[test]
    fn dump_dir_order_does_not_matter() {
        let a = args(&["--dump-dir", "/tmp/x", "--dry-run"]).unwrap();
        assert_eq!(a.dump_dir, Some(PathBuf::from("/tmp/x")));
        assert_eq!(args(&["--dump-dir", "/tmp/x"]).err().unwrap(), "--dump-dir needs --dry-run");
    }

    #[test]
    fn missing_values_name_the_flag() {
        for f in ["--queue", "--profile", "--dump-dir"] {
            assert_eq!(args(&["--dry-run", f]).err().unwrap(), format!("{f} needs a value"));
        }
    }

    /// A flag's value is taken as-is, even when it looks like a flag.
    #[test]
    fn value_is_next_arg_verbatim() {
        let a = args(&["--queue", "--utility"]).unwrap();
        assert_eq!(a.queue.as_deref(), Some("--utility"));
        assert!(!a.utility);
    }

    #[test]
    fn repeated_flags_last_wins() {
        let a = args(&["--queue", "A", "--queue", "B", "--profile", "x", "--profile", "y", "--utility", "--utility"]).unwrap();
        assert_eq!(a.queue.as_deref(), Some("B"));
        assert_eq!(a.profile, Some(PathBuf::from("y")));
        assert!(a.utility);
    }

    #[test]
    fn unknown_argument_names_it_and_shows_usage() {
        let e = args(&["--queue", "Q", "--colour"]).err().unwrap();
        assert!(e.starts_with("unknown argument: --colour\n"), "{e}");
        assert!(e.ends_with(USAGE));
        // Positional args and single-dash typos are rejected too.
        assert!(args(&["L800"]).is_err());
        assert!(args(&["-u"]).is_err());
    }

    /// --help prints USAGE and exits, so check USAGE covers every flag.
    #[test]
    fn usage_lists_every_flag() {
        for f in ["--utility", "--dry-run", "--dump-dir", "--queue", "--profile", "--report"] {
            assert!(USAGE.contains(f), "{f}");
        }
    }
}

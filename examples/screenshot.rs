//! Renders every window offscreen with Slint's software renderer and writes
//! one PPM file per window, for layout review against the Windows dialogs.
//! Nothing here talks to CUPS or the printer.
//!
//! cargo run --example screenshot -- OUT_DIR

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::rc::Rc;

use slint::Rgb8Pixel;
use slint::platform::software_renderer::SoftwareRenderer;
use slint::platform::{Platform, Renderer, WindowAdapter, WindowEvent, WindowProperties};
use slint::{ComponentHandle, LogicalSize, PhysicalSize, PlatformError, WindowSize};

slint::include_modules!();

#[path = "../src/texts.rs"]
#[allow(dead_code)] // the example shows only some of the texts
mod texts;
use texts::*;

#[path = "../src/layouts.rs"]
mod layouts;

#[path = "../src/settings.rs"]
#[allow(dead_code)] // only the tables are used here
mod settings;

#[path = "../src/ppd.rs"]
#[allow(dead_code)]
mod ppd;

#[path = "../src/ppd_sheet.rs"]
#[allow(dead_code)]
mod ppd_sheet;

/// The escpr PPD whose NickName starts with `nick`, from $ESCPR_PPD_DIR
/// (Epson's files, not in git).
fn escpr_sheet(nick: &str) -> Option<ppd_sheet::Sheet> {
    let dir = std::env::var("ESCPR_PPD_DIR").ok()?;
    std::fs::read_dir(dir).ok()?.flatten().find_map(|e| {
        let text = std::fs::read_to_string(e.path()).ok()?;
        let ppd = ppd::parse(&text).ok().filter(|_| text.contains(&format!("*NickName: \"{nick} ")))?;
        Some(ppd_sheet::Sheet::new(ppd))
    })
}

/// Same lists as main.rs `fill_lists`.
fn fill_lists(p: &Prefs) {
    let labels = |list: &[settings::Choice]| -> slint::ModelRc<slint::SharedString> {
        slint::VecModel::from_slice(&list.iter().map(|c| slint::SharedString::from(c.label)).collect::<Vec<_>>())
    };
    let flags = |list: &[settings::Choice]| -> slint::ModelRc<bool> {
        slint::VecModel::from_slice(&list.iter().map(|c| c.borderless).collect::<Vec<_>>())
    };
    p.set_quality_labels(labels(settings::QUALITY));
    p.set_source_labels(labels(settings::SOURCES));
    p.set_media_labels(labels(settings::MEDIA));
    p.set_size_labels(labels(settings::SIZES));
    p.set_output_labels(labels(settings::output_sizes()));
    p.set_color_mode_labels(labels(settings::COLOR_MODES));
    p.set_gamma_labels(labels(settings::GAMMAS));
    p.set_preset_labels(slint::VecModel::from_slice(
        &settings::PRESETS.iter().map(|x| slint::SharedString::from(x.name)).collect::<Vec<_>>(),
    ));
    p.set_source_borderless(flags(settings::SOURCES));
    p.set_media_borderless(flags(settings::MEDIA));
    p.set_size_borderless(flags(settings::SIZES));
    p.set_first_document_size(settings::FIRST_DOCUMENT_SIZE as i32);
    p.set_adv_quality_labels(slint::VecModel::from_slice(
        &settings::advanced_qualities(0).iter().map(|(l, _)| slint::SharedString::from(*l)).collect::<Vec<_>>(),
    ));
    p.set_adv_quality_pos(settings::advanced_quality_pos(0, 1) as i32);
    p.set_color_mode(1);
    p.set_gamma(1);
    p.set_quality(1);
}

/// An offscreen window that remembers the size its component asks for
/// (some dialogs are as tall as their text).
struct Shot {
    window: slint::Window,
    renderer: SoftwareRenderer,
    size: Cell<PhysicalSize>,
    wanted: Cell<LogicalSize>,
}

impl WindowAdapter for Shot {
    fn window(&self) -> &slint::Window {
        &self.window
    }
    fn size(&self) -> PhysicalSize {
        self.size.get()
    }
    fn set_size(&self, size: WindowSize) {
        self.size.set(size.to_physical(1.0));
        self.window.dispatch_event(WindowEvent::Resized { size: size.to_logical(1.0) });
    }
    fn renderer(&self) -> &dyn Renderer {
        &self.renderer
    }
    fn update_window_properties(&self, properties: WindowProperties<'_>) {
        self.wanted.set(properties.layout_constraints().preferred);
    }
}

/// The window that was created last, shared with the platform.
type Last = Rc<RefCell<Option<Rc<Shot>>>>;

struct Offscreen {
    last: Last,
}

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let w = Rc::new_cyclic(|me: &std::rc::Weak<Shot>| Shot {
            window: slint::Window::new(me.clone()),
            renderer: SoftwareRenderer::new(),
            size: Cell::default(),
            wanted: Cell::default(),
        });
        *self.last.borrow_mut() = Some(w.clone());
        Ok(w)
    }
}

/// Renders the window at the size it asks for.
fn render(last: &Last, name: &str, c: &impl ComponentHandle, out: &std::path::Path) {
    c.show().expect("show");
    let win = last.borrow().clone().expect("window");
    let size = win.wanted.get();
    let (w, h) = (size.width.round() as u32, size.height.round() as u32);
    win.set_size(WindowSize::Logical(size));
    let mut buf = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (w * h) as usize];
    win.renderer.render(&mut buf, w as usize);
    let path = out.join(format!("{name}.ppm"));
    let mut f = std::fs::File::create(&path).expect("create");
    write!(f, "P6\n{w} {h}\n255\n").unwrap();
    for p in &buf {
        f.write_all(&[p.r, p.g, p.b]).unwrap();
    }
    println!("{}", path.display());
}

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).expect("usage: screenshot OUT_DIR"));
    std::fs::create_dir_all(&out).unwrap();
    let p = Last::default();
    slint::platform::set_platform(Box::new(Offscreen { last: p.clone() })).unwrap();

    let main = MainWindow::new().unwrap();
    main.set_version_text(env!("CARGO_PKG_VERSION").into());
    main.set_dry_run(true);
    // profiles.toml: the L800 has no `tested` entry yet.
    main.set_experimental(true);
    fill_lists(&main.global::<Prefs>());
    // Ink levels of the live L800 reply in src/status/l800_st2_live_masked.hex.
    let inks: Vec<InkLevel> = [("Cyan", 0x00a0e0, 47), ("Magenta", 0xd0168c, 37), ("Light Magenta", 0xf08cc8, 18), ("Light Cyan", 0x7fd0f0, 3), ("Black", 0x202020, 81), ("Yellow", 0xf0c800, 27)]
        .iter()
        .map(|&(n, c, l)| InkLevel { name: n.into(), color: slint::Color::from_argb_encoded(0xff00_0000 | c), level: l })
        .collect();
    main.global::<Prefs>().set_inks(slint::VecModel::from_slice(&inks));
    for (tab, name) in [(0, "sheet-904-shortcuts"), (1, "sheet-900-main"), (2, "sheet-901-advanced"), (3, "sheet-902-page-layout"), (4, "main")] {
        main.set_current_tab(tab);
        render(&p, name, &main, &out);
    }
    // Page Layout with Multi-Page (4 per sheet), borders and collate on.
    let prefs = main.global::<Prefs>();
    prefs.set_multi_page(true);
    prefs.set_pages_index(1);
    prefs.set_page_borders(true);
    prefs.set_collate(true);
    prefs.set_reduce(true);
    main.set_current_tab(3);
    render(&p, "sheet-902-page-layout-multi", &main, &out);
    prefs.set_multi_page(false);
    prefs.set_collate(false);
    prefs.set_reduce(false);

    // A printer without the maintenance gate.
    main.set_maintenance(false);
    main.set_current_tab(4);
    render(&p, "main-no-escp", &main, &out);
    main.set_maintenance(true);

    let d = ColorControlsDialog::new().unwrap();
    d.set_color_mode_labels(main.global::<Prefs>().get_color_mode_labels());
    d.set_gamma_labels(main.global::<Prefs>().get_gamma_labels());
    d.set_color_mode(1);
    d.set_gamma(1);
    d.set_brightness(5);
    d.set_cyan(-10);
    d.set_yellow(12);
    render(&p, "color-controls-4300", &d, &out);

    for (name, l) in [
        ("page-order-4630", &layouts::PAGE_ORDER),
        ("poster-4700", &layouts::POSTER),
        ("margins-4750", &layouts::MARGINS),
        ("custom-settings-4500", &layouts::CUSTOM_SETTINGS),
        ("watermark-16200", &layouts::WATERMARK),
    ] {
        let d = LayoutDialog::new().unwrap();
        d.set_caption(l.caption.into());
        d.set_reason(l.reason.into());
        d.set_client_w(l.size.0);
        d.set_client_h(l.size.1);
        let controls: Vec<LayoutControl> =
            l.controls.iter().map(|&(kind, x, y, w, h, text)| LayoutControl { kind, x, y, w, h, text: text.into() }).collect();
        d.set_controls(slint::VecModel::from_slice(&controls));
        render(&p, name, &d, &out);
    }

    let d = CurrentSettingsDialog::new().unwrap();
    let lines: Vec<SettingLine> = [
        ("Paper Source", "Sheet"),
        ("Paper Type", "plain papers"),
        ("Quality", "Text"),
        ("Document Size", "A4 210 x 297 mm"),
        ("Borderless", "Off"),
        ("Orientation", "Portrait"),
        ("Reverse Order", "Off"),
        ("Grayscale", "Off"),
        ("Color Management", "Color Controls"),
        ("Color Mode", "EPSON Vivid"),
        ("Page Layout", "Off"),
        ("Scale ratio", "Off"),
    ]
    .iter()
    .map(|&(l, v)| SettingLine { label: l.into(), value: v.into() })
    .collect();
    d.set_lines(slint::VecModel::from_slice(&lines));
    render(&p, "current-settings-4050", &d, &out);

    let d = NozzleCheckDialog::new().unwrap();
    d.set_body(NOZZLE_INTRO.into());
    render(&p, "nozzle-210", &d, &out);

    let d = NozzleResultDialog::new().unwrap();
    d.set_body(NOZZLE_RESULT.into());
    render(&p, "nozzle-result-211", &d, &out);

    let d = HeadCleaningDialog::new().unwrap();
    d.set_body(CLEAN_INTRO.into());
    render(&p, "cleaning-226", &d, &out);

    let d = PowerCleaningDialog::new().unwrap();
    d.set_body(POWER_CONFIRM.into());
    render(&p, "power-360", &d, &out);

    let d = CleaningDoneDialog::new().unwrap();
    d.set_body(CLEAN_DONE.into());
    render(&p, "cleaning-done-227", &d, &out);

    let d = WaitDialog::new().unwrap();
    render(&p, "wait-201", &d, &out);

    let d = ProgressDialog::new().unwrap();
    d.set_body("Please wait...".into());
    render(&p, "progress-223-wait", &d, &out);
    d.set_body(CLEANING_NOW.into());
    d.set_bar_visible(true);
    d.set_value(0.45);
    d.set_animate(false);
    d.set_frame(4); // cyan group, mid-sweep
    render(&p, "progress-223", &d, &out);
    // Every animation frame, for review.
    for f in 0..12 {
        d.set_frame(f);
        render(&p, &format!("anim-223-{f:02}"), &d, &out);
    }

    let d = PaperGuideDialog::new().unwrap();
    d.set_body(PAPER_GUIDE.into());
    render(&p, "paper-guide-265", &d, &out);

    let d = CleaningDoneDialog::new().unwrap();
    d.set_body(CLEAN_DONE_STATUS.into());
    render(&p, "cleaning-done-224", &d, &out);

    let d = MessageDialog::new().unwrap();
    d.set_utility("Head Cleaning".into());
    d.set_heading("A problem has occurred.".into());
    d.set_body(SEE_DETAILS.into());
    d.set_show_details(true);
    render(&p, "problem-270", &d, &out);

    // Values of the live L800 reply in src/status/l800_st2_live_masked.hex.
    let d = StatusMonitorDialog::new().unwrap();
    d.set_title_text("Ink low".into());
    d.set_body("Light Cyan: You can keep printing. Refill the ink tank soon. The ink levels shown are estimates and can differ from the real levels. To be sure, look at the ink tanks on the printer.".into());
    d.set_inks(slint::VecModel::from_slice(&inks));
    render(&p, "status-monitor", &d, &out);

    // A generic printer with nine inks (synthetic levels).
    let d = StatusMonitorDialog::new().unwrap();
    d.set_title_text("Ink low".into());
    d.set_body("Light Black: You can keep printing, but get new ink ready soon. The ink levels shown are estimates and can differ from the real levels.".into());
    d.set_note("The ink levels shown are estimates and can differ from the real levels.".into());
    let nine: Vec<InkLevel> = [("Matte Black", 0x202020, 50), ("Black", 0x202020, 60), ("Cyan", 0x00a0e0, 70), ("Magenta", 0xd0168c, 3), ("Yellow", 0xf0c800, 80), ("Light Cyan", 0x7fd0f0, 90), ("Light Magenta", 0xf08cc8, 55), ("Light Black", 0x707070, 8), ("Light Light Black", 0xa8a8a8, 30)]
        .iter()
        .map(|&(n, c, l)| InkLevel { name: n.into(), color: slint::Color::from_argb_encoded(0xff00_0000 | c), level: l })
        .collect();
    d.set_inks(slint::VecModel::from_slice(&nine));
    render(&p, "status-monitor-generic", &d, &out);

    let d = PowerCleaningDialog::new().unwrap();
    d.set_body(CONFIRM_GENERIC.into());
    d.set_picture(false);
    render(&p, "confirm-generic", &d, &out);

    let d = ChooserDialog::new().unwrap();
    let queues = ["L800  (Epson L800 Series - epson-inkjet-printer 1.0.2-1)", "ET-2750  (Epson ET-2750 Series, driverless)"];
    d.set_queues(slint::VecModel::from_slice(&queues.map(slint::SharedString::from)));
    render(&p, "chooser", &d, &out);
    d.set_queues(slint::ModelRc::default());
    render(&p, "chooser-none", &d, &out);

    // Last: render() draws the window created last.
    // Another printer's sheet, from its escpr PPD: the L805 on Main and
    // Advanced, with Premium Glossy, High and borderless 10 x 15 cm.
    match escpr_sheet("EPSON L805 Series") {
        None => eprintln!("ESCPR_PPD_DIR unset or without the L805 PPD: no PPD sheet screenshots"),
        Some(sheet) => {
            let m = MainWindow::new().unwrap();
            m.set_version_text(env!("CARGO_PKG_VERSION").into());
            m.set_queue("L805".into());
            m.set_dry_run(true);
            let prefs = m.global::<Prefs>();
            fill_lists(&prefs);
            prefs.set_inks(slint::VecModel::from_slice(&inks));
            let glossy = sheet.view(&sheet.defaults()).media.items.iter().find(|(_, l)| l == "Epson Premium Glossy").unwrap().0;
            let mut sel = sheet.apply(&sheet.defaults(), &ppd_sheet::Edit::Media(glossy));
            for e in [ppd_sheet::Edit::Quality(3), ppd_sheet::Edit::Borderless(true)] {
                sel = sheet.apply(&sel, &e);
            }
            ppd_sheet::show(&prefs, &sheet.view(&sel));
            ppd_sheet::show_std(&prefs, &ppd_sheet::Std::default());
            for (tab, name) in [(1, "sheet-ppd-l805-main"), (2, "sheet-ppd-l805-advanced"), (3, "sheet-ppd-l805-page-layout")] {
                m.set_current_tab(tab);
                render(&p, name, &m, &out);
            }
        }
    }
    if let Some(sheet) = escpr_sheet("EPSON ET-2750 Series") {
        let d = PrinterOptionsDialog::new().unwrap();
        d.set_rows(slint::VecModel::from_slice(&ppd_sheet::option_rows(&sheet.view(&sheet.defaults()))));
        render(&p, "printer-options-et2750", &d, &out);
    }
}

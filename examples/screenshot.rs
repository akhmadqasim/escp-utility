//! Renders every window offscreen with Slint's software renderer and writes
//! one PPM file per window, for layout review against the Windows dialogs.
//! Nothing here talks to CUPS or the printer.
//!
//! cargo run --example screenshot -- OUT_DIR

use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

use slint::Rgb8Pixel;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, PhysicalSize, PlatformError};

slint::include_modules!();

#[path = "../src/layouts.rs"]
mod layouts;

#[path = "../src/settings.rs"]
#[allow(dead_code)] // only the tables are used here
mod settings;

/// Same lists as main.rs `fill_lists`.
fn fill_lists(p: &Prefs) {
    let labels = |list: &[settings::Choice]| -> slint::ModelRc<slint::SharedString> {
        slint::VecModel::from_slice(&list.iter().map(|c| slint::SharedString::from(c.label)).collect::<Vec<_>>())
    };
    let flags = |list: &[settings::Choice]| -> slint::ModelRc<bool> {
        slint::VecModel::from_slice(&list.iter().map(|c| c.borderless).collect::<Vec<_>>())
    };
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
    p.set_color_mode(1);
    p.set_gamma(1);
    p.set_quality(1);
}

/// The window that was created last, shared with the platform.
type Last = Rc<RefCell<Option<Rc<MinimalSoftwareWindow>>>>;

struct Offscreen {
    last: Last,
}

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let w = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        *self.last.borrow_mut() = Some(w.clone());
        Ok(w)
    }
}

fn render(last: &Last, name: &str, (w, h): (u32, u32), show: impl FnOnce() -> Result<(), PlatformError>, out: &std::path::Path) {
    show().expect("show");
    let win = last.borrow().clone().expect("window");
    win.set_size(PhysicalSize::new(w, h));
    let mut buf = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (w * h) as usize];
    win.request_redraw();
    win.draw_if_needed(|r| {
        r.render(&mut buf, w as usize);
    });
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
    fill_lists(&main.global::<Prefs>());
    // Ink levels of the live L800 reply in src/status/l800_st2_live_masked.hex.
    let inks: Vec<InkLevel> = [("Cyan", 0x00a0e0, 47), ("Magenta", 0xd0168c, 37), ("Light Magenta", 0xf08cc8, 18), ("Light Cyan", 0x7fd0f0, 3), ("Black", 0x202020, 81), ("Yellow", 0xf0c800, 27)]
        .iter()
        .map(|&(n, c, l)| InkLevel { name: n.into(), color: slint::Color::from_argb_encoded(0xff00_0000 | c), level: l })
        .collect();
    main.global::<Prefs>().set_inks(slint::VecModel::from_slice(&inks));
    for (tab, name) in [(0, "sheet-904-shortcuts"), (1, "sheet-900-main"), (2, "sheet-901-advanced"), (3, "sheet-902-page-layout"), (4, "main")] {
        main.set_current_tab(tab);
        render(&p, name, (459, 505), || main.show(), &out);
    }
    // Page Layout with Multi-Page (4 per sheet), borders and collate on.
    let prefs = main.global::<Prefs>();
    prefs.set_multi_page(true);
    prefs.set_pages_index(1);
    prefs.set_page_borders(true);
    prefs.set_collate(true);
    prefs.set_reduce(true);
    main.set_current_tab(3);
    render(&p, "sheet-902-page-layout-multi", (459, 505), || main.show(), &out);
    prefs.set_multi_page(false);
    prefs.set_collate(false);
    prefs.set_reduce(false);

    let d = ColorControlsDialog::new().unwrap();
    d.set_color_mode_labels(main.global::<Prefs>().get_color_mode_labels());
    d.set_gamma_labels(main.global::<Prefs>().get_gamma_labels());
    d.set_color_mode(1);
    d.set_gamma(1);
    d.set_brightness(5);
    d.set_cyan(-10);
    d.set_yellow(12);
    render(&p, "color-controls-4300", (555, 496), || d.show(), &out);

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
        render(&p, name, (l.size.0 as u32, l.size.1 as u32), || d.show(), &out);
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
    render(&p, "current-settings-4050", (213, 496), || d.show(), &out);

    let d = NozzleCheckDialog::new().unwrap();
    d.set_body("Before you print the nozzle check pattern, make sure the printer is switched on and connected to this computer.\n\nTake out the CD/DVD tray and set the front tray to the paper position; otherwise the nozzle check cannot run. Load A4 or Letter plain paper in the sheet feeder.\n\nClick [Print] to print the pattern, or [Cancel] to close.".into());
    render(&p, "nozzle-210", (420, 374), || d.show(), &out);

    let d = NozzleResultDialog::new().unwrap();
    d.set_body("Compare your printout with the samples above. If lines are missing or broken, click [Clean] to clean the print head. If the pattern is complete, click [Finish].".into());
    render(&p, "nozzle-result-211", (435, 414), || d.show(), &out);

    let d = HeadCleaningDialog::new().unwrap();
    d.set_body("First take out the CD/DVD tray and set the front tray to the paper position; otherwise head cleaning cannot run. Select [Cleaning], then click [Start] to clean the print head nozzles.\n\nCleaning uses ink, so clean only when printouts show faint areas or gaps. Run a Nozzle Check first to see whether cleaning is needed.\n\nNote: select [Power Cleaning] only if print quality has not improved after three normal cleanings. Power Cleaning uses more ink than a normal cleaning.\n\nCaution: keep the CD/DVD tray out until cleaning has finished.".into());
    render(&p, "cleaning-226", (420, 372), || d.show(), &out);

    let d = PowerCleaningDialog::new().unwrap();
    d.set_body("Do you really want to run Power Cleaning? It uses more ink than a normal cleaning.\n\nBefore you start, check that the transportation lock is in the unlocked (Printing) position, as in the picture below.".into());
    render(&p, "power-360", (435, 414), || d.show(), &out);

    let d = CleaningDoneDialog::new().unwrap();
    d.set_body("Cleaning is finished when the power light on the printer stops flashing.\n\nThen click [Print Nozzle Check Pattern] to check that the cleaning worked.".into());
    render(&p, "cleaning-done-227", (420, 335), || d.show(), &out);

    let d = WaitDialog::new().unwrap();
    render(&p, "wait-201", (240, 65), || d.show(), &out);

    let d = ProgressDialog::new().unwrap();
    d.set_body("The print head is being cleaned.\nPlease wait.\nCaution: keep the CD/DVD tray out until cleaning has finished.".into());
    d.set_bar_visible(true);
    d.set_value(0.45);
    d.set_animate(false);
    d.set_frame(4); // cyan group, mid-sweep
    render(&p, "progress-223", (420, 335), || d.show(), &out);
    // Every animation frame, for review.
    for f in 0..12 {
        d.set_frame(f);
        render(&p, &format!("anim-223-{f:02}"), (420, 335), || d.show(), &out);
    }

    let d = PaperGuideDialog::new().unwrap();
    d.set_body("Plain paper is used to clean the inside of the printer.\n\nLoad A4 or Letter plain paper in the printer, then click [Cleaning].\nRepeat this until the inside of the printer is clean.".into());
    render(&p, "paper-guide-265", (420, 335), || d.show(), &out);

    let d = CleaningDoneDialog::new().unwrap();
    d.set_body("The cleaning cycle has finished.\n\nTo check that it worked, click [Print Nozzle Check Pattern].".into());
    render(&p, "cleaning-done-224", (420, 335), || d.show(), &out);

    let d = MessageDialog::new().unwrap();
    d.set_utility("Head Cleaning".into());
    d.set_heading("A problem has occurred.".into());
    d.set_body("Click [Details] to see more.".into());
    d.set_show_details(true);
    render(&p, "problem-270", (371, 197), || d.show(), &out);

    // Values of the live L800 reply in src/status/l800_st2_live_masked.hex.
    let d = StatusMonitorDialog::new().unwrap();
    d.set_title_text("Ink low".into());
    d.set_body("Light Cyan: You can keep printing. Refill the ink tank soon. The ink levels shown are estimates and can differ from the real levels. To be sure, look at the ink tanks on the printer.".into());
    let inks: Vec<InkLevel> = [("Cyan", 0x00a0e0, 47), ("Magenta", 0xd0168c, 37), ("Light Magenta", 0xf08cc8, 18), ("Light Cyan", 0x7fd0f0, 3), ("Black", 0x202020, 81), ("Yellow", 0xf0c800, 27)]
        .iter()
        .map(|&(n, c, l)| InkLevel { name: n.into(), color: slint::Color::from_argb_encoded(0xff00_0000 | c), level: l })
        .collect();
    d.set_inks(slint::VecModel::from_slice(&inks));
    render(&p, "status-monitor", (479, 412), || d.show(), &out);
}

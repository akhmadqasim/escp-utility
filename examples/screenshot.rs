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

struct Offscreen {
    last: RefCell<Option<Rc<MinimalSoftwareWindow>>>,
}

impl Platform for Offscreen {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        let w = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        *self.last.borrow_mut() = Some(w.clone());
        Ok(w)
    }
}

fn render(platform: &Offscreen, name: &str, (w, h): (u32, u32), show: impl FnOnce() -> Result<(), PlatformError>, out: &std::path::Path) {
    show().expect("show");
    let win = platform.last.borrow().clone().expect("window");
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
    let platform = Offscreen { last: RefCell::new(None) };
    let p: &'static Offscreen = Box::leak(Box::new(platform));
    slint::platform::set_platform(Box::new(Forward(p))).unwrap();

    let main = MainWindow::new().unwrap();
    main.set_version_text(env!("CARGO_PKG_VERSION").into());
    main.set_dry_run(true);
    let (mw, mh) = (main.window().size().width.max(459), main.window().size().height.max(503));
    render(p, "main", (mw, mh), || main.show(), &out);

    let d = NozzleCheckDialog::new().unwrap();
    d.set_body("Before you print the nozzle check pattern, make sure the printer is switched on and connected to this computer.\n\nTake out the CD/DVD tray and set the front tray to the paper position; otherwise the nozzle check cannot run. Load A4 or Letter plain paper in the sheet feeder.\n\nClick [Print] to print the pattern, or [Cancel] to close.".into());
    render(p, "nozzle-210", (420, 374), || d.show(), &out);

    let d = NozzleResultDialog::new().unwrap();
    d.set_body("Compare your printout with the samples above. If lines are missing or broken, click [Clean] to clean the print head. If the pattern is complete, click [Finish].".into());
    render(p, "nozzle-result-211", (435, 414), || d.show(), &out);

    let d = HeadCleaningDialog::new().unwrap();
    d.set_body("First take out the CD/DVD tray and set the front tray to the paper position; otherwise head cleaning cannot run. Select [Cleaning], then click [Start] to clean the print head nozzles.\n\nCleaning uses ink, so clean only when printouts show faint areas or gaps. Run a Nozzle Check first to see whether cleaning is needed.\n\nNote: select [Power Cleaning] only if print quality has not improved after three normal cleanings. Power Cleaning uses more ink than a normal cleaning.\n\nCaution: keep the CD/DVD tray out until cleaning has finished.".into());
    render(p, "cleaning-226", (420, 372), || d.show(), &out);

    let d = PowerCleaningDialog::new().unwrap();
    d.set_body("Do you really want to run Power Cleaning? It uses more ink than a normal cleaning.\n\nBefore you start, check that the transportation lock is in the unlocked (Printing) position, as in the picture below.".into());
    render(p, "power-360", (435, 414), || d.show(), &out);

    let d = CleaningDoneDialog::new().unwrap();
    d.set_body("Cleaning is finished when the power light on the printer stops flashing.\n\nThen click [Print Nozzle Check Pattern] to check that the cleaning worked.".into());
    render(p, "cleaning-done-227", (420, 335), || d.show(), &out);

    let d = WaitDialog::new().unwrap();
    render(p, "wait-201", (240, 65), || d.show(), &out);

    let d = ProgressDialog::new().unwrap();
    d.set_body("The print head is being cleaned.\nPlease wait.\nCaution: keep the CD/DVD tray out until cleaning has finished.".into());
    d.set_bar_visible(true);
    d.set_value(0.45);
    render(p, "progress-223", (420, 335), || d.show(), &out);

    let d = CleaningDoneDialog::new().unwrap();
    d.set_body("The cleaning cycle has finished.\n\nTo check that it worked, click [Print Nozzle Check Pattern].".into());
    render(p, "cleaning-done-224", (420, 335), || d.show(), &out);

    let d = MessageDialog::new().unwrap();
    d.set_utility("Head Cleaning".into());
    d.set_heading("A problem has occurred.".into());
    d.set_body("Click [Details] to see more.".into());
    d.set_show_details(true);
    render(p, "problem-270", (371, 197), || d.show(), &out);

    // Values of the live L800 reply in src/status/l800_st2_live_masked.hex.
    let d = StatusMonitorDialog::new().unwrap();
    d.set_title_text("Ink low".into());
    d.set_body("Light Cyan: You can keep printing. Refill the ink tank soon. The ink levels shown are estimates and can differ from the real levels. To be sure, look at the ink tanks on the printer.".into());
    let inks: Vec<InkLevel> = [("Cyan", 0x00a0e0, 47), ("Magenta", 0xd0168c, 37), ("Light Magenta", 0xf08cc8, 18), ("Light Cyan", 0x7fd0f0, 3), ("Black", 0x202020, 81), ("Yellow", 0xf0c800, 27)]
        .iter()
        .map(|&(n, c, l)| InkLevel { name: n.into(), color: slint::Color::from_argb_encoded(0xff00_0000 | c), level: l })
        .collect();
    d.set_inks(Rc::new(slint::VecModel::from(inks)).into());
    render(p, "status-monitor", (479, 412), || d.show(), &out);
}

struct Forward(&'static Offscreen);
impl Platform for Forward {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        self.0.create_window_adapter()
    }
}

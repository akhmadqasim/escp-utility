//! Printing Preferences: the Windows driver settings mapped to the L800 PPD
//! and to standard CUPS job options, stored per user with `lpoptions`.
//!
//! Evidence: phase2/rea/SETTINGS.md (the Windows L800 tables in E_FCF0G5P.CFG
//! match the PPD lists 1:1) and `lpoptions -p L800 -l` for the PPD keys.
//! Storage (SETTINGS.md §3): `lpoptions -p <queue> -o key=value` writes
//! `~/.cups/lpoptions`. That needs no root, is per user like the Windows
//! DEVMODE, and GTK/GNOME and `lp` apply it.
//!
//! Labels are the Windows strings (E_IUIRG5P); the values are the PPD keys.

use std::collections::BTreeMap;
use std::process::Command;

/// One list entry: PPD value, Windows label, borderless allowed (SETTINGS.md §1).
pub struct Choice {
    pub ppd: &'static str,
    pub label: &'static str,
    pub borderless: bool,
}

const fn c(ppd: &'static str, label: &'static str, borderless: bool) -> Choice {
    Choice { ppd, label, borderless }
}

/// Main > Quality Option radios 1941-1945 = PPD `PrintQuality`.
pub const QUALITY: [&str; 5] = ["Draft", "Text", "TextImage", "Photo", "BestPhoto"];

/// Source combo 1962 = `InputSlot` (labels: strings 1530, 1574).
pub const SOURCES: &[Choice] = &[c("Sheet", "Sheet", true), c("CD_DVD", "CD/DVD", false)];

/// Type combo 1964 = `MediaType` (labels: strings 1100-1238).
pub const MEDIA: &[Choice] = &[
    c("PLAIN", "plain papers", false),
    c("EULTRAGLOSSY", "Epson Ultra Glossy", true),
    c("EPREMGLOSS", "Epson Premium Glossy", true),
    c("EPSGLOS", "Epson Premium Semigloss", true),
    c("EPENTRY", "Epson Glossy", true),
    c("EGCP", "Epson Photo", true),
    c("EMATTE", "Epson Matte", true),
    c("EPQIJ", "Epson Photo Quality Ink Jet", true),
    c("PHOTOSTICKERS", "Epson Photo Stickers", false),
    c("CD_DVD", "CD/DVD", false),
    c("ENVELOPE", "Envelope", false),
];

/// Size combo 1966 = `PageSize` (labels: E_IUIRG5P paper-size strings).
/// The last five are document sizes that need Reduce/Enlarge.
pub const SIZES: &[Choice] = &[
    c("A4", "A4 210 x 297 mm", true),
    c("4x6in", "10 x 15 cm (4 x 6 in)", true),
    c("5x7in", "13 x 18 cm (5 x 7 in)", true),
    c("A6", "A6 105 x 148 mm", false),
    c("A5", "A5 148 x 210 mm", false),
    c("B5", "B5 182 x 257 mm", false),
    c("3.5x5in", "9 x 13 cm (3.5 x 5 in)", true),
    c("5x8in", "13 x 20 cm (5 x 8 in)", false),
    c("8x10in", "20 x 25 cm (8 x 10 in)", false),
    c("169widesize", "16:9 wide size (102 x 181 mm)", true),
    c("100x148mm", "100 x 148 mm", true),
    c("ENVELOPE_10", "Envelope #10 4 1/8 x 9 1/2 in", false),
    c("ENVELOPEDL", "Envelope DL  110 x 220 mm", false),
    c("ENVELOPEC6", "Envelope C6  114 x 162 mm", false),
    c("Letter", "Letter 8 1/2 x 11 in", true),
    c("Legal", "Legal 8 1/2 x 14 in", false),
    c("A3", "A3 297 x 420 mm", false),
    c("A3+", "A3+ 329 x 483 mm", false),
    c("A2", "A2 420 x 594 mm", false),
    c("B4", "B4 257 x 364 mm", false),
    c("B3", "B3 364 x 515 mm", false),
];

/// Page Layout > By Output Paper Size = `OutputPaper`: the printable sizes.
pub fn output_sizes() -> &'static [Choice] {
    &SIZES[..16]
}

/// Advanced > Color Mode 3143 = `CorrectionColor` (strings 1501, 1504, 1510).
pub const COLOR_MODES: &[Choice] =
    &[c("EPSONStandard", "EPSON Standard", false), c("EPSONVivid", "EPSON Vivid", false), c("AdobeRGB", "Adobe RGB", false)];

/// Advanced > Gamma 3141 = `GammaValue`.
pub const GAMMAS: &[Choice] = &[c("1.8", "1.8", false), c("2.2", "2.2", false)];

/// Pages per Sheet counts (CUPS `number-up`).
pub const PAGES_PER_SHEET: [u32; 2] = [2, 4];
/// Poster Printing = `PosterPrinting`, with the number of sheets shown.
pub const POSTERS: [(&str, u32); 4] = [("2x1", 2), ("2x2", 4), ("3x3", 9), ("4x4", 16)];

/// Everything the four settings pages edit.
#[derive(Clone, Debug, PartialEq)]
pub struct Values {
    pub quality: usize,
    pub source: usize,
    pub media: usize,
    pub size: usize,
    pub borderless: bool,
    pub landscape: bool,
    pub grayscale: bool,
    pub reverse: bool,
    pub color_mode: usize,
    pub gamma: usize,
    pub reduce: bool,
    pub by_percent: bool,
    pub output_size: usize,
    /// 10..=400 %
    pub scale: u32,
    pub multi_page: bool,
    pub poster: bool,
    pub pages_index: usize,
    pub poster_index: usize,
    pub page_borders: bool,
    /// 1..=99
    pub copies: u32,
    pub collate: bool,
    pub rotate180: bool,
    pub mirror: bool,
    /// Color Controls (dialog 4300), each -25..=25.
    pub brightness: i32,
    pub contrast: i32,
    pub saturation: i32,
    pub cyan: i32,
    pub magenta: i32,
    pub yellow: i32,
}

impl Default for Values {
    /// The PPD defaults (`*Default...` lines, as `lpoptions -l` shows them with
    /// no user file) and the CUPS defaults.
    fn default() -> Self {
        Values {
            quality: 1, // Text
            source: 0,
            media: 0,
            size: 0,
            borderless: false,
            landscape: false,
            grayscale: false,
            reverse: false,
            color_mode: 1, // EPSON Vivid
            gamma: 1,      // 2.2
            reduce: false,
            by_percent: false,
            output_size: 0,
            scale: 100,
            multi_page: false,
            poster: false,
            pages_index: 0,
            poster_index: 0,
            page_borders: false,
            copies: 1,
            collate: false,
            rotate180: false,
            mirror: false,
            brightness: 0,
            contrast: 0,
            saturation: 0,
            cyan: 0,
            magenta: 0,
            yellow: 0,
        }
    }
}

fn index_of(list: &[Choice], ppd: &str) -> Option<usize> {
    list.iter().position(|ch| ch.ppd == ppd)
}

/// Index of A3 in `SIZES`: it and the sizes after it need Reduce/Enlarge.
pub const FIRST_DOCUMENT_SIZE: usize = 16;
const CD_DVD_MEDIA: usize = 9;
const VIVID: usize = 1;

impl Values {
    /// The PPD constraints, applied the way the Windows driver greys out
    /// controls (SETTINGS.md §1): CD/DVD needs A4, A3/A3+/A2/B4/B3 need
    /// Reduce/Enlarge, and EPSON Vivid allows only gamma 2.2.
    pub fn constrained(&self) -> Values {
        let mut v = self.clone();
        if v.source == 1 || v.media == CD_DVD_MEDIA {
            v.size = 0;
        }
        if v.size >= FIRST_DOCUMENT_SIZE {
            v.reduce = true;
        }
        if v.color_mode == VIVID {
            v.gamma = 1;
        }
        if !v.borderless_allowed() {
            v.borderless = false;
        }
        v
    }

    /// Borderless is only offered when the source, type and size allow it
    /// (PPD UIConstraints, SETTINGS.md §1).
    pub fn borderless_allowed(&self) -> bool {
        SOURCES[self.source].borderless && MEDIA[self.media].borderless && SIZES[self.size].borderless
    }

    /// Read from `lpoptions -p Q -l` (PPD options, current value marked `*`)
    /// and `lpoptions -p Q` (destination options such as `copies`).
    pub fn parse(ppd_listing: &str, dest: &str) -> Values {
        let ppd = parse_ppd_listing(ppd_listing);
        let opts = parse_dest_options(dest);
        let mut v = Values::default();
        let get = |k: &str| ppd.get(k).map(String::as_str);
        let pick = |list: &[Choice], k: &str, d: usize| get(k).and_then(|x| index_of(list, x)).unwrap_or(d);
        v.quality = get("PrintQuality").and_then(|x| QUALITY.iter().position(|q| *q == x)).unwrap_or(v.quality);
        v.source = pick(SOURCES, "InputSlot", v.source);
        v.media = pick(MEDIA, "MediaType", v.media);
        v.size = pick(SIZES, "PageSize", v.size);
        v.borderless = get("Borderless") == Some("On");
        v.grayscale = get("Color") == Some("Grayscale");
        v.color_mode = pick(COLOR_MODES, "CorrectionColor", v.color_mode);
        v.gamma = pick(GAMMAS, "GammaValue", v.gamma);
        match get("ReduceEnlarge") {
            Some("ByOutputPaperSize") => v.reduce = true,
            Some("ByPercentage") => (v.reduce, v.by_percent) = (true, true),
            _ => {}
        }
        v.output_size = get("OutputPaper").and_then(|x| index_of(output_sizes(), x)).unwrap_or(0);
        v.scale = get("ScaleRatio").and_then(|x| x.parse().ok()).unwrap_or(100);
        if let Some(i) = get("PosterPrinting").and_then(|x| POSTERS.iter().position(|p| p.0 == x)) {
            (v.multi_page, v.poster, v.poster_index) = (true, true, i);
        }
        v.rotate180 = get("Rotate180") == Some("On");
        v.mirror = get("MirrorImage") == Some("On");
        let level = |k: &str| get(k).and_then(|x| x.parse::<i32>().ok()).map_or(0, |n| n.clamp(-25, 25));
        v.brightness = level("BrightnessValue");
        v.contrast = level("ContrastValue");
        v.saturation = level("SaturationValue");
        v.cyan = level("CyanValue");
        v.magenta = level("MagentaValue");
        v.yellow = level("YellowValue");

        let o = |k: &str| opts.get(k).map(String::as_str);
        v.landscape = matches!(o("orientation-requested"), Some("4") | Some("5"));
        v.reverse = o("outputorder") == Some("reverse");
        let up = o("number-up").and_then(|n| n.parse::<u32>().ok());
        if let Some(i) = up.and_then(|n| PAGES_PER_SHEET.iter().position(|p| *p == n)).filter(|_| !v.poster) {
            (v.multi_page, v.pages_index) = (true, i);
        }
        v.page_borders = matches!(o("page-border"), Some(b) if b != "none");
        v.copies = o("copies").and_then(|n| n.parse().ok()).filter(|n| (1..=99).contains(n)).unwrap_or(1);
        v.collate = o("collate") == Some("true");
        v
    }

    /// Every option this window controls, as `key=value` pairs.
    pub fn to_options(&self) -> Vec<(String, String)> {
        let v = self.constrained();
        v.options()
    }

    fn options(&self) -> Vec<(String, String)> {
        let o: Vec<(&str, String)> = vec![
            ("PrintQuality", QUALITY[self.quality].into()),
            ("InputSlot", SOURCES[self.source].ppd.into()),
            ("MediaType", MEDIA[self.media].ppd.into()),
            ("PageSize", SIZES[self.size].ppd.into()),
            ("Borderless", if self.borderless { "On" } else { "Off" }.into()),
            ("Color", if self.grayscale { "Grayscale" } else { "Color" }.into()),
            ("CorrectionColor", COLOR_MODES[self.color_mode].ppd.into()),
            ("GammaValue", GAMMAS[self.gamma].ppd.into()),
            (
                "ReduceEnlarge",
                match (self.reduce, self.by_percent) {
                    (false, _) => "Off",
                    (true, false) => "ByOutputPaperSize",
                    (true, true) => "ByPercentage",
                }
                .into(),
            ),
            ("OutputPaper", output_sizes()[self.output_size].ppd.into()),
            ("ScaleRatio", self.scale.to_string()),
            ("PosterPrinting", if self.multi_page && self.poster { POSTERS[self.poster_index].0 } else { "Off" }.into()),
            ("Rotate180", if self.rotate180 { "On" } else { "Off" }.into()),
            ("MirrorImage", if self.mirror { "On" } else { "Off" }.into()),
            ("BrightnessValue", self.brightness.to_string()),
            ("ContrastValue", self.contrast.to_string()),
            ("SaturationValue", self.saturation.to_string()),
            ("CyanValue", self.cyan.to_string()),
            ("MagentaValue", self.magenta.to_string()),
            ("YellowValue", self.yellow.to_string()),
            ("orientation-requested", if self.landscape { "4" } else { "3" }.into()),
            ("outputorder", if self.reverse { "reverse" } else { "normal" }.into()),
            (
                "number-up",
                if self.multi_page && !self.poster { PAGES_PER_SHEET[self.pages_index] } else { 1 }.to_string(),
            ),
            ("page-border", if self.multi_page && !self.poster && self.page_borders { "single" } else { "none" }.into()),
            ("copies", self.copies.to_string()),
            ("collate", self.collate.to_string()),
        ];
        o.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }
}

/// `Key/Label: a *b c` lines -> Key -> b.
fn parse_ppd_listing(text: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for line in text.lines() {
        let Some((head, choices)) = line.split_once(": ") else { continue };
        let key = head.split('/').next().unwrap_or(head);
        if let Some(cur) = choices.split_whitespace().find_map(|w| w.strip_prefix('*')) {
            m.insert(key.to_string(), cur.to_string());
        }
    }
    m
}

/// `a=1 b='x y' c` -> {a: 1, b: x y, c: ""}.
fn parse_dest_options(text: &str) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    let mut chars = text.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none() {
            break;
        }
        let (mut key, mut val, mut in_val, mut quote) = (String::new(), String::new(), false, None);
        while let Some(&c) = chars.peek() {
            if quote.is_none() && c.is_whitespace() {
                break;
            }
            chars.next();
            match (c, quote) {
                ('=', None) if !in_val => in_val = true,
                ('\'' | '"', None) => quote = Some(c),
                (q, Some(open)) if q == open => quote = None,
                (c, _) if in_val => val.push(c),
                (c, _) => key.push(c),
            }
        }
        m.insert(key, val);
    }
    m
}

fn lpoptions(args: &[&str]) -> Result<String, String> {
    let o = Command::new("lpoptions").env("LC_ALL", "C").args(args).output().map_err(|e| format!("Cannot run lpoptions: {e}"))?;
    if !o.status.success() {
        return Err(format!("lpoptions {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// Current per-user values for `queue` (read-only queries to cupsd).
pub fn load(queue: &str) -> Result<Values, String> {
    Ok(Values::parse(&lpoptions(&["-p", queue, "-l"])?, &lpoptions(&["-p", queue])?))
}

/// The command that saves `v` for this user (written to ~/.cups/lpoptions).
pub fn save_args(queue: &str, v: &Values) -> Vec<String> {
    let mut args = vec!["-p".to_string(), queue.to_string()];
    for (k, val) in v.to_options() {
        args.push("-o".into());
        args.push(format!("{k}={val}"));
    }
    args
}

pub fn save(queue: &str, v: &Values) -> Result<(), String> {
    let args = save_args(queue, v);
    lpoptions(&args.iter().map(String::as_str).collect::<Vec<_>>()).map(|_| ())
}

/// One Printing Preset of the Shortcuts tab (E_FUIXG5P.XML, names from
/// E_IUIXG5P strings 4002-4013 for A4 regions). Only fields with a sure
/// mapping are applied: source, type, size, borderless, orientation. The
/// presets' QUALITY/SPEED and colour fields have no known PPD mapping.
pub struct Preset {
    pub name: &'static str,
    pub source: usize,
    pub media: usize,
    pub size: usize,
    pub borderless: bool,
    pub landscape: bool,
}

pub const PRESETS: &[Preset] = &[
    // ID 1: PaperSourceID 1, PaperTypeID 1 (plain), PaperSizeID 0 (A4/Letter), portrait
    Preset { name: "A4 Document - High Quality", source: 0, media: 0, size: 0, borderless: false, landscape: false },
    // ID 2: same paper, Duplex 2 (no Linux 2-sided printing)
    Preset { name: "A4 Document - Double Sided", source: 0, media: 0, size: 0, borderless: false, landscape: false },
    // ID 3/4: PaperTypeID 272 (Premium Glossy), PaperSizeID 285 (10 x 15 cm), borderless
    Preset { name: "10 x 15 cm Photo - Portrait", source: 0, media: 2, size: 1, borderless: true, landscape: false },
    Preset { name: "10 x 15 cm Photo - Landscape", source: 0, media: 2, size: 1, borderless: true, landscape: true },
    // ID 5: PaperSourceID 4 (CD/DVD), PaperTypeID 283 (CD/DVD), PaperSizeID 9 (A4)
    Preset { name: "Printable CD/DVD Printing", source: 1, media: 9, size: 0, borderless: false, landscape: false },
    // ID 6/7: Premium Glossy, A4, borderless
    Preset { name: "A4 Photo - Landscape", source: 0, media: 2, size: 0, borderless: true, landscape: true },
    Preset { name: "A4 Photo - Portrait", source: 0, media: 2, size: 0, borderless: true, landscape: false },
];

impl Preset {
    pub fn apply(&self, v: &mut Values) {
        (v.source, v.media, v.size, v.borderless, v.landscape) =
            (self.source, self.media, self.size, self.borderless, self.landscape);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: &str = "MediaType/Type: *PLAIN EULTRAGLOSSY EPREMGLOSS EPSGLOS EPENTRY EGCP EMATTE EPQIJ PHOTOSTICKERS CD_DVD CD_DVD_PREVIEW ENVELOPE
PrintQuality/Quality Option: Draft *Text TextImage Photo BestPhoto
PageSize/Size: Custom.WIDTHxHEIGHT *A4 4x6in 5x7in A6 A5 B5 3.5x5in 5x8in 8x10in 169widesize 100x148mm ENVELOPE_10 ENVELOPEDL ENVELOPEC6 Letter Legal A3 A3+ A2 B4 B3
InputSlot/Source: *Sheet CD_DVD
Color/Color: *Color Grayscale
Borderless/Borderless: *Off On
ReduceEnlarge/Reduce/Enlarge: *Off ByOutputPaperSize ByPercentage
OutputPaper/By Output Paper Size: *A4 4x6in 5x7in A6 A5 B5 3.5x5in 5x8in 8x10in 169widesize 100x148mm ENVELOPE_10 ENVELOPEDL ENVELOPEC6 Letter Legal
ScaleRatio/By Percentage: 10 99 *100 101 400
PosterPrinting/Poster Printing: *Off 2x1 2x2 3x3 4x4
Rotate180/Rotate 180 degrees: *Off On
MirrorImage/Mirror Image: *Off On
Watermark/Watermark: *None Confidential Draft Urgent Priority Hot DoNotCopy ForYourEyesOnly ReviewCopy TopSecret Duplicate
CorrectionColor/Color Mode: EPSONStandard *EPSONVivid AdobeRGB
GammaValue/Gamma: 1.8 *2.2
BrightnessValue/Brightness: -25 -5 *0 25
ContrastValue/Contrast: -25 *0 25
SaturationValue/Saturation: -25 *0 25
CyanValue/Cyan: -25 *0 25
MagentaValue/Magenta: -25 *0 25
YellowValue/Yellow: -25 *0 25
";
    const DEST: &str = "copies=1 finishings=3 number-up=1 printer-info=L800 printer-make-and-model='Epson L800 Series' printer-state=3";

    #[test]
    fn stock_queue_reads_as_ppd_defaults() {
        assert_eq!(Values::parse(LISTING, DEST), Values::default());
    }

    #[test]
    fn user_values_round_trip() {
        let v = Values {
            quality: 4,
            media: 2,
            size: 1,
            borderless: true,
            landscape: true,
            grayscale: true,
            reverse: true,
            color_mode: 0,
            reduce: true,
            by_percent: true,
            scale: 150,
            multi_page: true,
            pages_index: 1,
            page_borders: true,
            copies: 3,
            collate: true,
            mirror: true,
            brightness: -5,
            cyan: 25,
            yellow: -25,
            ..Values::default()
        };
        let opts = v.to_options();
        let listing: String = LISTING
            .lines()
            .map(|l| {
                let key = l.split('/').next().unwrap();
                match opts.iter().find(|(k, _)| k == key) {
                    Some((_, val)) => format!("{key}/x: *{val}\n"),
                    None => format!("{l}\n"),
                }
            })
            .collect();
        let dest: String = opts.iter().filter(|(k, _)| k.contains('-') || k == "copies" || k == "collate" || k == "outputorder").map(|(k, val)| format!("{k}={val} ")).collect();
        assert_eq!(Values::parse(&listing, &dest), v);
    }

    #[test]
    fn borderless_needs_a_borderless_paper() {
        let mut v = Values { borderless: true, ..Values::default() };
        assert!(!v.borderless_allowed()); // plain papers
        assert!(v.to_options().contains(&("Borderless".into(), "Off".into())));
        v.media = 2;
        assert!(v.borderless_allowed());
        assert!(v.to_options().contains(&("Borderless".into(), "On".into())));
    }

    #[test]
    fn constraints() {
        let v = Values { source: 1, size: 3, ..Values::default() }.constrained();
        assert_eq!(v.size, 0); // CD/DVD -> A4
        let v = Values { size: FIRST_DOCUMENT_SIZE, ..Values::default() }.constrained();
        assert!(v.reduce);
        let v = Values { color_mode: 1, gamma: 0, ..Values::default() }.constrained();
        assert_eq!(v.gamma, 1);
        assert_eq!(SIZES[FIRST_DOCUMENT_SIZE].ppd, "A3");
        assert_eq!(MEDIA[CD_DVD_MEDIA].ppd, "CD_DVD");
        assert_eq!(COLOR_MODES[VIVID].ppd, "EPSONVivid");
    }

    #[test]
    fn save_args_use_lpoptions_o() {
        let a = save_args("L800", &Values::default());
        assert_eq!(&a[..4], &["-p", "L800", "-o", "PrintQuality=Text"]);
        assert!(a.contains(&"orientation-requested=3".to_string()));
    }

    #[test]
    fn quoted_dest_values() {
        let m = parse_dest_options("a=1 b='x y' c");
        assert_eq!(m["b"], "x y");
        assert_eq!(m["c"], "");
    }

    #[test]
    fn tables_match_the_ppd() {
        let ppd = parse_ppd_listing(LISTING);
        assert_eq!(ppd["PageSize"], SIZES[0].ppd);
        let choices = |key: &str| -> Vec<String> {
            let line = LISTING.lines().find(|l| l.starts_with(&format!("{key}/"))).unwrap();
            line.split_once(": ").unwrap().1.split_whitespace().map(|w| w.trim_start_matches('*').to_string()).collect()
        };
        for (key, list) in [("InputSlot", SOURCES), ("MediaType", MEDIA), ("PageSize", SIZES), ("CorrectionColor", COLOR_MODES), ("GammaValue", GAMMAS)] {
            let have = choices(key);
            assert!(list.iter().all(|ch| have.iter().any(|h| h == ch.ppd)), "{key}");
        }
        let out = choices("OutputPaper");
        assert!(output_sizes().iter().all(|ch| out.iter().any(|h| h == ch.ppd)));
    }
}

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
pub const QUALITY: &[Choice] = &[
    c("Draft", "Draft", false),
    c("Text", "Text", false),
    c("TextImage", "Text & Image", false),
    c("Photo", "Photo", false),
    c("BestPhoto", "Best Photo", false),
];

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
    &SIZES[..FIRST_DOCUMENT_SIZE]
}

/// Paper Type entries listed for CD/DVD (SETTINGS-visibility.md §2.2: the
/// media whose source mask has the CD/DVD bit).
const CD_DVD_MEDIA_LIST: [usize; 7] = [1, 2, 3, 4, 5, 6, 9];

/// What the Source combo plus the Borderless radio select. Windows switches
/// to the hidden source "Sheet (Borderless)" (string 1542) for Borderless,
/// and each source lists its own types and sizes (SETTINGS-visibility.md §0, §2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feed {
    Sheet,
    Borderless,
    CdDvd,
}

impl Values {
    pub fn feed(&self) -> Feed {
        match (self.source, self.borderless) {
            (1, _) => Feed::CdDvd,
            (_, true) => Feed::Borderless,
            _ => Feed::Sheet,
        }
    }

    /// Paper Type combo: indices into `MEDIA`, in CFG order.
    pub fn media_list(&self) -> Vec<usize> {
        match self.feed() {
            Feed::Sheet => (0..MEDIA.len()).collect(),
            Feed::Borderless => (0..MEDIA.len()).filter(|&i| MEDIA[i].borderless).collect(),
            Feed::CdDvd => CD_DVD_MEDIA_LIST.to_vec(),
        }
    }

    /// Size combo: indices into `SIZES` (Sheet: all; Borderless: the seven
    /// borderless sizes; CD/DVD: A4 only).
    pub fn size_list(&self) -> Vec<usize> {
        match self.feed() {
            Feed::Sheet => (0..SIZES.len()).collect(),
            Feed::Borderless => (0..SIZES.len()).filter(|&i| SIZES[i].borderless).collect(),
            Feed::CdDvd => vec![0],
        }
    }

    /// Output Paper combo (Page Layout 3403): printable sizes; CD/DVD: A4.
    pub fn output_list(&self) -> Vec<usize> {
        match self.feed() {
            Feed::CdDvd => vec![0],
            _ => (0..output_sizes().len()).collect(),
        }
    }
}

/// Main quality radio -> print mode name per media (SETTINGS-visibility.md §2.3).
fn main_modes(media: usize) -> [&'static str; 5] {
    match media {
        0 => ["Fast Economy", "Normal", "Normal", "Fine", "Fine"],
        1 | 6 => ["Photo", "Photo", "Photo", "Photo", "Photo RPM (Max dpi)"],
        2..=5 => ["Draft", "Photo", "Photo", "Photo", "Photo RPM (Max dpi)"],
        10 => ["Normal", "Normal", "Normal", "Fine", "Fine"],
        _ => ["Photo"; 5],
    }
}

/// Advanced > Print Quality 3105 for a media type: (Windows label from
/// strings 1400-1430, the PPD `PrintQuality` index it is saved as). The PPD
/// has five qualities, so each mode is saved as the Main option that selects
/// it on Windows; "Economy" (plain papers) has no Main option and is saved
/// as Draft (approximate).
pub fn advanced_qualities(media: usize) -> &'static [(&'static str, usize)] {
    match media {
        0 => &[("Economy", 0), ("Fast Economy", 0), ("Normal", 1), ("Fine", 3)],
        1 | 6 => &[("Photo", 3), ("Photo RPM (Max dpi)", 4)],
        2..=5 => &[("Draft", 0), ("Photo", 3), ("Photo RPM (Max dpi)", 4)],
        10 => &[("Normal", 1), ("Fine", 3)],
        _ => &[("Photo", 3)],
    }
}

/// Which Advanced Print Quality entry shows for a Main quality.
pub fn advanced_quality_pos(media: usize, quality: usize) -> usize {
    let mode = main_modes(media)[quality.min(4)];
    let list = advanced_qualities(media);
    // "Fast Economy" comes after "Economy" on plain papers: match by label.
    list.iter().position(|(l, _)| *l == mode).unwrap_or(0)
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
    /// 1..=9999 (Page Layout 3427/3428)
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
        if v.source == 1 && !CD_DVD_MEDIA_LIST.contains(&v.media) {
            v.media = CD_DVD_MEDIA;
        }
        if v.source == 1 {
            // The Output Paper list for CD/DVD is A4 only (`output_list`).
            v.output_size = 0;
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

    /// Reduce/Enlarge counts only on the plain sheet feed (FUN_68f8d8e4);
    /// elsewhere it is greyed out, so it is neither saved nor shown as set.
    pub fn reduce_active(&self) -> bool {
        self.reduce && self.feed() == Feed::Sheet
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
        let mut v = Values::default();
        let get = |k: &str| ppd.get(k).map(String::as_str);
        let pick = |list: &[Choice], k: &str, d: usize| get(k).and_then(|x| index_of(list, x)).unwrap_or(d);
        v.quality = pick(QUALITY, "PrintQuality", v.quality);
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

        let std = Std::parse(dest);
        (v.landscape, v.reverse, v.page_borders, v.copies, v.collate) = (std.landscape, std.reverse, std.page_borders, std.copies, std.collate);
        if std.multi_page && !v.poster {
            (v.multi_page, v.pages_index) = (true, std.pages_index);
        }
        v
    }

    /// Every option this window controls, as `key=value` pairs, after the
    /// constraints.
    pub fn to_options(&self) -> Vec<(String, String)> {
        let s = &self.constrained();
        // Greyed-out controls are not saved; their PPD default goes instead.
        // Only the active Reduce/Enlarge method's value counts.
        let d = Values::default();
        let (reduce, output, scale) = match (s.reduce_active(), s.by_percent) {
            (false, _) => ("Off", d.output_size, d.scale),
            (true, false) => ("ByOutputPaperSize", s.output_size, d.scale),
            (true, true) => ("ByPercentage", d.output_size, s.scale),
        };
        let o: Vec<(&str, String)> = vec![
            ("PrintQuality", QUALITY[s.quality].ppd.into()),
            ("InputSlot", SOURCES[s.source].ppd.into()),
            ("MediaType", MEDIA[s.media].ppd.into()),
            ("PageSize", SIZES[s.size].ppd.into()),
            ("Borderless", if s.borderless { "On" } else { "Off" }.into()),
            ("Color", if s.grayscale { "Grayscale" } else { "Color" }.into()),
            ("CorrectionColor", COLOR_MODES[s.color_mode].ppd.into()),
            ("GammaValue", GAMMAS[s.gamma].ppd.into()),
            ("ReduceEnlarge", reduce.into()),
            ("OutputPaper", output_sizes()[output].ppd.into()),
            ("ScaleRatio", scale.to_string()),
            ("PosterPrinting", if s.multi_page && s.poster { POSTERS[s.poster_index].0 } else { "Off" }.into()),
            ("Rotate180", if s.rotate180 { "On" } else { "Off" }.into()),
            ("MirrorImage", if s.mirror { "On" } else { "Off" }.into()),
            ("BrightnessValue", s.brightness.to_string()),
            ("ContrastValue", s.contrast.to_string()),
            ("SaturationValue", s.saturation.to_string()),
            ("CyanValue", s.cyan.to_string()),
            ("MagentaValue", s.magenta.to_string()),
            ("YellowValue", s.yellow.to_string()),
        ];
        // The CUPS options; Poster Printing is not Pages per Sheet.
        let std = Std {
            landscape: s.landscape,
            reverse: s.reverse,
            multi_page: s.multi_page && !s.poster,
            pages_index: s.pages_index,
            page_borders: s.page_borders,
            copies: s.copies,
            collate: s.collate,
        };
        o.into_iter().map(|(k, v)| (k.to_string(), v)).chain(std.to_options()).collect()
    }
}

/// The CUPS job options every queue has (orientation, order, pages per
/// sheet, copies): the L800 sheet and the PPD sheets save them alike.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Std {
    pub landscape: bool,
    pub reverse: bool,
    pub multi_page: bool,
    /// Index into `PAGES_PER_SHEET`.
    pub pages_index: usize,
    pub page_borders: bool,
    pub copies: u32,
    pub collate: bool,
}

impl Default for Std {
    fn default() -> Self {
        Std { landscape: false, reverse: false, multi_page: false, pages_index: 0, page_borders: false, copies: 1, collate: false }
    }
}

impl Std {
    /// From `lpoptions -p Q` (destination options).
    pub fn parse(dest: &str) -> Std {
        let opts: BTreeMap<String, String> = parse_dest_options(dest);
        let o = |k: &str| opts.get(k).map(String::as_str);
        let up = o("number-up").and_then(|n| n.parse::<u32>().ok());
        let pages = up.and_then(|n| PAGES_PER_SHEET.iter().position(|p| *p == n));
        Std {
            landscape: matches!(o("orientation-requested"), Some("4") | Some("5")),
            reverse: o("outputorder") == Some("reverse"),
            multi_page: pages.is_some(),
            pages_index: pages.unwrap_or(0),
            page_borders: matches!(o("page-border"), Some(b) if b != "none"),
            copies: o("copies").and_then(|n| n.parse().ok()).filter(|n| (1..=9999).contains(n)).unwrap_or(1),
            collate: o("collate") == Some("true"),
        }
    }

    pub fn to_options(&self) -> Vec<(String, String)> {
        let up = if self.multi_page { PAGES_PER_SHEET[self.pages_index] } else { 1 };
        [
            ("orientation-requested", if self.landscape { "4" } else { "3" }.to_string()),
            ("outputorder", if self.reverse { "reverse" } else { "normal" }.to_string()),
            ("number-up", up.to_string()),
            ("page-border", if self.multi_page && self.page_borders { "single" } else { "none" }.to_string()),
            ("copies", self.copies.to_string()),
            ("collate", self.collate.to_string()),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
    }
}

/// `Key/Label: a *b c` lines -> Key -> b.
pub fn parse_ppd_listing(text: &str) -> BTreeMap<String, String> {
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
pub fn parse_dest_options(text: &str) -> BTreeMap<String, String> {
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
            // lpoptions escapes spaces and quotes with a backslash.
            let (c, escaped) = match c {
                '\\' => (chars.next().unwrap_or('\\'), true),
                c => (c, false),
            };
            match (c, quote) {
                (c, _) if escaped && in_val => val.push(c),
                (c, _) if escaped => key.push(c),
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

pub fn lpoptions(args: &[&str]) -> Result<String, String> {
    let o = Command::new("lpoptions").env("LC_ALL", "C").args(args).output().map_err(|e| format!("Cannot run lpoptions: {e}"))?;
    if !o.status.success() {
        return Err(format!("lpoptions {}: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// `lpoptions -p Q -l` (PPD options) and `lpoptions -p Q` (destination
/// options): read-only queries to cupsd.
pub fn load_raw(queue: &str) -> Result<(String, String), String> {
    Ok((lpoptions(&["-p", queue, "-l"])?, lpoptions(&["-p", queue])?))
}

/// The command that saves `options` for this user (written to
/// ~/.cups/lpoptions): `Values::to_options`, or a PPD sheet's options
/// followed by its `Std` ones.
pub fn save_args(queue: &str, options: impl IntoIterator<Item = (String, String)>) -> Vec<String> {
    let mut args = vec!["-p".to_string(), queue.to_string()];
    for (k, val) in options {
        args.push("-o".into());
        args.push(format!("{k}={val}"));
    }
    args
}

/// Run `lpoptions` with save arguments (`save_args`).
pub fn save(args: &[String]) -> Result<(), String> {
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
        // Borderless photo (Reduce/Enlarge is greyed out there, so left off),
        // and a plain sheet with each Reduce/Enlarge method.
        for v in [
            Values {
                quality: 4,
                media: 2,
                size: 1,
                borderless: true,
                landscape: true,
                grayscale: true,
                reverse: true,
                color_mode: 0,
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
            },
            Values { reduce: true, by_percent: true, scale: 150, rotate180: true, ..Values::default() },
            Values { reduce: true, output_size: 14, ..Values::default() },
        ] {
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
    fn lists_follow_the_source() {
        let sheet = Values::default();
        assert_eq!(sheet.media_list().len(), 11);
        assert_eq!(sheet.size_list().len(), SIZES.len());
        let bl = Values { media: 2, borderless: true, ..Values::default() };
        assert_eq!(bl.feed(), Feed::Borderless);
        assert_eq!(bl.media_list(), vec![1, 2, 3, 4, 5, 6, 7]);
        let sizes: Vec<&str> = bl.size_list().iter().map(|&i| SIZES[i].ppd).collect();
        assert_eq!(sizes, ["A4", "4x6in", "5x7in", "3.5x5in", "169widesize", "100x148mm", "Letter"]);
        let cd = Values { source: 1, output_size: 5, ..Values::default() }.constrained();
        assert_eq!(cd.size_list(), vec![0]);
        assert_eq!(cd.output_size, 0);
        assert_eq!(cd.media, CD_DVD_MEDIA);
        assert!(cd.media_list().contains(&cd.media));
    }

    #[test]
    fn advanced_quality_lists() {
        assert_eq!(advanced_qualities(0).len(), 4);
        assert_eq!(advanced_qualities(0)[advanced_quality_pos(0, 1)].0, "Normal"); // Text
        assert_eq!(advanced_qualities(0)[advanced_quality_pos(0, 0)].0, "Fast Economy"); // Draft
        assert_eq!(advanced_qualities(2)[advanced_quality_pos(2, 4)].0, "Photo RPM (Max dpi)");
        assert_eq!(advanced_qualities(9), &[("Photo", 3)]);
    }

    /// The `-o` pairs saved for some everyday combinations, and the invalid
    /// ones the constraints turn into valid ones.
    #[test]
    fn combinations_map_to_lpoptions() {
        let d = Values::default;
        let check = |v: Values, want: &[(&str, &str)]| {
            let opts = v.to_options();
            for &(k, val) in want {
                assert!(opts.contains(&(k.into(), val.into())), "{k}={val} not in {opts:?}");
            }
        };
        // Borderless photo on A4 Premium Glossy, Best Photo.
        check(
            Values { media: 2, borderless: true, quality: 4, ..d() },
            &[("InputSlot", "Sheet"), ("MediaType", "EPREMGLOSS"), ("PageSize", "A4"), ("Borderless", "On"), ("PrintQuality", "BestPhoto")],
        );
        // Color Controls (dialog 4300).
        check(
            Values { color_mode: 0, gamma: 0, brightness: 10, contrast: -5, saturation: 25, cyan: -25, magenta: 3, yellow: -1, ..d() },
            &[
                ("CorrectionColor", "EPSONStandard"),
                ("GammaValue", "1.8"),
                ("BrightnessValue", "10"),
                ("ContrastValue", "-5"),
                ("SaturationValue", "25"),
                ("CyanValue", "-25"),
                ("MagentaValue", "3"),
                ("YellowValue", "-1"),
            ],
        );
        check(Values { grayscale: true, ..d() }, &[("Color", "Grayscale")]);
        // Reduce/Enlarge: only the active method's value is saved.
        check(
            Values { reduce: true, by_percent: true, scale: 150, output_size: 3, ..d() },
            &[("ReduceEnlarge", "ByPercentage"), ("ScaleRatio", "150"), ("OutputPaper", "A4")],
        );
        check(
            Values { reduce: true, scale: 150, output_size: 14, ..d() },
            &[("ReduceEnlarge", "ByOutputPaperSize"), ("OutputPaper", "Letter"), ("ScaleRatio", "100")],
        );
        // Greyed out (borderless, CD/DVD): Reduce/Enlarge is not saved.
        check(
            Values { media: 2, borderless: true, reduce: true, by_percent: true, scale: 150, ..d() },
            &[("ReduceEnlarge", "Off"), ("ScaleRatio", "100")],
        );
        // Invalid: borderless on plain paper, CD/DVD with Letter and plain
        // paper, EPSON Vivid with gamma 1.8, A3 without Reduce/Enlarge.
        check(Values { borderless: true, ..d() }, &[("Borderless", "Off")]);
        check(
            Values { source: 1, size: 14, media: 0, reduce: true, output_size: 5, ..d() },
            &[("InputSlot", "CD_DVD"), ("MediaType", "CD_DVD"), ("PageSize", "A4"), ("ReduceEnlarge", "Off"), ("OutputPaper", "A4")],
        );
        check(Values { color_mode: 1, gamma: 0, ..d() }, &[("CorrectionColor", "EPSONVivid"), ("GammaValue", "2.2")]);
        check(Values { size: FIRST_DOCUMENT_SIZE, ..d() }, &[("PageSize", "A3"), ("ReduceEnlarge", "ByOutputPaperSize")]);
    }

    #[test]
    fn save_args_use_lpoptions_o() {
        let a = save_args("L800", Values::default().to_options());
        assert_eq!(&a[..4], &["-p", "L800", "-o", "PrintQuality=Text"]);
        assert!(a.contains(&"orientation-requested=3".to_string()));
    }

    #[test]
    fn quoted_dest_values() {
        let m = parse_dest_options("a=1 b='x y' c");
        assert_eq!(m["b"], "x y");
        assert_eq!(m["c"], "");
        let m = parse_dest_options("printer-info=Bob\\'s\\ L800 copies=2");
        assert_eq!(m["printer-info"], "Bob's L800");
        assert_eq!(m["copies"], "2");
    }

    #[test]
    fn save_args_keep_each_value_one_argv_word() {
        // No shell: spaces, quotes and backslashes go to lpoptions verbatim.
        let opts = [("job-sheets", "a b"), ("printer-info", "Bob's \"L800\""), ("x", "c:\\d")].map(|(k, v)| (k.to_string(), v.to_string()));
        let a = save_args("L800 inst", opts);
        assert_eq!(a, ["-p", "L800 inst", "-o", "job-sheets=a b", "-o", "printer-info=Bob's \"L800\"", "-o", "x=c:\\d"]);
    }

    #[test]
    fn save_args_empty_options_only_name_the_queue() {
        assert_eq!(save_args("L800/photo", Vec::new()), ["-p", "L800/photo"]);
    }

    #[test]
    fn dest_values_quotes_and_escapes() {
        // lpoptions output: single-quoted values with \' and \\ escaped.
        let m = parse_dest_options(r#"a='it\'s' b='c:\\d' c="q" d=x\ y e="two words""#);
        assert_eq!(m["a"], "it's");
        assert_eq!(m["b"], "c:\\d");
        assert_eq!(m["c"], "q");
        assert_eq!(m["d"], "x y");
        assert_eq!(m["e"], "two words");
        assert_eq!(m.len(), 5);
    }

    #[test]
    fn dest_values_whitespace_and_empty() {
        assert!(parse_dest_options("").is_empty());
        assert!(parse_dest_options("  \n\t ").is_empty());
        let m = parse_dest_options("  copies=3\n collate=true  ");
        assert_eq!((m["copies"].as_str(), m["collate"].as_str()), ("3", "true"));
        // A later duplicate wins (lpoptions merges system then user values).
        assert_eq!(parse_dest_options("copies=2 copies=5")["copies"], "5");
        // An '=' inside a value stays in it.
        assert_eq!(parse_dest_options("k=a=b")["k"], "a=b");
    }

    #[test]
    fn std_round_trips_through_dest_text() {
        for s in [
            Std::default(),
            Std { landscape: true, reverse: true, multi_page: true, pages_index: PAGES_PER_SHEET.len() - 1, page_borders: true, copies: 9999, collate: true },
        ] {
            let text: Vec<String> = s.to_options().into_iter().map(|(k, v)| format!("{k}={v}")).collect();
            assert_eq!(Std::parse(&text.join(" ")), s);
        }
    }

    #[test]
    fn std_rejects_out_of_range_dest_values() {
        let s = Std::parse("copies=0 number-up=3 orientation-requested=6 page-border=none");
        assert_eq!(s, Std::default());
        assert_eq!(Std::parse("copies=10000").copies, 1);
        assert!(Std::parse("orientation-requested=5").landscape);
    }

    #[test]
    fn l800_values_round_trip_through_lpoptions_text() {
        // Save -> what `lpoptions -p Q -l` / `lpoptions -p Q` would show -> parse.
        let v = Values { quality: 3, grayscale: true, brightness: -25, yellow: 25, landscape: true, copies: 4, collate: true, ..Values::default() };
        let opts = v.to_options();
        let saved: BTreeMap<String, String> = opts.iter().cloned().collect();
        let listing: String = LISTING
            .lines()
            .map(|l| {
                let (head, choices) = l.split_once(": ").unwrap();
                let key = head.split('/').next().unwrap();
                let cs: Vec<String> = choices
                    .split_whitespace()
                    .map(|c| c.trim_start_matches('*'))
                    .map(|c| if saved.get(key).map(String::as_str) == Some(c) { format!("*{c}") } else { c.to_string() })
                    .collect();
                format!("{head}: {}\n", cs.join(" "))
            })
            .collect();
        let dest: Vec<String> = opts.iter().filter(|(k, _)| k.contains('-') || k == "copies" || k == "collate" || k == "outputorder").map(|(k, v)| format!("{k}={v}")).collect();
        assert_eq!(Values::parse(&listing, &dest.join(" ")), v.constrained());
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

/// Constraint and preset rules of the L800 settings model, checked over
/// every combination rather than single examples.
#[cfg(test)]
mod constraint_tests {
    use super::*;

    fn opt(v: &Values, key: &str) -> String {
        v.to_options().into_iter().find(|(k, _)| k == key).map(|(_, v)| v).unwrap()
    }

    #[test]
    fn borderless_only_on_borderless_media_and_sizes() {
        for (media, m) in MEDIA.iter().enumerate() {
            for (size, z) in SIZES.iter().enumerate() {
                let v = Values { media, size, borderless: true, ..Values::default() }.constrained();
                let want = m.borderless && z.borderless && media != CD_DVD_MEDIA;
                assert_eq!(v.borderless, want, "{} {}", m.ppd, z.ppd);
                assert_eq!(opt(&v, "Borderless"), if want { "On" } else { "Off" });
            }
        }
        // Never on the CD/DVD tray, whatever type or size was set.
        let cd = Values { source: 1, media: 2, size: 1, borderless: true, ..Values::default() };
        assert!(!cd.constrained().borderless);
    }

    #[test]
    fn cd_dvd_forces_a4() {
        for size in 0..SIZES.len() {
            let tray = Values { source: 1, size, output_size: 5, ..Values::default() }.constrained();
            assert_eq!((tray.size, tray.output_size), (0, 0));
            assert_eq!(opt(&tray, "PageSize"), "A4");
            let media = Values { media: CD_DVD_MEDIA, size, ..Values::default() }.constrained();
            assert_eq!(media.size, 0);
        }
    }

    #[test]
    fn document_sizes_need_reduce_enlarge() {
        for (i, ch) in SIZES.iter().enumerate() {
            let v = Values { size: i, ..Values::default() }.constrained();
            let doc = ["A3", "A3+", "A2", "B4", "B3"].contains(&ch.ppd);
            assert_eq!(v.reduce, doc, "{}", ch.ppd);
            assert_eq!(opt(&v, "ReduceEnlarge") != "Off", doc, "{}", ch.ppd);
            // Document sizes are never offered as output paper.
            assert_eq!(output_sizes().iter().any(|o| o.ppd == ch.ppd), !doc);
        }
    }

    #[test]
    fn gamma_18_not_with_vivid() {
        for (color_mode, c) in COLOR_MODES.iter().enumerate() {
            let v = Values { color_mode, gamma: 0, ..Values::default() };
            let want = if c.ppd == "EPSONVivid" { "2.2" } else { "1.8" };
            assert_eq!(opt(&v, "GammaValue"), want);
        }
    }

    /// The PPD has no quiet mode, so nothing for it is saved.
    #[test]
    fn quiet_mode_is_never_saved() {
        assert!(Values::default().to_options().iter().all(|(k, _)| !k.to_lowercase().contains("quiet")));
    }

    #[test]
    fn greyed_reduce_enlarge_saves_off() {
        for v in [
            Values { media: 2, borderless: true, reduce: true, output_size: 3, ..Values::default() },
            Values { source: 1, reduce: true, by_percent: true, scale: 250, ..Values::default() },
        ] {
            assert!(!v.constrained().reduce_active());
            assert_eq!(opt(&v, "ReduceEnlarge"), "Off");
            assert_eq!(opt(&v, "ScaleRatio"), "100");
            assert_eq!(opt(&v, "OutputPaper"), "A4");
        }
    }

    #[test]
    fn copies_out_of_range_read_as_one() {
        for (dest, want) in [("copies=0", 1), ("copies=1", 1), ("copies=9999", 9999), ("copies=10000", 1), ("copies=-3", 1), ("copies=x", 1)] {
            assert_eq!(Std::parse(dest).copies, want, "{dest}");
        }
    }

    #[test]
    fn presets_apply_only_their_fields() {
        let start = Values {
            quality: 4,
            grayscale: true,
            color_mode: 2,
            gamma: 0,
            reduce: true,
            by_percent: true,
            scale: 50,
            copies: 7,
            collate: true,
            mirror: true,
            brightness: 9,
            ..Values::default()
        };
        for p in PRESETS {
            let mut v = start.clone();
            p.apply(&mut v);
            assert_eq!((v.source, v.media, v.size, v.borderless, v.landscape), (p.source, p.media, p.size, p.borderless, p.landscape), "{}", p.name);
            let mut rest = v.clone();
            (rest.source, rest.media, rest.size, rest.borderless, rest.landscape) =
                (start.source, start.media, start.size, start.borderless, start.landscape);
            assert_eq!(rest, start, "{} changed other fields", p.name);
            // Every preset is already valid: the constraints leave it as is.
            let c = v.constrained();
            assert_eq!((c.source, c.media, c.size, c.borderless), (p.source, p.media, p.size, p.borderless), "{}", p.name);
        }
    }
}

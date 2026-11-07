//! Printing Preferences for any PPD but the L800's (which keeps the Windows
//! tables in settings.rs): the same Windows pages, driven by the queue's PPD
//! through `ppd::slot_map` (MULTI-MODEL-PLAN §3).
//!
//! The state is one complete, conflict-free PPD selection (`ppd::resolve`)
//! plus the CUPS options every queue has. escpr PPDs fold two controls
//! into others, so they are taken apart here and put back on save:
//! - quality is a MediaType suffix (`PLAIN_NORMAL`, `PMPHOTO_HIGH`): the
//!   Type combo lists the base media, the Quality radios pick the suffix;
//! - borderless is a `T` variant of PageSize (`TA4` of `A4`): the Size combo
//!   lists the base sizes, the Borderless radio picks the variant.
//!
//! A control whose slot the PPD lacks is greyed out; the options no slot
//! takes are listed in the Advanced "Printer Options..." dialog. Lists show
//! only the choices the PPD's constraints allow with the other values.

use crate::ppd::{Ppd, Selection, Slot, SlotMap};
pub use crate::settings::Std;

/// escpr quality suffixes, low to high; index = Main quality radio. The
/// fifth radio (Best Photo) has no level.
const LEVELS: [&str; 4] = ["SUPERDRAFT", "DRAFT", "NORMAL", "HIGH"];
pub const RADIOS: usize = 5;
/// The colour slider slots, in the order of `View::levels`.
pub const LEVEL_SLOTS: [Slot; 6] = [Slot::Brightness, Slot::Contrast, Slot::Saturation, Slot::Cyan, Slot::Magenta, Slot::Yellow];

#[derive(Clone, Debug, PartialEq, Eq)]
enum Quality {
    None,
    /// Radio i is choice i of this option (PrintQuality).
    Option(String),
    /// escpr: the radio is the MediaType suffix.
    InMedia,
}

/// A Paper Type entry: (quality radio, MediaType choice, level label).
/// Without `Quality::InMedia` it has one choice, at radio 0.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Media {
    label: String,
    choices: Vec<(usize, String, String)>,
}

/// A Size entry and its borderless `T` variant (escpr).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Size {
    label: String,
    plain: String,
    borderless: Option<String>,
}

pub struct Sheet {
    pub ppd: Ppd,
    pub map: SlotMap,
    quality: Quality,
    media: Vec<Media>,
    sizes: Vec<Size>,
}

/// A list control: the entries shown, as (index in the full list, label).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Field {
    pub items: Vec<(usize, String)>,
    /// Row of the current value in `items`.
    pub pos: usize,
    /// The PPD has the option behind it.
    pub present: bool,
}

impl Field {
    pub fn labels(&self) -> Vec<&str> {
        self.items.iter().map(|(_, l)| l.as_str()).collect()
    }
    /// The full-list index shown at `row`.
    pub fn index(&self, row: usize) -> Option<usize> {
        self.items.get(row).map(|(i, _)| *i)
    }
}

/// What the pages show for a selection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct View {
    /// Main quality radio; None: no quality option.
    pub quality: Option<usize>,
    pub quality_labels: Vec<String>,
    pub quality_enabled: Vec<bool>,
    /// Advanced Print Quality combo: (radio, label).
    pub adv_quality: Field,
    pub source: Field,
    pub media: Field,
    pub size: Field,
    pub borderless: bool,
    pub borderless_allowed: bool,
    /// None: no grayscale option.
    pub grayscale: Option<bool>,
    pub color_mode: Field,
    pub gamma: Field,
    /// Brightness, Contrast, Saturation, Cyan, Magenta, Yellow (None: missing).
    pub levels: [Option<i32>; 6],
    /// Printer Options dialog: (option keyword, label, its choices).
    pub leftovers: Vec<(String, String, Field)>,
}

/// One user action on the pages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    Quality(usize),
    /// Index into the Paper Type entries.
    Media(usize),
    /// Index into the Size entries.
    Size(usize),
    Borderless(bool),
    Grayscale(bool),
    /// An option set to a choice (Source, Color Mode, Gamma, leftovers).
    Choice(String, String),
    Level(Slot, i32),
}

/// The Windows Main quality labels, for radios the PPD has no level for.
fn windows_quality(r: usize) -> &'static str {
    crate::settings::QUALITY[r].label
}

/// `PLAIN_NORMAL` -> (`PLAIN`, radio 2).
fn split_level(keyword: &str) -> Option<(&str, usize)> {
    let (base, level) = keyword.rsplit_once('_')?;
    Some((base, LEVELS.iter().position(|l| *l == level)?))
}

/// The base label of labels like "plain papers-High" / "plain papers-Standard":
/// their common prefix, cut at its last '-'.
fn base_label(labels: &[&str]) -> String {
    let first = labels[0];
    let mut n = first.len();
    for l in &labels[1..] {
        n = first.bytes().zip(l.bytes()).take(n).take_while(|(a, b)| a == b).count();
    }
    // Bytes compared: labels that differ inside a UTF-8 character.
    while !first.is_char_boundary(n) {
        n -= 1;
    }
    let prefix = &first[..n];
    prefix.rfind('-').map_or(prefix, |i| &prefix[..i]).trim().to_string()
}

impl Sheet {
    pub fn new(ppd: Ppd) -> Sheet {
        let map = crate::ppd::slot_map(&ppd);
        let media_opt = map.slots.get(&Slot::MediaType).and_then(|b| ppd.option(&b.option));
        let quality = match (map.slots.get(&Slot::PrintQuality), media_opt) {
            (Some(b), _) => Quality::Option(b.option.clone()),
            (None, Some(m)) if !m.choices.is_empty() && m.choices.iter().all(|c| split_level(&c.keyword).is_some()) => Quality::InMedia,
            _ => Quality::None,
        };
        let mut media: Vec<Media> = Vec::new();
        let mut bases: Vec<&str> = Vec::new();
        for c in media_opt.map_or(&[][..], |m| &m.choices) {
            match (&quality, split_level(&c.keyword)) {
                (Quality::InMedia, Some((base, r))) => {
                    let i = bases.iter().position(|b| *b == base).unwrap_or_else(|| {
                        bases.push(base);
                        media.push(Media { label: String::new(), choices: vec![] });
                        media.len() - 1
                    });
                    media[i].choices.push((r, c.keyword.clone(), c.label.clone()));
                }
                _ => media.push(Media { label: c.label.clone(), choices: vec![(0, c.keyword.clone(), String::new())] }),
            }
        }
        if quality == Quality::InMedia {
            for m in &mut media {
                let labels: Vec<&str> = m.choices.iter().map(|c| c.2.as_str()).collect();
                m.label = base_label(&labels);
                let cut = m.label.len();
                for c in &mut m.choices {
                    c.2 = c.2.get(cut..).unwrap_or("").trim_start_matches('-').trim().to_string();
                }
                m.choices.sort_by_key(|c| c.0);
            }
        }
        let size_opt = map.slots.get(&Slot::PageSize).and_then(|b| ppd.option(&b.option));
        let size_choices = size_opt.map_or(&[][..], |o| &o.choices);
        let is_variant = |k: &str| k.strip_prefix('T').is_some_and(|b| size_choices.iter().any(|c| c.keyword == b));
        let sizes = size_choices
            .iter()
            .filter(|c| !is_variant(&c.keyword))
            .map(|c| {
                let t = format!("T{}", c.keyword);
                Size {
                    label: c.label.clone(),
                    plain: c.keyword.clone(),
                    borderless: size_choices.iter().any(|x| x.keyword == t).then_some(t),
                }
            })
            .collect();
        Sheet { ppd, map, quality, media, sizes }
    }

    fn option_of(&self, slot: Slot) -> Option<&str> {
        self.map.slots.get(&slot).map(|b| b.option.as_str())
    }

    fn allowed(&self, option: &str, sel: &Selection) -> Vec<String> {
        self.ppd.allowed_choices(option, sel).iter().map(|c| c.keyword.clone()).collect()
    }

    /// The PPD defaults, conflict-free.
    pub fn defaults(&self) -> Selection {
        self.ppd.resolve(&Selection::new(), None)
    }

    /// The Paper Type entry and radio of a MediaType choice.
    fn media_of(&self, keyword: &str) -> Option<(usize, usize)> {
        self.media.iter().enumerate().find_map(|(i, m)| m.choices.iter().find(|c| c.1 == keyword).map(|c| (i, c.0)))
    }

    fn size_of(&self, keyword: &str) -> Option<(usize, bool)> {
        self.sizes.iter().position(|s| s.plain == keyword).map(|i| (i, false)).or_else(|| {
            self.sizes.iter().position(|s| s.borderless.as_deref() == Some(keyword)).map(|i| (i, true))
        })
    }

    /// Borderless through a `Borderless` option: (on, off) choices.
    fn borderless_choices(&self) -> Option<(String, String)> {
        let o = self.ppd.option(self.option_of(Slot::Borderless)?)?;
        let find = |names: [&str; 2]| o.choices.iter().find(|c| names.contains(&c.keyword.as_str())).map(|c| c.keyword.clone());
        Some((find(["On", "True"])?, find(["Off", "False"])?))
    }

    /// Apply an edit and settle the constraints (the edited option wins).
    pub fn apply(&self, sel: &Selection, edit: &Edit) -> Selection {
        let mut s = sel.clone();
        let mut set = |o: &str, c: String| {
            s.insert(o.to_string(), c);
            Some(o.to_string())
        };
        let media_opt = self.option_of(Slot::MediaType).unwrap_or("MediaType");
        let size_opt = self.option_of(Slot::PageSize).unwrap_or("PageSize");
        let edited = match edit {
            Edit::Quality(r) => match &self.quality {
                Quality::Option(o) => self.ppd.option(o).and_then(|opt| opt.choices.get(*r)).and_then(|c| set(o, c.keyword.clone())),
                Quality::InMedia => {
                    let m = sel.get(media_opt).and_then(|k| self.media_of(k)).map(|(i, _)| &self.media[i]);
                    m.and_then(|m| m.choices.iter().find(|c| c.0 == *r)).and_then(|c| set(media_opt, c.1.clone()))
                }
                Quality::None => None,
            },
            Edit::Media(i) => {
                let Some(m) = self.media.get(*i) else { return sel.clone() };
                // Keep the quality if this paper has it, else the nearest allowed one.
                let want = sel.get(media_opt).and_then(|k| self.media_of(k)).map_or(2, |(_, r)| r);
                let allowed = self.allowed(media_opt, sel);
                let pick = m.choices.iter().min_by_key(|c| (!allowed.contains(&c.1), c.0.abs_diff(want), usize::MAX - c.0));
                pick.and_then(|c| set(media_opt, c.1.clone()))
            }
            Edit::Size(i) => {
                let Some(z) = self.sizes.get(*i) else { return sel.clone() };
                let bl = sel.get(size_opt).and_then(|k| self.size_of(k)).is_some_and(|(_, b)| b);
                set(size_opt, if bl { z.borderless.clone().unwrap_or(z.plain.clone()) } else { z.plain.clone() })
            }
            Edit::Borderless(on) => match self.borderless_choices() {
                Some((yes, no)) => set(self.option_of(Slot::Borderless).unwrap_or("Borderless"), if *on { yes } else { no }),
                None => {
                    let z = sel.get(size_opt).and_then(|k| self.size_of(k)).map(|(i, _)| &self.sizes[i]);
                    z.and_then(|z| set(size_opt, if *on { z.borderless.clone()? } else { z.plain.clone() }))
                }
            },
            Edit::Grayscale(on) => {
                let b = self.map.slots.get(&Slot::Grayscale);
                let o = b.and_then(|b| self.ppd.option(&b.option));
                let gray = b.and_then(|b| b.choice.clone());
                match (o, gray) {
                    (Some(o), Some(gray)) => {
                        let colour = o.default.clone().filter(|d| *d != gray).or_else(|| o.choices.iter().find(|c| c.keyword != gray).map(|c| c.keyword.clone()));
                        let c = if *on { Some(gray) } else { colour };
                        c.and_then(|c| set(&o.keyword, c))
                    }
                    _ => None,
                }
            }
            Edit::Choice(o, c) => set(o, c.clone()),
            Edit::Level(slot, v) => {
                let o = self.option_of(*slot).and_then(|o| self.ppd.option(o));
                let near = o.and_then(|o| {
                    o.choices.iter().filter_map(|c| Some((c.keyword.parse::<i32>().ok()?, c))).min_by_key(|(n, _)| n.abs_diff(*v)).map(|(_, c)| c.keyword.clone())
                });
                o.zip(near).and_then(|(o, c)| set(&o.keyword, c))
            }
        };
        self.ppd.resolve(&s, edited.as_deref())
    }

    /// A choice list as a Field: the allowed choices of `option`.
    fn choice_field(&self, option: Option<&str>, sel: &Selection) -> Field {
        let Some(o) = option.and_then(|o| self.ppd.option(o)) else { return Field::default() };
        let allowed = self.allowed(&o.keyword, sel);
        let items: Vec<(usize, String)> =
            o.choices.iter().enumerate().filter(|(_, c)| allowed.contains(&c.keyword)).map(|(i, c)| (i, c.label.clone())).collect();
        let cur = sel.get(&o.keyword).and_then(|k| o.choices.iter().position(|c| &c.keyword == k));
        Field { pos: items.iter().position(|(i, _)| Some(*i) == cur).unwrap_or(0), items, present: true }
    }

    pub fn view(&self, sel: &Selection) -> View {
        let mut v = View::default();
        let media_opt = self.option_of(Slot::MediaType);
        let size_opt = self.option_of(Slot::PageSize);
        // Paper Type: the entries with an allowed choice.
        let media_allowed = media_opt.map(|o| self.allowed(o, sel)).unwrap_or_default();
        let cur_media = media_opt.and_then(|o| sel.get(o)).and_then(|k| self.media_of(k));
        v.media.present = media_opt.is_some();
        for (i, m) in self.media.iter().enumerate() {
            if m.choices.iter().any(|c| media_allowed.contains(&c.1)) || cur_media.map(|c| c.0) == Some(i) {
                v.media.items.push((i, m.label.clone()));
            }
        }
        v.media.pos = v.media.items.iter().position(|(i, _)| cur_media.map(|c| c.0) == Some(*i)).unwrap_or(0);
        // Quality radios.
        v.quality_labels = (0..RADIOS).map(|r| windows_quality(r).to_string()).collect();
        v.quality_enabled = vec![false; RADIOS];
        match &self.quality {
            Quality::InMedia => {
                if let Some((i, r)) = cur_media {
                    v.quality = Some(r);
                    for c in &self.media[i].choices {
                        v.quality_labels[c.0] = c.2.clone();
                        v.quality_enabled[c.0] = media_allowed.contains(&c.1) || c.0 == r;
                    }
                }
            }
            Quality::Option(o) => {
                let allowed = self.allowed(o, sel);
                if let Some(opt) = self.ppd.option(o) {
                    for (r, c) in opt.choices.iter().take(RADIOS).enumerate() {
                        v.quality_labels[r] = c.label.clone();
                        v.quality_enabled[r] = allowed.contains(&c.keyword);
                    }
                    v.quality = sel.get(o).and_then(|k| opt.choices.iter().position(|c| &c.keyword == k));
                }
            }
            Quality::None => {}
        }
        v.adv_quality.present = v.quality.is_some();
        v.adv_quality.items = (0..RADIOS).filter(|&r| v.quality_enabled[r]).map(|r| (r, v.quality_labels[r].clone())).collect();
        v.adv_quality.pos = v.adv_quality.items.iter().position(|(r, _)| Some(*r) == v.quality).unwrap_or(0);
        // Size and borderless.
        let size_allowed = size_opt.map(|o| self.allowed(o, sel)).unwrap_or_default();
        let cur_size = size_opt.and_then(|o| sel.get(o)).and_then(|k| self.size_of(k));
        v.size.present = size_opt.is_some();
        for (i, z) in self.sizes.iter().enumerate() {
            let would = match (cur_size.is_some_and(|c| c.1), &z.borderless) {
                (true, Some(t)) => t,
                _ => &z.plain,
            };
            if size_allowed.contains(would) || cur_size.map(|c| c.0) == Some(i) {
                v.size.items.push((i, z.label.clone()));
            }
        }
        v.size.pos = v.size.items.iter().position(|(i, _)| cur_size.map(|c| c.0) == Some(*i)).unwrap_or(0);
        match self.borderless_choices() {
            Some((yes, _)) => {
                let o = self.option_of(Slot::Borderless).unwrap_or_default();
                v.borderless = sel.get(o) == Some(&yes);
                v.borderless_allowed = self.allowed(o, sel).contains(&yes);
            }
            None => {
                v.borderless = cur_size.is_some_and(|c| c.1);
                v.borderless_allowed = cur_size
                    .and_then(|(i, _)| self.sizes[i].borderless.as_ref())
                    .is_some_and(|t| size_allowed.contains(t) || v.borderless);
            }
        }
        // Source, grayscale, colour.
        v.source = self.choice_field(self.option_of(Slot::InputSlot), sel);
        v.grayscale = self.map.slots.get(&Slot::Grayscale).map(|b| sel.get(&b.option) == b.choice.as_ref());
        v.color_mode = self.choice_field(self.option_of(Slot::CorrectionColor), sel);
        v.gamma = self.choice_field(self.option_of(Slot::GammaValue), sel);
        for (n, slot) in LEVEL_SLOTS.iter().enumerate() {
            v.levels[n] = self.option_of(*slot).map(|o| sel.get(o).and_then(|c| c.parse().ok()).unwrap_or(0));
        }
        v.leftovers = self
            .map
            .leftovers
            .iter()
            .filter_map(|k| Some((k.clone(), self.ppd.option(k)?.label.clone(), self.choice_field(Some(k), sel))))
            .collect();
        v
    }

    /// Settings saved with `lpoptions -o`: every PPD option but PageRegion
    /// (CUPS sets it from PageSize).
    pub fn to_options(&self, sel: &Selection) -> Vec<(String, String)> {
        sel.iter().filter(|(k, _)| *k != "PageRegion").map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    /// From `lpoptions -p Q -l` (current values marked `*`), settled.
    pub fn read_listing(&self, listing: &str) -> Selection {
        let raw: Selection = crate::settings::parse_ppd_listing(listing).into_iter().collect();
        self.ppd.resolve(&raw, None)
    }
}

// The pages: the `Prefs` global shows a view, and a change on it is read
// back as an edit.

/// A Slint string list.
pub fn strings<S: AsRef<str>>(items: impl IntoIterator<Item = S>) -> slint::ModelRc<slint::SharedString> {
    slint::VecModel::from_slice(&items.into_iter().map(|s| s.as_ref().into()).collect::<Vec<slint::SharedString>>())
}

/// Show `v` on the pages.
pub fn show(p: &crate::Prefs, v: &View) {
    p.set_ppd_mode(true);
    p.set_quality(v.quality.map_or(-1, |r| r as i32));
    p.set_quality_labels(strings(&v.quality_labels));
    p.set_quality_enabled(slint::ModelRc::new(slint::VecModel::from(v.quality_enabled.clone())));
    p.set_has_quality(v.adv_quality.present);
    p.set_adv_quality_labels(strings(v.adv_quality.labels()));
    p.set_adv_quality_pos(v.adv_quality.pos as i32);
    p.set_has_source(v.source.present);
    p.set_source_labels(strings(v.source.labels()));
    p.set_source(v.source.pos as i32);
    p.set_has_media(v.media.present);
    p.set_media_labels(strings(v.media.labels()));
    p.set_media_pos(v.media.pos as i32);
    p.set_media(v.media.pos as i32);
    p.set_has_size(v.size.present);
    p.set_size_labels(strings(v.size.labels()));
    p.set_size_pos(v.size.pos as i32);
    p.set_size(v.size.pos as i32);
    p.set_borderless(v.borderless);
    p.set_ppd_borderless_allowed(v.borderless_allowed);
    p.set_has_grayscale(v.grayscale.is_some());
    p.set_grayscale(v.grayscale.unwrap_or(false));
    p.set_has_color_mode(v.color_mode.present);
    p.set_color_mode_labels(strings(v.color_mode.labels()));
    p.set_color_mode(v.color_mode.pos as i32);
    p.set_has_gamma(v.gamma.present);
    p.set_gamma_labels(strings(v.gamma.labels()));
    p.set_gamma(v.gamma.pos as i32);
    p.set_has_color_controls(v.color_mode.present || v.gamma.present || v.levels.iter().any(Option::is_some));
    let l = v.levels.map(|l| l.unwrap_or(0));
    p.set_brightness(l[0]);
    p.set_contrast(l[1]);
    p.set_saturation(l[2]);
    p.set_cyan(l[3]);
    p.set_magenta(l[4]);
    p.set_yellow(l[5]);
    p.set_has_printer_options(!v.leftovers.is_empty());
}

/// Show the CUPS options on the pages (Poster Printing is the L800's).
pub fn show_std(p: &crate::Prefs, s: &Std) {
    p.set_landscape(s.landscape);
    p.set_reverse(s.reverse);
    p.set_multi_page(s.multi_page);
    p.set_poster(false);
    p.set_pages_index(s.pages_index as i32);
    p.set_page_borders(s.page_borders);
    p.set_copies(s.copies as i32);
    p.set_collate(s.collate);
}

pub fn std_of(p: &crate::Prefs) -> Std {
    Std {
        landscape: p.get_landscape(),
        reverse: p.get_reverse(),
        multi_page: p.get_multi_page(),
        pages_index: usize::try_from(p.get_pages_index()).unwrap_or(0).min(crate::settings::PAGES_PER_SHEET.len() - 1),
        page_borders: p.get_page_borders(),
        copies: p.get_copies().clamp(1, 9999) as u32,
        collate: p.get_collate(),
    }
}

/// The Printer Options dialog rows.
pub fn option_rows(v: &View) -> Vec<crate::OptionRow> {
    v.leftovers
        .iter()
        .map(|(_, label, f)| crate::OptionRow { label: label.into(), choices: strings(f.labels()), index: f.pos as i32 })
        .collect()
}

impl Sheet {
    /// Row `row` of `field`, the list of the option behind `slot` (or named `option`).
    fn choice_edit(&self, option: Option<&str>, field: &Field, row: usize) -> Option<Edit> {
        let o = self.ppd.option(option?)?;
        Some(Edit::Choice(o.keyword.clone(), o.choices.get(field.index(row)?)?.keyword.clone()))
    }

    /// The edit behind a `Prefs.changed()`: the first page value that differs from `v`.
    pub fn edit_from(&self, p: &crate::Prefs, v: &View) -> Option<Edit> {
        let row = |i: i32| usize::try_from(i).unwrap_or(0);
        let q = p.get_quality();
        if q != v.quality.map_or(-1, |r| r as i32) && q >= 0 {
            return Some(Edit::Quality(q as usize));
        }
        if row(p.get_source()) != v.source.pos {
            return self.choice_edit(self.option_of(Slot::InputSlot), &v.source, row(p.get_source()));
        }
        if p.get_borderless() != v.borderless {
            return Some(Edit::Borderless(p.get_borderless()));
        }
        if v.grayscale.is_some_and(|g| g != p.get_grayscale()) {
            return Some(Edit::Grayscale(p.get_grayscale()));
        }
        if row(p.get_color_mode()) != v.color_mode.pos {
            return self.choice_edit(self.option_of(Slot::CorrectionColor), &v.color_mode, row(p.get_color_mode()));
        }
        if row(p.get_gamma()) != v.gamma.pos {
            return self.choice_edit(self.option_of(Slot::GammaValue), &v.gamma, row(p.get_gamma()));
        }
        None
    }

    /// A row picked in a list (`Prefs.pick`): "media", "size" or "quality".
    pub fn pick(&self, v: &View, list: &str, row: usize) -> Option<Edit> {
        match list {
            "media" => Some(Edit::Media(v.media.index(row)?)),
            "size" => Some(Edit::Size(v.size.index(row)?)),
            "quality" => Some(Edit::Quality(v.adv_quality.index(row)?)),
            _ => None,
        }
    }

    /// Row `row` of Printer Options entry `i`.
    pub fn option_pick(&self, v: &View, i: usize, row: usize) -> Option<Edit> {
        let (k, _, f) = v.leftovers.get(i)?;
        self.choice_edit(Some(k), f, row)
    }

    /// The edits of the Color Controls dialog's OK: the values that changed.
    pub fn color_edits(&self, v: &View, mode_row: usize, gamma_row: usize, levels: [i32; 6]) -> Vec<Edit> {
        let mut out = Vec::new();
        if v.color_mode.present && mode_row != v.color_mode.pos {
            out.extend(self.choice_edit(self.option_of(Slot::CorrectionColor), &v.color_mode, mode_row));
        }
        if v.gamma.present && gamma_row != v.gamma.pos {
            out.extend(self.choice_edit(self.option_of(Slot::GammaValue), &v.gamma, gamma_row));
        }
        for (n, slot) in LEVEL_SLOTS.iter().enumerate() {
            if v.levels[n].is_some_and(|l| l != levels[n]) {
                out.push(Edit::Level(*slot, levels[n]));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The escpr PPD whose NickName starts with `nick` ($ESCPR_PPD_DIR).
    fn escpr(nick: &str) -> Option<Sheet> {
        let dir = std::env::var("ESCPR_PPD_DIR").ok()?;
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
            if text.contains(&format!("*NickName: \"{nick} ")) {
                return Some(Sheet::new(crate::ppd::parse(&text).unwrap()));
            }
        }
        panic!("no escpr PPD for {nick}");
    }

    /// What `lpoptions -p Q -l` would list after saving `sel`.
    fn listing(sheet: &Sheet, sel: &Selection) -> String {
        sheet
            .ppd
            .options
            .iter()
            .map(|o| {
                let cur = sel.get(&o.keyword);
                let choices: Vec<String> =
                    o.choices.iter().map(|c| if Some(&c.keyword) == cur { format!("*{}", c.keyword) } else { c.keyword.clone() }).collect();
                format!("{}/{}: {}\n", o.keyword, o.label, choices.join(" "))
            })
            .collect()
    }

    /// Save, read back, and compare.
    fn round_trip(sheet: &Sheet, sel: &Selection) {
        let saved: Selection = sheet.to_options(sel).into_iter().collect();
        assert!(!saved.contains_key("PageRegion"));
        let back = sheet.read_listing(&listing(sheet, &saved));
        let mut want = sel.clone();
        want.remove("PageRegion");
        back.iter().filter(|(k, _)| *k != "PageRegion").for_each(|(k, v)| assert_eq!(Some(v), want.get(k), "{k}"));
        assert!(sheet.ppd.conflicts(sel).is_empty());
    }

    #[test]
    fn l805() {
        let Some(s) = escpr("EPSON L805 Series") else { return eprintln!("ESCPR_PPD_DIR unset: skipped") };
        let slots: Vec<Slot> = s.map.slots.keys().copied().collect();
        assert_eq!(slots, [Slot::MediaType, Slot::PageSize, Slot::Grayscale, Slot::Brightness, Slot::Contrast, Slot::Saturation]);
        assert!(s.map.leftovers.is_empty(), "{:?}", s.map.leftovers);
        assert_eq!(s.quality, Quality::InMedia);
        let d = s.defaults();
        let v = s.view(&d);
        // PLAIN_NORMAL: plain papers, radio 2 "Standard", radio 3 "High".
        assert_eq!(v.media.labels()[v.media.pos], "plain papers");
        assert_eq!(v.quality, Some(2));
        assert_eq!(v.quality_labels[2..4], ["Standard", "High"]);
        assert_eq!(v.quality_enabled, [false, false, true, true, false]);
        assert_eq!(v.adv_quality.labels(), ["Standard", "High"]);
        let labels = v.media.labels();
        assert!(labels.contains(&"CD/DVD") && labels.contains(&"Epson Premium Glossy"), "{labels:?}");
        assert_eq!(v.size.labels()[v.size.pos], "A4");
        assert!(!v.size.labels().iter().any(|l| l.contains("Borderless")));
        // Plain paper: no borderless (*UIConstraints *PageSize TA4 *MediaType PLAIN_NORMAL).
        assert!(!v.borderless && !v.borderless_allowed);
        assert_eq!(v.grayscale, Some(false));
        assert_eq!(v.levels, [Some(0), Some(0), Some(0), None, None, None]);
        assert!(!v.source.present && !v.color_mode.present && !v.gamma.present);

        // Premium Glossy, High, borderless 10 x 15, grayscale, brightness +5.
        let glossy = s.media.iter().position(|m| m.label == "Epson Premium Glossy").unwrap();
        let photo = s.sizes.iter().position(|z| z.plain == "4X6FULL").unwrap();
        let mut sel = s.apply(&d, &Edit::Media(glossy));
        assert_eq!(sel["MediaType"], "PMPHOTO_NORMAL");
        for e in [Edit::Quality(3), Edit::Size(photo), Edit::Borderless(true), Edit::Grayscale(true), Edit::Level(Slot::Brightness, 5)] {
            sel = s.apply(&sel, &e);
        }
        assert_eq!((sel["MediaType"].as_str(), sel["PageSize"].as_str(), sel["Ink"].as_str(), sel["Brightness"].as_str()), ("PMPHOTO_HIGH", "T4X6FULL", "MONO", "5"));
        let v = s.view(&sel);
        assert!(v.borderless && v.borderless_allowed);
        assert_eq!(v.size.labels()[v.size.pos], "10 x 15 cm (4 x 6 in)");
        // Borderless lists only the papers that allow it.
        assert!(!v.media.labels().contains(&"plain papers"));
        round_trip(&s, &sel);
        // Back to plain paper: borderless goes, the quality stays.
        sel = s.apply(&s.apply(&sel, &Edit::Borderless(false)), &Edit::Media(0));
        assert_eq!((sel["MediaType"].as_str(), sel["PageSize"].as_str()), ("PLAIN_HIGH", "4X6FULL"));
        round_trip(&s, &sel);
        let args = crate::settings::save_args("L805", s.to_options(&sel).into_iter().chain(Std::default().to_options()));
        assert!(args.contains(&"MediaType=PLAIN_HIGH".to_string()) && args.contains(&"copies=1".to_string()), "{args:?}");
    }

    #[test]
    fn et2750() {
        let Some(s) = escpr("EPSON ET-2750 Series") else { return eprintln!("ESCPR_PPD_DIR unset: skipped") };
        assert_eq!(s.map.leftovers, ["Duplex"]);
        let d = s.defaults();
        let glossy = s.media.iter().position(|m| m.label == "Epson Premium Glossy").unwrap();
        let sel = s.apply(&d, &Edit::Media(glossy));
        let v = s.view(&sel);
        // DRAFT "Standard", NORMAL "High", HIGH "Photo RPM".
        assert_eq!(v.quality_labels[1..4], ["Standard", "High", "Photo RPM"]);
        assert_eq!(v.quality, Some(2));
        let sel = s.apply(&sel, &Edit::Quality(1));
        assert_eq!(sel["MediaType"], "PMPHOTO_DRAFT");
        // Duplex: a leftover with its allowed choices.
        let sel = s.apply(&s.apply(&d, &Edit::Media(0)), &Edit::Choice("Duplex".into(), "DuplexNoTumble".into()));
        assert_eq!(sel["Duplex"], "DuplexNoTumble");
        let v = s.view(&sel);
        let (k, label, f) = &v.leftovers[0];
        assert_eq!((k.as_str(), label.as_str()), ("Duplex", "Duplex Tumble"));
        assert_eq!(f.labels()[f.pos], "2-sided long edge");
        round_trip(&s, &sel);
    }

    #[test]
    fn wf4630() {
        let Some(s) = escpr("EPSON WF-4630 Series") else { return eprintln!("ESCPR_PPD_DIR unset: skipped") };
        assert!(s.map.slots.contains_key(&Slot::InputSlot));
        assert_eq!(s.map.leftovers, ["Duplex"]);
        let d = s.defaults();
        let v = s.view(&d);
        assert_eq!(v.source.labels()[v.source.pos], "Auto Select");
        assert_eq!(v.quality_labels[..4], ["Draft", "Standard", "Standard-Vivid", "High"]);
        assert_eq!(v.levels, [None; 6]);
        // The sizes have no borderless variants.
        assert!(!v.borderless_allowed);
        let cassette = s.ppd.option("InputSlot").unwrap().choices.iter().find(|c| c.keyword == "Cassette1").unwrap().keyword.clone();
        let mut sel = s.apply(&d, &Edit::Choice("InputSlot".into(), cassette));
        sel = s.apply(&sel, &Edit::Quality(0));
        assert_eq!((sel["InputSlot"].as_str(), sel["MediaType"].as_str()), ("Cassette1", "PLAIN_SUPERDRAFT"));
        // Every source lists only the media it allows, and the current one.
        let v = s.view(&sel);
        assert!(v.media.items.iter().any(|(i, _)| *i == 0));
        round_trip(&s, &sel);
    }

    /// Every escpr PPD: the defaults show, and each listed paper, size and
    /// quality can be picked and saved without a conflict.
    #[test]
    fn escpr_corpus_edits() {
        let Ok(dir) = std::env::var("ESCPR_PPD_DIR") else { return eprintln!("ESCPR_PPD_DIR unset: skipped") };
        let mut n = 0;
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            if e.path().extension().is_none_or(|x| x != "ppd") {
                continue;
            }
            let s = Sheet::new(crate::ppd::parse(&std::fs::read_to_string(e.path()).unwrap()).unwrap());
            let d = s.defaults();
            let v = s.view(&d);
            assert_eq!(s.quality, Quality::InMedia, "{:?}", e.path());
            for (i, _) in &v.media.items {
                let sel = s.apply(&d, &Edit::Media(*i));
                assert!(s.ppd.conflicts(&sel).is_empty());
                assert_eq!(s.view(&sel).media.index(s.view(&sel).media.pos), Some(*i), "{:?} media {i}", e.path());
            }
            for (i, _) in &v.size.items {
                let sel = s.apply(&d, &Edit::Size(*i));
                assert_eq!(s.view(&sel).size.index(s.view(&sel).size.pos), Some(*i), "{:?} size {i}", e.path());
            }
            n += 1;
        }
        assert!(n > 600);
    }

    /// Every escpr PPD: the defaults are complete and conflict-free, the
    /// Type/Quality and Size/Borderless split gives back each MediaType and
    /// PageSize choice exactly once, and saving then reading the listing
    /// returns the same selection.
    #[test]
    fn escpr_corpus_decomposition() {
        let Ok(dir) = std::env::var("ESCPR_PPD_DIR") else { return eprintln!("ESCPR_PPD_DIR unset: skipped") };
        let (mut n, mut media_n, mut size_n) = (0, 0, 0);
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "ppd") {
                continue;
            }
            let s = Sheet::new(crate::ppd::parse(&std::fs::read_to_string(&path).unwrap()).unwrap());
            let d = s.defaults();
            assert!(s.ppd.conflicts(&d).is_empty(), "{path:?} defaults conflict");
            for o in &s.ppd.options {
                assert!(o.choice(&d[&o.keyword]).is_some(), "{path:?} default {}", o.keyword);
            }
            // MediaType: each choice in one entry, at its own radio; flattened back in PPD order.
            let media = s.ppd.option(s.option_of(Slot::MediaType).unwrap()).unwrap();
            let mut flat: Vec<&str> = s.media.iter().flat_map(|m| m.choices.iter().map(|c| c.1.as_str())).collect();
            for m in &s.media {
                let mut radios: Vec<usize> = m.choices.iter().map(|c| c.0).collect();
                radios.dedup();
                assert_eq!(radios.len(), m.choices.len(), "{path:?} {} radios", m.label);
                assert!(!m.label.is_empty(), "{path:?} empty media label");
            }
            for c in &media.choices {
                let (i, r) = s.media_of(&c.keyword).unwrap_or_else(|| panic!("{path:?} lost {}", c.keyword));
                assert!(s.media[i].choices.iter().any(|x| x.0 == r && x.1 == c.keyword));
                media_n += 1;
            }
            let mut want: Vec<&str> = media.choices.iter().map(|c| c.keyword.as_str()).collect();
            flat.sort_unstable();
            want.sort_unstable();
            assert_eq!(flat, want, "{path:?} MediaType");
            // PageSize: plain sizes plus T variants, each exactly once.
            let size = s.ppd.option(s.option_of(Slot::PageSize).unwrap()).unwrap();
            let mut flat: Vec<&str> = s.sizes.iter().flat_map(|z| std::iter::once(z.plain.as_str()).chain(z.borderless.as_deref())).collect();
            for c in &size.choices {
                let (i, b) = s.size_of(&c.keyword).unwrap_or_else(|| panic!("{path:?} lost {}", c.keyword));
                assert_eq!(if b { s.sizes[i].borderless.as_deref() } else { Some(s.sizes[i].plain.as_str()) }, Some(c.keyword.as_str()));
                size_n += 1;
            }
            let mut want: Vec<&str> = size.choices.iter().map(|c| c.keyword.as_str()).collect();
            flat.sort_unstable();
            want.sort_unstable();
            assert_eq!(flat, want, "{path:?} PageSize");
            // Save -> load is the identity, for the defaults and each paper
            // (other values move to fit it). A paper that no size allows is
            // never listed, so it is skipped.
            let mut sels = vec![d.clone()];
            sels.extend((0..s.media.len()).map(|i| s.apply(&d, &Edit::Media(i))).filter(|sel| s.ppd.conflicts(sel).is_empty()));
            eprintln!("{path:?}: {} of {} papers", sels.len() - 1, s.media.len());
            for sel in &sels {
                let saved: Selection = s.to_options(sel).into_iter().collect();
                let mut back = s.read_listing(&listing(&s, &saved));
                back.remove("PageRegion");
                assert_eq!(back, saved, "{path:?} round trip");
            }
            n += 1;
        }
        eprintln!("{n} PPDs, {media_n} MediaType and {size_n} PageSize choices");
        assert!(n > 600);
    }

    #[test]
    fn base_labels() {
        assert_eq!(base_label(&["plain papers-High", "plain papers-Standard-Vivid", "plain papers-Standard"]), "plain papers");
        assert_eq!(base_label(&["X-Standard-Vivid", "X-Standard"]), "X");
        assert_eq!(base_label(&["Thick-Paper-High", "Thick-Paper-Standard"]), "Thick-Paper");
        assert_eq!(base_label(&["CD/DVD-Standard"]), "CD/DVD");
        // é and è share their first byte: no cut inside the character.
        assert_eq!(base_label(&["Papier-é", "Papier-è"]), "Papier");
        assert_eq!(split_level("PLAIN_SUPERDRAFT"), Some(("PLAIN", 0)));
        assert_eq!(split_level("CD_DVD"), None);
    }

    #[test]
    fn std_round_trip() {
        let s = Std { landscape: true, reverse: true, multi_page: true, pages_index: 1, page_borders: true, copies: 3, collate: true };
        let dest: String = s.to_options().iter().map(|(k, v)| format!("{k}={v} ")).collect();
        assert_eq!(Std::parse(&dest), s);
        assert_eq!(Std::parse("copies=1 number-up=1"), Std::default());
    }

    /// A mini escpr-like PPD: quality as MediaType suffix, borderless as `T`
    /// PageSize, Ink=MONO grayscale, a Duplex leftover, UTF-8 labels.
    const MINI: &str = r#"*PPD-Adobe: "4.3"
*NickName: "EPSON Mini Series, epson-escpr"
*OpenUI *PageSize/Paper Size: PickOne
*DefaultPageSize: A4
*PageSize A4/A4: ""
*PageSize TA4/A4 (Borderless): ""
*PageSize LETTER/Letter: ""
*CloseUI: *PageSize
*OpenUI *PageRegion/Paper Region: PickOne
*DefaultPageRegion: A4
*PageRegion A4/A4: ""
*PageRegion TA4/A4 (Borderless): ""
*PageRegion LETTER/Letter: ""
*CloseUI: *PageRegion
*OpenUI *MediaType/Media Type: PickOne
*DefaultMediaType: PLAIN_NORMAL
*MediaType PLAIN_DRAFT/plain papers-Draft: ""
*MediaType PLAIN_NORMAL/plain papers-Standard: ""
*MediaType PLAIN_HIGH/plain papers-High: ""
*MediaType PHOTO_NORMAL/Papier-é: ""
*MediaType PHOTO_HIGH/Papier-è: ""
*MediaType JP_NORMAL/写真用紙-標準: ""
*CloseUI: *MediaType
*OpenUI *Ink/Ink: PickOne
*DefaultInk: COLOR
*Ink COLOR/Color: ""
*Ink MONO/Black: ""
*CloseUI: *Ink
*OpenUI *Duplex/Duplex Tumble: PickOne
*DefaultDuplex: None
*Duplex None/Off: ""
*Duplex DuplexNoTumble/2-sided long edge: ""
*CloseUI: *Duplex
*UIConstraints: *PageSize TA4 *MediaType PLAIN_DRAFT
*UIConstraints: *MediaType PLAIN_DRAFT *PageSize TA4
*UIConstraints: *PageSize TA4 *MediaType PLAIN_NORMAL
*UIConstraints: *MediaType PLAIN_NORMAL *PageSize TA4
*UIConstraints: *PageSize TA4 *MediaType PLAIN_HIGH
*UIConstraints: *MediaType PLAIN_HIGH *PageSize TA4
"#;

    fn mini() -> Sheet {
        Sheet::new(crate::ppd::parse(MINI).unwrap())
    }

    #[test]
    fn mini_media_quality_decomposed() {
        let s = mini();
        assert_eq!(s.quality, Quality::InMedia);
        let labels: Vec<&str> = s.media.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(labels, ["plain papers", "Papier", "写真用紙"]);
        let radios = |i: usize| s.media[i].choices.iter().map(|c| (c.0, c.2.as_str())).collect::<Vec<_>>();
        assert_eq!(radios(0), [(1, "Draft"), (2, "Standard"), (3, "High")]);
        assert_eq!(radios(1), [(2, "é"), (3, "è")]);
        assert_eq!(radios(2), [(2, "標準")]);
        let v = s.view(&s.defaults());
        assert_eq!((v.media.labels()[v.media.pos], v.quality), ("plain papers", Some(2)));
        assert_eq!(v.quality_enabled, [false, true, true, true, false]);
        assert_eq!(v.quality_labels[1..4], ["Draft", "Standard", "High"]);
        assert_eq!(v.adv_quality.labels(), ["Draft", "Standard", "High"]);
    }

    #[test]
    fn mini_quality_recomposes_exact_keyword() {
        let s = mini();
        let d = s.defaults();
        for (r, want) in [(1, "PLAIN_DRAFT"), (2, "PLAIN_NORMAL"), (3, "PLAIN_HIGH")] {
            let sel = s.apply(&d, &Edit::Quality(r));
            assert_eq!(sel["MediaType"], want);
            assert_eq!(s.view(&sel).quality, Some(r));
            round_trip(&s, &sel);
        }
        // A radio the paper lacks changes nothing.
        assert_eq!(s.apply(&d, &Edit::Quality(0))["MediaType"], "PLAIN_NORMAL");
        // Paper change keeps the radio where it can, else the nearest.
        let high = s.apply(&d, &Edit::Quality(3));
        assert_eq!(s.apply(&high, &Edit::Media(1))["MediaType"], "PHOTO_HIGH");
        let draft = s.apply(&d, &Edit::Quality(1));
        assert_eq!(s.apply(&draft, &Edit::Media(1))["MediaType"], "PHOTO_NORMAL");
        assert_eq!(s.apply(&draft, &Edit::Media(2))["MediaType"], "JP_NORMAL");
        let back = s.apply(&s.apply(&draft, &Edit::Media(1)), &Edit::Media(0));
        assert_eq!(back["MediaType"], "PLAIN_NORMAL");
    }

    #[test]
    fn mini_borderless_is_t_pagesize() {
        let s = mini();
        let sizes: Vec<_> = s.sizes.iter().map(|z| (z.plain.as_str(), z.borderless.as_deref())).collect();
        assert_eq!(sizes, [("A4", Some("TA4")), ("LETTER", None)]);
        let d = s.defaults();
        let v = s.view(&d);
        assert_eq!(v.size.labels(), ["A4", "Letter"]);
        // Plain paper forbids TA4.
        assert!(!v.borderless && !v.borderless_allowed);
        let photo = s.apply(&d, &Edit::Media(1));
        assert!(s.view(&photo).borderless_allowed);
        let bl = s.apply(&photo, &Edit::Borderless(true));
        assert_eq!(bl["PageSize"], "TA4");
        let v = s.view(&bl);
        assert!(v.borderless && v.borderless_allowed);
        assert_eq!(v.size.labels()[v.size.pos], "A4");
        assert!(!v.media.labels().contains(&"plain papers"), "{:?}", v.media.labels());
        round_trip(&s, &bl);
        // Letter has no variant: borderless drops and is not allowed.
        let letter = s.apply(&bl, &Edit::Size(1));
        assert_eq!(letter["PageSize"], "LETTER");
        let v = s.view(&letter);
        assert!(!v.borderless && !v.borderless_allowed);
        assert_eq!(s.apply(&letter, &Edit::Borderless(true))["PageSize"], "LETTER");
        // Back to A4 plain; borderless off restores the plain size.
        assert_eq!(s.apply(&bl, &Edit::Borderless(false))["PageSize"], "A4");
        // Size edit keeps borderless when the new size has a variant.
        assert_eq!(s.apply(&bl, &Edit::Size(0))["PageSize"], "TA4");
        // Edited PageSize wins over a forbidding MediaType.
        let forced = s.apply(&s.apply(&bl, &Edit::Media(0)), &Edit::Borderless(true));
        assert!(s.ppd.conflicts(&forced).is_empty());
    }

    #[test]
    fn mini_grayscale_is_ink_mono() {
        let s = mini();
        let b = &s.map.slots[&Slot::Grayscale];
        assert_eq!((b.option.as_str(), b.choice.as_deref()), ("Ink", Some("MONO")));
        let d = s.defaults();
        assert_eq!(s.view(&d).grayscale, Some(false));
        let g = s.apply(&d, &Edit::Grayscale(true));
        assert_eq!(g["Ink"], "MONO");
        assert_eq!(s.view(&g).grayscale, Some(true));
        let c = s.apply(&g, &Edit::Grayscale(false));
        assert_eq!(c["Ink"], "COLOR");
        round_trip(&s, &g);
    }

    #[test]
    fn mini_missing_slots_greyed_and_leftovers() {
        let s = mini();
        assert_eq!(s.map.leftovers, ["Duplex"]);
        let v = s.view(&s.defaults());
        assert_eq!(v.levels, [None; 6]);
        assert!(!v.source.present && !v.color_mode.present && !v.gamma.present);
        assert!(v.source.items.is_empty());
        assert!(v.media.present && v.size.present);
        assert_eq!(v.leftovers.len(), 1);
        let (k, label, f) = &v.leftovers[0];
        assert_eq!((k.as_str(), label.as_str(), f.labels()[f.pos]), ("Duplex", "Duplex Tumble", "Off"));
        assert_eq!(f.labels(), ["Off", "2-sided long edge"]);
        let sel = s.apply(&s.defaults(), &Edit::Choice("Duplex".into(), "DuplexNoTumble".into()));
        let f = &s.view(&sel).leftovers[0].2;
        assert_eq!(f.labels()[f.pos], "2-sided long edge");
        assert!(!s.to_options(&sel).iter().any(|(k, _)| k == "PageRegion"));
        // No-op edits on absent slots.
        let d = s.defaults();
        assert_eq!(s.apply(&d, &Edit::Level(Slot::Cyan, 5)), d);
    }

    #[test]
    fn mini_without_suffix_or_slots() {
        // One MediaType choice without a level: no quality, labels kept whole.
        let text = MINI.replace("*MediaType JP_NORMAL/写真用紙-標準: \"\"", "*MediaType CD/CD/DVD-Ö: \"\"");
        let s = Sheet::new(crate::ppd::parse(&text).unwrap());
        assert_eq!(s.quality, Quality::None);
        assert_eq!(s.media.len(), 6);
        assert_eq!(s.media[3].label, "Papier-é");
        let v = s.view(&s.defaults());
        assert_eq!((v.quality, v.adv_quality.present), (None, false));
        assert_eq!(v.quality_enabled, [false; RADIOS]);
        assert_eq!(s.apply(&s.defaults(), &Edit::Quality(3)), s.defaults());
        // No MediaType / Ink at all: greyed, and nothing panics.
        let bare = "*PPD-Adobe: \"4.3\"\n*OpenUI *PageSize/Größe: PickOne\n*DefaultPageSize: A4\n*PageSize A4/Ä4: \"\"\n*CloseUI: *PageSize\n";
        let s = Sheet::new(crate::ppd::parse(bare).unwrap());
        let v = s.view(&s.defaults());
        assert!(!v.media.present && v.media.items.is_empty());
        assert_eq!((v.grayscale, v.quality), (None, None));
        assert_eq!(v.size.labels(), ["Ä4"]);
        assert!(!v.borderless_allowed);
        assert_eq!(s.apply(&s.defaults(), &Edit::Grayscale(true)), s.defaults());
        assert!(s.map.leftovers.is_empty());
    }

    #[test]
    fn base_label_multibyte_no_panic() {
        assert_eq!(base_label(&["写真用紙-高品質", "写真用紙-標準"]), "写真用紙");
        // 高 and 髙 share their first two bytes.
        assert_eq!(base_label(&["紙高", "紙髙"]), "紙");
        assert_eq!(base_label(&["ñ", "ö"]), "");
    }

}

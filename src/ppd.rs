//! PPD options for any CUPS queue: fetch the queue's PPD from cupsd, parse
//! its UI options and `*UIConstraints`, keep a selection conflict-free, and
//! map the options onto the Printing Preferences slots.
//!
//! Plan: MULTI-MODEL-PLAN §3 (M4). Parsed: `*OpenUI`/`*JCLOpenUI` blocks with
//! their choices and `*Default<Keyword>`, `*NickName`, `*1284DeviceID`, and
//! `*UIConstraints` in the 4-token form `*K1 C1 *K2 C2` (the only form in the
//! 617 escpr PPDs and the L800 PPD; other forms are skipped). Labels are the
//! base strings, which are English; localized `*xx.` lines are skipped (the
//! L800 `*en.` lines repeat the base strings). Hex substrings in labels
//! (`CD<2F>DVD`) are decoded. A selection holds one choice per option, as
//! `lpoptions -o key=value` stores it.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Option keyword -> choice keyword.
pub type Selection = BTreeMap<String, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ui {
    PickOne,
    Boolean,
    PickMany,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub keyword: String,
    /// The translation string, or the keyword when the PPD gives none.
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PpdOption {
    pub keyword: String,
    /// The translation string, or the keyword when the PPD gives none.
    pub label: String,
    pub ui: Ui,
    /// `*Default<Keyword>` as written (it may name no choice).
    pub default: Option<String>,
    pub choices: Vec<Choice>,
}

impl PpdOption {
    pub fn choice(&self, keyword: &str) -> Option<&Choice> {
        self.choices.iter().find(|c| c.keyword == keyword)
    }
}

/// `*UIConstraints: *K1 C1 *K2 C2`: K1=C1 and K2=C2 must not both be set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Constraint {
    pub a: (String, String),
    pub b: (String, String),
}

impl Constraint {
    /// The other side, when one side is `option=choice`.
    fn partner(&self, option: &str, choice: &str) -> Option<(&str, &str)> {
        if self.a.0 == option && self.a.1 == choice {
            Some((&self.b.0, &self.b.1))
        } else if self.b.0 == option && self.b.1 == choice {
            Some((&self.a.0, &self.a.1))
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ppd {
    pub nickname: Option<String>,
    pub device_id: Option<String>,
    /// In file order.
    pub options: Vec<PpdOption>,
    pub constraints: Vec<Constraint>,
}

/// The queue's PPD as cupsd serves it to any local user
/// (GET http://localhost:631/printers/<queue>.ppd; /etc/cups/ppd is root-only).
pub fn fetch(queue: &str) -> Result<String, String> {
    fetch_from("localhost:631", queue)
}

fn fetch_from(addr: &str, queue: &str) -> Result<String, String> {
    let path: String = queue
        .bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect();
    http_get(addr, &format!("/printers/{path}.ppd")).map_err(|e| format!("PPD of queue {queue}: {e}"))
}

fn http_get(addr: &str, path: &str) -> Result<String, String> {
    let mut s = TcpStream::connect(addr).map_err(|e| format!("cannot connect to CUPS at {addr}: {e}"))?;
    let timeout = Some(Duration::from_secs(10));
    s.set_read_timeout(timeout).and_then(|()| s.set_write_timeout(timeout)).map_err(|e| e.to_string())?;
    // HTTP/1.0: the body runs to EOF and is never chunked.
    let request = format!("GET {path} HTTP/1.0\r\nHost: localhost\r\n\r\n");
    s.write_all(request.as_bytes()).map_err(|e| format!("cannot send the request: {e}"))?;
    let mut reply = Vec::new();
    s.read_to_end(&mut reply).map_err(|e| format!("cannot read the reply: {e}"))?;
    let end = reply.windows(4).position(|w| w == b"\r\n\r\n").ok_or("malformed HTTP reply")?;
    let head = String::from_utf8_lossy(&reply[..end]);
    let status = head.lines().next().unwrap_or_default();
    match status.split_whitespace().nth(1) {
        Some("200") => {}
        Some("404") => return Err("CUPS has none (404 Not Found)".into()),
        _ => return Err(format!("CUPS replied {status:?}")),
    }
    let mut body = &reply[end + 4..];
    let length = head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case("content-length").then(|| v.trim().parse::<usize>().ok())?
    });
    if let Some(n) = length {
        body = body.get(..n).ok_or("truncated reply")?;
    }
    // UTF-8 in practice; else the declared *LanguageEncoding ISOLatin1, byte = char.
    Ok(String::from_utf8(body.to_vec()).unwrap_or_else(|e| e.into_bytes().iter().map(|&b| b as char).collect()))
}

/// `*fr.PageSize`, `*zh_TW.Translation`: a translation, skipped.
fn is_localized(main: &str) -> bool {
    main.split_once('.').is_some_and(|(lang, _)| !lang.is_empty() && lang.bytes().all(|b| b.is_ascii_alphabetic() || b == b'_'))
}

/// A translation string with its `<hex>` substrings decoded (`CD<2F>DVD` -> `CD/DVD`).
fn decode_label(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('<') {
        out.extend_from_slice(&rest.as_bytes()[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else {
            out.extend_from_slice(&rest.as_bytes()[start..]);
            rest = "";
            break;
        };
        let hex: String = after[..end].chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = (hex.len().is_multiple_of(2) && hex.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect::<Vec<u8>>());
        match bytes {
            Some(b) => out.extend(b),
            // Not hex: keep the text as written.
            None => out.extend_from_slice(&rest.as_bytes()[start..start + 2 + end]),
        }
        rest = &after[end + 1..];
    }
    out.extend_from_slice(rest.as_bytes());
    String::from_utf8(out).unwrap_or_else(|e| e.into_bytes().iter().map(|&b| b as char).collect())
}

fn unquote(v: &str) -> &str {
    v.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(v)
}

/// Parse PPD text. Errors (with the line number) are structural: no
/// `*PPD-Adobe` header, unbalanced `*OpenUI`/`*CloseUI`, an unknown UI type,
/// or a quoted value that never closes.
pub fn parse(text: &str) -> Result<Ppd, String> {
    if !text.starts_with("*PPD-Adobe:") {
        return Err("not a PPD file (no *PPD-Adobe header)".into());
    }
    let mut ppd = Ppd::default();
    let mut defaults = HashMap::new();
    let mut open: Option<PpdOption> = None;
    let mut lines = text.lines().enumerate();
    while let Some((i, line)) = lines.next() {
        let n = i + 1;
        // `*%` is a comment; lines without `*` are blank or stray text.
        let Some(entry) = line.strip_prefix('*').filter(|e| !e.starts_with('%')) else { continue };
        // No colon: `*End` and the like.
        let Some((key, value)) = entry.split_once(':') else { continue };
        let mut value = value.trim().to_string();
        // A quoted value runs to the next `"`, across lines if need be.
        if value.starts_with('"') && !value[1..].contains('"') {
            loop {
                let Some((_, next)) = lines.next() else { return Err(format!("line {n}: quoted value never closed")) };
                value.push('\n');
                value.push_str(next);
                if next.contains('"') {
                    break;
                }
            }
        }
        // `*Main Name/Translation: value`; the translation may hold `/`.
        let (main, spec) = key.split_once([' ', '\t']).map_or((key, ""), |(m, s)| (m, s.trim()));
        if is_localized(main) {
            continue;
        }
        let (name, label) = spec.split_once('/').map_or((spec, None), |(k, l)| (k, Some(l)));
        match main {
            "OpenUI" | "JCLOpenUI" => {
                if let Some(o) = &open {
                    return Err(format!("line {n}: *{main} inside *OpenUI *{}", o.keyword));
                }
                let keyword = name.trim_start_matches('*').to_string();
                let ui = match value.as_str() {
                    "PickOne" => Ui::PickOne,
                    "Boolean" => Ui::Boolean,
                    "PickMany" => Ui::PickMany,
                    t => return Err(format!("line {n}: unknown UI type {t:?}")),
                };
                let label = label.map_or(keyword.clone(), decode_label);
                open = Some(PpdOption { keyword, label, ui, default: None, choices: Vec::new() });
            }
            "CloseUI" | "JCLCloseUI" => match open.take() {
                Some(o) if o.keyword == value.trim_start_matches('*') => ppd.options.push(o),
                _ => return Err(format!("line {n}: *{main}: {value} closes no open option")),
            },
            "UIConstraints" => {
                if let [k1, c1, k2, c2] = value.split_whitespace().collect::<Vec<_>>()[..]
                    && let (Some(k1), Some(k2)) = (k1.strip_prefix('*'), k2.strip_prefix('*'))
                    && !c1.starts_with('*')
                    && !c2.starts_with('*')
                {
                    ppd.constraints.push(Constraint { a: (k1.into(), c1.into()), b: (k2.into(), c2.into()) });
                }
            }
            "NickName" => ppd.nickname = Some(unquote(&value).to_string()),
            "1284DeviceID" => ppd.device_id = Some(unquote(&value).to_string()),
            _ => {
                if let Some(k) = main.strip_prefix("Default")
                    && spec.is_empty()
                {
                    defaults.insert(k.to_string(), value);
                } else if let Some(o) = open.as_mut()
                    && main == o.keyword
                    && !name.is_empty()
                {
                    o.choices.push(Choice { keyword: name.to_string(), label: label.map_or(name.to_string(), decode_label) });
                }
            }
        }
    }
    if let Some(o) = open {
        return Err(format!("*OpenUI *{} never closed", o.keyword));
    }
    for o in &mut ppd.options {
        o.default = defaults.remove(&o.keyword);
    }
    Ok(ppd)
}

impl Ppd {
    pub fn option(&self, keyword: &str) -> Option<&PpdOption> {
        self.options.iter().find(|o| o.keyword == keyword)
    }

    /// The value of `option` in `sel`, else its default.
    fn current<'a>(&'a self, sel: &'a Selection, option: &str) -> Option<&'a str> {
        sel.get(option).map(String::as_str).or_else(|| self.option(option)?.default.as_deref())
    }

    /// The constraints `sel` breaks (options missing from `sel` count as their default).
    #[cfg(test)]
    pub fn conflicts(&self, sel: &Selection) -> Vec<&Constraint> {
        let set = |(o, c): &(String, String)| self.current(sel, o) == Some(c.as_str());
        self.constraints.iter().filter(|k| set(&k.a) && set(&k.b)).collect()
    }

    /// The choices of `option` that break no constraint with the other
    /// options' values in `sel` (missing options count as their default).
    pub fn allowed_choices(&self, option: &str, sel: &Selection) -> Vec<&Choice> {
        let Some(o) = self.option(option) else { return Vec::new() };
        let excluded = |c: &Choice| {
            self.constraints.iter().any(|k| {
                k.partner(option, &c.keyword).is_some_and(|(p, pc)| p != option && self.current(sel, p) == Some(pc))
            })
        };
        o.choices.iter().filter(|c| !excluded(c)).collect()
    }

    /// A complete, conflict-free selection: one value per PPD option (keys
    /// the PPD lacks are dropped). Options are settled one by one, `edited`
    /// (the one the user just changed) first, then the rest in PPD order. An
    /// option keeps its value from `sel` unless that is no choice or conflicts
    /// with an option settled before it; then it falls back to its default,
    /// else to its first allowed choice. Same input, same output.
    pub fn resolve(&self, sel: &Selection, edited: Option<&str>) -> Selection {
        let is_edited = |o: &&PpdOption| Some(o.keyword.as_str()) == edited;
        let order = self.options.iter().filter(is_edited).chain(self.options.iter().filter(|o| !is_edited(o)));
        let mut out = Selection::new();
        for o in order {
            let valid = |c: &&str| o.choice(c).is_some();
            let wanted = sel.get(&o.keyword).map(String::as_str).filter(valid);
            let default = o.default.as_deref().filter(valid);
            let fits = |c: &&str| {
                !self.constraints.iter().any(|k| k.partner(&o.keyword, c).is_some_and(|(p, pc)| out.get(p).is_some_and(|v| v == pc)))
            };
            let all = o.choices.iter().map(|c| c.keyword.as_str());
            let pick = wanted.into_iter().chain(default).chain(all).find(fits);
            // Nothing fits (unsatisfiable PPD): keep the value, the conflict stays.
            if let Some(c) = pick.or(wanted).or(default).or(o.choices.first().map(|c| c.keyword.as_str())) {
                out.insert(o.keyword.clone(), c.to_string());
            }
        }
        out
    }
}

/// Printing Preferences controls that a PPD option can drive (MULTI-MODEL-PLAN §3).
/// The names are the PPD keywords.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Slot {
    PrintQuality,
    InputSlot,
    MediaType,
    PageSize,
    Borderless,
    Grayscale,
    CorrectionColor,
    GammaValue,
    Brightness,
    Contrast,
    Saturation,
    Cyan,
    Magenta,
    Yellow,
}

impl Slot {
    pub const ALL: [Slot; 14] = [
        Slot::PrintQuality,
        Slot::InputSlot,
        Slot::MediaType,
        Slot::PageSize,
        Slot::Borderless,
        Slot::Grayscale,
        Slot::CorrectionColor,
        Slot::GammaValue,
        Slot::Brightness,
        Slot::Contrast,
        Slot::Saturation,
        Slot::Cyan,
        Slot::Magenta,
        Slot::Yellow,
    ];

    /// (option, choice it must have), first match wins.
    fn candidates(self) -> &'static [(&'static str, Option<&'static str>)] {
        match self {
            Slot::PrintQuality => &[("PrintQuality", None)],
            Slot::InputSlot => &[("InputSlot", None)],
            Slot::MediaType => &[("MediaType", None)],
            Slot::PageSize => &[("PageSize", None)],
            Slot::Borderless => &[("Borderless", None)],
            Slot::Grayscale => &[("Color", Some("Grayscale")), ("Ink", Some("MONO"))],
            Slot::CorrectionColor => &[("CorrectionColor", None)],
            Slot::GammaValue => &[("GammaValue", None)],
            Slot::Brightness => &[("Brightness", None), ("BrightnessValue", None)],
            Slot::Contrast => &[("Contrast", None), ("ContrastValue", None)],
            Slot::Saturation => &[("Saturation", None), ("SaturationValue", None)],
            Slot::Cyan => &[("CyanValue", None)],
            Slot::Magenta => &[("MagentaValue", None)],
            Slot::Yellow => &[("YellowValue", None)],
        }
    }
}

/// The option behind a slot; `choice` is the checked value of the Grayscale box.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub option: String,
    pub choice: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SlotMap {
    /// Slots missing here are greyed out.
    pub slots: BTreeMap<Slot, Binding>,
    /// Options no slot takes, in PPD order, for the "Printer Options..."
    /// dialog. PageRegion is left out: CUPS sets it from PageSize.
    pub leftovers: Vec<String>,
}

pub fn slot_map(ppd: &Ppd) -> SlotMap {
    let mut map = SlotMap::default();
    for slot in Slot::ALL {
        let hit = slot.candidates().iter().find(|(opt, choice)| {
            ppd.option(opt).is_some_and(|o| choice.is_none_or(|c| o.choice(c).is_some()))
        });
        if let Some((opt, choice)) = hit {
            map.slots.insert(slot, Binding { option: opt.to_string(), choice: choice.map(str::to_string) });
        }
    }
    let taken = |k: &str| k == "PageRegion" || map.slots.values().any(|b| b.option == k);
    map.leftovers = ppd.options.iter().filter(|o| !taken(&o.keyword)).map(|o| o.keyword.clone()).collect();
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::thread;

    fn sel(pairs: &[(&str, &str)]) -> Selection {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn keywords(choices: &[&Choice]) -> Vec<String> {
        choices.iter().map(|c| c.keyword.clone()).collect()
    }

    const SYNTAX: &str = r#"*PPD-Adobe: "4.3"
*% Comment that mentions *OpenUI *Fake/Fake: PickOne
*NickName: "Mini Printer, Test Driver"
*1284DeviceID: "MFG:EPSON;MDL:Mini;CMD:ESCPL2;"
*ja.Translation PageSize/用紙サイズ:""

*OpenUI *PageSize/Paper Size: PickOne
*OrderDependency: 10 AnySetup *PageSize
*DefaultPageSize: A4
*PageSize A4/A4 / 210 x 297 mm: "<</PageSize[595 842]>>setpagedevice"
*fr.PageSize 	A4/A4 (fr):""
*it.PageSize 	L/Carta ""L"":""
*PageSize L/3.5x5 in / 89 x 127 mm: "
<</PageSize[252 360]>>
*PageSize Fake/Not a choice: this line is inside the quoted value
setpagedevice"
*End
*?PageSize: "query, not a choice"
*CloseUI: *PageSize

*OpenUI *ReduceEnlarge/Reduce/Enlarge: Boolean
*DefaultReduceEnlarge: False
*ReduceEnlarge True/On: ""
*ReduceEnlarge False/Off: ""
*CloseUI: *ReduceEnlarge

*OpenUI *Finish: PickMany
*Finish Staple: ""
*CloseUI: *Finish

*JCLOpenUI *JCLQuiet/Quiet Mode: PickOne
*DefaultJCLQuiet: Off
*JCLQuiet Off/Off: "@PJL SET QUIET=OFF<0A>"
*JCLQuiet On: "@PJL SET QUIET=ON<0A>"
*JCLCloseUI: *JCLQuiet
*DefaultFont: Courier
"#;

    #[test]
    fn parses_options_choices_defaults_and_ids() {
        let p = parse(SYNTAX).unwrap();
        assert_eq!(p.nickname.as_deref(), Some("Mini Printer, Test Driver"));
        assert_eq!(p.device_id.as_deref(), Some("MFG:EPSON;MDL:Mini;CMD:ESCPL2;"));
        let names: Vec<_> = p.options.iter().map(|o| (o.keyword.as_str(), o.label.as_str(), o.ui, o.default.as_deref())).collect();
        assert_eq!(
            names,
            [
                ("PageSize", "Paper Size", Ui::PickOne, Some("A4")),
                ("ReduceEnlarge", "Reduce/Enlarge", Ui::Boolean, Some("False")),
                ("Finish", "Finish", Ui::PickMany, None),
                ("JCLQuiet", "Quiet Mode", Ui::PickOne, Some("Off")),
            ]
        );
        let c = |k: &str, l: &str| Choice { keyword: k.into(), label: l.into() };
        assert_eq!(p.options[0].choices, [c("A4", "A4 / 210 x 297 mm"), c("L", "3.5x5 in / 89 x 127 mm")]);
        assert_eq!(p.options[2].choices, [c("Staple", "Staple")]);
        assert_eq!(p.options[3].choices, [c("Off", "Off"), c("On", "On")]);
    }

    #[test]
    fn hex_substrings_in_labels() {
        assert_eq!(decode_label("CD<2F>DVD-Standard"), "CD/DVD-Standard");
        assert_eq!(decode_label("9 x 13 cm (3<2E>5 x 5 in)"), "9 x 13 cm (3.5 x 5 in)");
        assert_eq!(decode_label("<E2 80 93>"), "\u{2013}");
        assert_eq!(decode_label("a <b> c"), "a <b> c");
        assert_eq!(decode_label("open <2F"), "open <2F");
    }

    #[test]
    fn crlf_parses_the_same() {
        assert_eq!(parse(&SYNTAX.replace('\n', "\r\n")).unwrap(), parse(SYNTAX).unwrap());
    }

    #[test]
    fn structural_errors() {
        let h = "*PPD-Adobe: \"4.3\"\n";
        let cases = [
            ("<html>404</html>".to_string(), "not a PPD"),
            (format!("{h}*OpenUI *A: PickOne\n*OpenUI *B: PickOne\n"), "line 3: *OpenUI inside *OpenUI *A"),
            (format!("{h}*OpenUI *A: PickOne\n*CloseUI: *B\n"), "line 3: *CloseUI: *B closes no open option"),
            (format!("{h}*CloseUI: *A\n"), "line 2: *CloseUI: *A closes no open option"),
            (format!("{h}*OpenUI *A: PickOne\n*A X: \"\"\n"), "*OpenUI *A never closed"),
            (format!("{h}*OpenUI *A: PickSome\n"), "line 2: unknown UI type \"PickSome\""),
            (format!("{h}*Foo: \"open\nstill open\n"), "line 2: quoted value never closed"),
        ];
        for (text, want) in cases {
            let e = parse(&text).unwrap_err();
            assert!(e.contains(want), "{e:?} lacks {want:?}");
        }
    }

    const RULES: &str = r#"*PPD-Adobe: "4.3"
*UIConstraints: *MediaType Photo *Quality Draft
*UIConstraints: *Quality Draft *MediaType Photo
*UIConstraints: *MediaType CD *PageSize A4
*UIConstraints: *MediaType CD *PageSize Letter
*UIConstraints: *MediaType *Duplex
*NonUIConstraints: *MediaType CD *Quality Best
*OpenUI *MediaType/Type: PickOne
*DefaultMediaType: Plain
*MediaType Plain/Plain: ""
*MediaType Photo/Photo: ""
*MediaType CD/CD: ""
*CloseUI: *MediaType
*OpenUI *Quality/Quality: PickOne
*DefaultQuality: Draft
*Quality Draft/Draft: ""
*Quality Normal/Normal: ""
*Quality Best/Best: ""
*CloseUI: *Quality
*OpenUI *PageSize/Size: PickOne
*DefaultPageSize: A4
*PageSize A4/A4: ""
*PageSize Letter/Letter: ""
*PageSize Disc/12 cm disc: ""
*CloseUI: *PageSize
"#;

    #[test]
    fn only_four_token_ui_constraints() {
        let p = parse(RULES).unwrap();
        let k = |a: &str, b: &str, c: &str, d: &str| Constraint { a: (a.into(), b.into()), b: (c.into(), d.into()) };
        assert_eq!(
            p.constraints,
            [
                k("MediaType", "Photo", "Quality", "Draft"),
                k("Quality", "Draft", "MediaType", "Photo"),
                k("MediaType", "CD", "PageSize", "A4"),
                k("MediaType", "CD", "PageSize", "Letter"),
            ]
        );
    }

    #[test]
    fn allowed_choices_hide_conflicts() {
        let p = parse(RULES).unwrap();
        // Defaults: Quality Draft rules out Photo, PageSize A4 rules out CD.
        assert_eq!(keywords(&p.allowed_choices("MediaType", &Selection::new())), ["Plain"]);
        assert_eq!(keywords(&p.allowed_choices("MediaType", &sel(&[("Quality", "Best"), ("PageSize", "Disc")]))), ["Plain", "Photo", "CD"]);
        assert_eq!(keywords(&p.allowed_choices("Quality", &sel(&[("MediaType", "Photo")]))), ["Normal", "Best"]);
        assert_eq!(keywords(&p.allowed_choices("PageSize", &sel(&[("MediaType", "CD")]))), ["Disc"]);
        // The option's own value does not limit it.
        assert_eq!(keywords(&p.allowed_choices("Quality", &sel(&[("Quality", "Best")]))), ["Draft", "Normal", "Best"]);
        assert!(p.allowed_choices("NoSuchOption", &Selection::new()).is_empty());
    }

    #[test]
    fn conflicts_count_both_lines() {
        let p = parse(RULES).unwrap();
        assert!(p.conflicts(&Selection::new()).is_empty());
        assert_eq!(p.conflicts(&sel(&[("MediaType", "Photo")])).len(), 2);
    }

    #[test]
    fn resolve_fills_defaults_and_drops_unknown_keys() {
        let p = parse(RULES).unwrap();
        let want = sel(&[("MediaType", "Plain"), ("Quality", "Draft"), ("PageSize", "A4")]);
        assert_eq!(p.resolve(&sel(&[("Bogus", "1")]), None), want);
    }

    #[test]
    fn resolve_after_load_keeps_ppd_order() {
        let p = parse(RULES).unwrap();
        // Quality: not a choice -> default. PageSize: Letter and the default A4
        // conflict with CD (settled first) -> first allowed choice.
        let loaded = sel(&[("MediaType", "CD"), ("Quality", "Bogus"), ("PageSize", "Letter")]);
        assert_eq!(p.resolve(&loaded, None), sel(&[("MediaType", "CD"), ("Quality", "Draft"), ("PageSize", "Disc")]));
    }

    #[test]
    fn resolve_after_edit_keeps_the_edit() {
        let p = parse(RULES).unwrap();
        let s = sel(&[("MediaType", "Photo"), ("Quality", "Draft"), ("PageSize", "A4")]);
        // Quality: Draft and its default conflict -> first allowed (Normal).
        assert_eq!(p.resolve(&s, Some("MediaType")), sel(&[("MediaType", "Photo"), ("Quality", "Normal"), ("PageSize", "A4")]));
        // MediaType: Photo conflicts -> default Plain.
        assert_eq!(p.resolve(&s, Some("Quality")), sel(&[("MediaType", "Plain"), ("Quality", "Draft"), ("PageSize", "A4")]));
    }

    #[test]
    fn resolve_is_conflict_free_deterministic_and_stable() {
        let p = parse(RULES).unwrap();
        let values = |k: &str| p.option(k).unwrap().choices.iter().map(|c| c.keyword.clone()).collect::<Vec<_>>();
        for m in values("MediaType") {
            for q in values("Quality") {
                for s in values("PageSize") {
                    let input = sel(&[("MediaType", &m), ("Quality", &q), ("PageSize", &s)]);
                    for edited in [None, Some("MediaType"), Some("Quality"), Some("PageSize")] {
                        let out = p.resolve(&input, edited);
                        assert!(p.conflicts(&out).is_empty(), "{input:?} {edited:?} -> {out:?}");
                        assert_eq!(out, p.resolve(&input, edited));
                        assert_eq!(p.resolve(&out, None), out);
                        if let Some(e) = edited {
                            assert_eq!(out[e], input[e]);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn slot_map_binds_and_leaves_over() {
        let text = r#"*PPD-Adobe: "4.3"
*OpenUI *PageSize: PickOne
*PageSize A4: ""
*CloseUI: *PageSize
*OpenUI *PageRegion: PickOne
*PageRegion A4: ""
*CloseUI: *PageRegion
*OpenUI *Color/Color Mode: PickOne
*Color Color: ""
*Color Vivid: ""
*CloseUI: *Color
*OpenUI *Ink: PickOne
*Ink COLOR: ""
*Ink MONO: ""
*CloseUI: *Ink
*OpenUI *Brightness: PickOne
*Brightness 0: ""
*CloseUI: *Brightness
*OpenUI *ContrastValue: PickOne
*ContrastValue 0: ""
*CloseUI: *ContrastValue
*OpenUI *Duplex: PickOne
*Duplex None: ""
*CloseUI: *Duplex
"#;
        let m = slot_map(&parse(text).unwrap());
        let b = |o: &str, c: Option<&str>| Binding { option: o.into(), choice: c.map(Into::into) };
        let want: BTreeMap<_, _> = [
            (Slot::PageSize, b("PageSize", None)),
            (Slot::Grayscale, b("Ink", Some("MONO"))),
            (Slot::Brightness, b("Brightness", None)),
            (Slot::Contrast, b("ContrastValue", None)),
        ]
        .into();
        assert_eq!(m.slots, want);
        // Color has no Grayscale choice, so it is not the Grayscale box.
        assert_eq!(m.leftovers, ["Color", "Duplex"]);
    }

    /// A one-shot HTTP server on 127.0.0.1; returns its address and the request it got.
    fn serve(reply: Vec<u8>) -> (String, thread::JoinHandle<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap().to_string();
        let h = thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut req = Vec::new();
            let mut buf = [0; 1024];
            while !req.ends_with(b"\r\n\r\n") {
                let n = s.read(&mut buf).unwrap();
                assert!(n > 0, "request cut short");
                req.extend_from_slice(&buf[..n]);
            }
            s.write_all(&reply).unwrap();
            String::from_utf8(req).unwrap()
        });
        (addr, h)
    }

    #[test]
    fn fetch_gets_the_body_and_encodes_the_queue() {
        let body = "*PPD-Adobe: \"4.3\"\n";
        let (addr, h) = serve(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len()).into_bytes());
        assert_eq!(fetch_from(&addr, "My Queue#1").unwrap(), body);
        assert!(h.join().unwrap().starts_with("GET /printers/My%20Queue%231.ppd HTTP/1.0\r\n"));
    }

    #[test]
    fn fetch_errors() {
        let (addr, _) = serve(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec());
        assert_eq!(fetch_from(&addr, "Nope").unwrap_err(), "PPD of queue Nope: CUPS has none (404 Not Found)");
        let (addr, _) = serve(b"HTTP/1.1 403 Forbidden\r\n\r\n".to_vec());
        let e = fetch_from(&addr, "Q").unwrap_err();
        assert!(e.contains("CUPS replied \"HTTP/1.1 403 Forbidden\""), "{e}");
        let (addr, _) = serve(b"HTTP/1.1 200 OK\r\nContent-Length: 99\r\n\r\n*PPD".to_vec());
        assert!(fetch_from(&addr, "Q").unwrap_err().ends_with("truncated reply"));
    }

    #[test]
    fn fetch_reads_latin1() {
        let (addr, _) = serve(b"HTTP/1.0 200 OK\r\n\r\n*NickName: \"Caf\xe9\"\n".to_vec());
        assert_eq!(fetch_from(&addr, "Q").unwrap(), "*NickName: \"Café\"\n");
    }

    /// PPD_LIVE_QUEUE=L800: one read-only GET from the running cupsd (nothing
    /// reaches the printer). With PPD_FIXTURES set, the reply must equal
    /// $PPD_FIXTURES/<queue>.ppd.
    #[test]
    fn live_cupsd() {
        let Ok(queue) = std::env::var("PPD_LIVE_QUEUE") else { return eprintln!("PPD_LIVE_QUEUE unset: skipped") };
        let text = fetch(&queue).unwrap();
        parse(&text).unwrap();
        if let Ok(dir) = std::env::var("PPD_FIXTURES") {
            assert_eq!(text, std::fs::read_to_string(PathBuf::from(dir).join(format!("{queue}.ppd"))).unwrap());
        }
        assert!(fetch("ppd-spike-no-such-queue").unwrap_err().contains("404"));
    }

    /// PPD_FIXTURES: the L800 PPD as cupsd serves it (Epson's file, not in git).
    #[test]
    fn l800_fixture() {
        let Ok(dir) = std::env::var("PPD_FIXTURES") else { return eprintln!("PPD_FIXTURES unset: skipped") };
        let p = parse(&std::fs::read_to_string(PathBuf::from(dir).join("L800.ppd")).unwrap()).unwrap();
        assert!(p.nickname.as_deref().unwrap().starts_with("Epson L800 Series - epson-inkjet-printer"));
        assert_eq!(p.device_id.as_deref(), Some("MFG:EPSON;MDL:L800 Series;DES:EPSON L800 Series;"));
        assert_eq!(p.constraints.len(), 91);
        let q = p.option("PrintQuality").unwrap();
        assert_eq!((q.label.as_str(), q.default.as_deref()), ("Quality Option", Some("Text")));
        assert_eq!(p.option("ReduceEnlarge").unwrap().label, "Reduce/Enlarge");
        assert_eq!(p.option("MediaType").unwrap().choice("PLAIN").unwrap().label, "plain papers");
        // *UIConstraints: *GammaValue 1.8 *CorrectionColor EPSONVivid (the default).
        let defaults = p.resolve(&Selection::new(), None);
        assert!(p.conflicts(&defaults).is_empty());
        assert!(!keywords(&p.allowed_choices("GammaValue", &defaults)).contains(&"1.8".to_string()));
        let mut s = defaults.clone();
        s.insert("GammaValue".into(), "1.8".into());
        let out = p.resolve(&s, Some("GammaValue"));
        assert_eq!(out["GammaValue"], "1.8");
        assert_ne!(out["CorrectionColor"], "EPSONVivid");
        let m = slot_map(&p);
        assert_eq!(m.slots.len(), Slot::ALL.len(), "{m:?}");
        assert_eq!(m.slots[&Slot::Grayscale], Binding { option: "Color".into(), choice: Some("Grayscale".into()) });
        eprintln!("L800: {} options, leftovers {:?}", p.options.len(), m.leftovers);
    }

    /// ESCPR_PPD_DIR: every escpr PPD parses, its defaults are choices, raise
    /// no conflict and survive resolve unchanged, and every *UIConstraints line
    /// is in the 4-token form. Prints the slot coverage (--nocapture).
    #[test]
    fn escpr_corpus() {
        let Ok(dir) = std::env::var("ESCPR_PPD_DIR") else { return eprintln!("ESCPR_PPD_DIR unset: skipped") };
        let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "ppd")).collect();
        files.sort();
        assert!(!files.is_empty());
        let (mut bad, mut options, mut constraints) = (Vec::new(), 0, 0);
        let mut slots: BTreeMap<Slot, usize> = BTreeMap::new();
        let mut leftovers: BTreeMap<String, usize> = BTreeMap::new();
        for f in &files {
            let name = f.file_name().unwrap().to_string_lossy();
            let text = std::fs::read_to_string(f).unwrap();
            let p = match parse(&text) {
                Ok(p) => p,
                Err(e) => {
                    bad.push(format!("{name}: {e}"));
                    continue;
                }
            };
            options += p.options.len();
            constraints += p.constraints.len();
            let lines = text.lines().filter(|l| l.starts_with("*UIConstraints:")).count();
            if lines != p.constraints.len() {
                bad.push(format!("{name}: {lines} *UIConstraints lines, {} parsed", p.constraints.len()));
            }
            if p.nickname.is_none() || p.device_id.is_none() {
                bad.push(format!("{name}: no *NickName or *1284DeviceID"));
            }
            for o in &p.options {
                if o.default.as_deref().and_then(|d| o.choice(d)).is_none() {
                    bad.push(format!("{name}: {} default {:?} is no choice", o.keyword, o.default));
                }
            }
            let raw: Selection = p.options.iter().filter_map(|o| Some((o.keyword.clone(), o.default.clone()?))).collect();
            if !p.conflicts(&raw).is_empty() {
                bad.push(format!("{name}: defaults conflict: {:?}", p.conflicts(&raw)));
            }
            let resolved = p.resolve(&Selection::new(), None);
            if resolved != raw || !p.conflicts(&resolved).is_empty() {
                bad.push(format!("{name}: resolve changed the defaults"));
            }
            let m = slot_map(&p);
            for s in m.slots.keys() {
                *slots.entry(*s).or_default() += 1;
            }
            for l in m.leftovers {
                *leftovers.entry(l).or_default() += 1;
            }
        }
        eprintln!("{} PPDs, {options} options, {constraints} constraints, {} failures", files.len(), bad.len());
        for s in Slot::ALL {
            eprintln!("  slot {s:?}: {}", slots.get(&s).unwrap_or(&0));
        }
        let mut top: Vec<_> = leftovers.into_iter().collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        eprintln!("  leftovers: {top:?}");
        assert!(bad.is_empty(), "{bad:#?}");
    }
}

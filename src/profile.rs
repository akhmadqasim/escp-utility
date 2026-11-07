//! Per-model maintenance data (profiles.toml, built in; `--profile FILE`
//! adds more). See profiles.toml for the fields.

use serde::Deserialize;

/// One Head Cleaning radio.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cleaning {
    pub label: String,
    /// Last byte of `CH 02 00 00 <ch>`.
    pub ch: u8,
}

impl Cleaning {
    /// A non-zero ch (e.g. L800 Power Cleaning) asks for confirmation first.
    pub fn confirms(&self) -> bool {
        self.ch != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub mdl: String,
    /// Who tested it on real hardware, and when; empty: experimental.
    pub tested: String,
    /// "G5P": the Windows L800 texts and status tables; "": generic.
    pub windows_family: String,
    /// Last byte of `NC 02 00 00 <n>`.
    pub nc_param: u8,
    pub cleaning: Vec<Cleaning>,
}

impl Profile {
    /// For a printer without a profile: what Epson's Linux utility sends to
    /// every model (NC 00, CH 00).
    pub fn generic() -> Profile {
        Profile {
            mdl: String::new(),
            tested: String::new(),
            windows_family: String::new(),
            nc_param: 0x00,
            cleaning: vec![Cleaning { label: "Cleaning".into(), ch: 0x00 }],
        }
    }

    pub fn experimental(&self) -> bool {
        self.tested.is_empty()
    }

    /// The profile lines of Printer and Option Information and `--report`.
    /// Only the generic profile has no MDL (`lint` rejects an empty one).
    pub fn summary(&self) -> String {
        let name = if !self.mdl.is_empty() { format!("{} (Windows family \"{}\")", self.mdl, self.windows_family) } else { "generic".into() };
        let tested = if self.experimental() { "Experimental: not yet tested on this model.".to_string() } else { format!("Tested: {}", self.tested) };
        let cleaning: Vec<String> = self.cleaning.iter().map(|c| format!("{} (CH {:02X})", c.label, c.ch)).collect();
        format!("Profile: {name}\n{tested}\nNozzle check: NC {:02X}; cleaning: {}\n", self.nc_param, cleaning.join(", "))
    }

    /// The Windows L800 texts and status tables apply.
    pub fn l800_windows(&self) -> bool {
        self.windows_family == "G5P"
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    printer: Vec<Profile>,
}

/// Parse and lint a profiles file.
pub fn parse(text: &str) -> Result<Vec<Profile>, String> {
    let f: File = toml::from_str(text).map_err(|e| e.to_string())?;
    lint(&f.printer)?;
    Ok(f.printer)
}

/// The built-in profiles.
pub fn builtin() -> Vec<Profile> {
    parse(include_str!("../profiles.toml")).expect("profiles.toml")
}

/// The first cleaning is the normal one (ch 0, no confirmation), no ch
/// appears twice, each MDL appears once, and windows_family is "G5P" or ""
/// (a typo would silently give the generic texts and pre-check).
pub fn lint(profiles: &[Profile]) -> Result<(), String> {
    for (i, p) in profiles.iter().enumerate() {
        if p.mdl.is_empty() {
            return Err(format!("printer {}: mdl is empty", i + 1));
        }
        if profiles[..i].iter().any(|q| q.mdl == p.mdl) {
            return Err(format!("{}: mdl appears twice", p.mdl));
        }
        if !matches!(p.windows_family.as_str(), "" | "G5P") {
            return Err(format!("{}: windows_family must be \"G5P\" or \"\", not {:?}", p.mdl, p.windows_family));
        }
        if p.cleaning.iter().enumerate().any(|(j, c)| p.cleaning[..j].iter().any(|d| d.ch == c.ch)) {
            return Err(format!("{}: a cleaning ch appears twice", p.mdl));
        }
        match p.cleaning.first() {
            Some(c) if c.ch == 0 => {}
            Some(_) => return Err(format!("{}: the first cleaning must have ch 0", p.mdl)),
            None => return Err(format!("{}: no cleaning", p.mdl)),
        }
    }
    Ok(())
}

/// The profile for `mdl`, if one matches exactly.
pub fn find<'a>(profiles: &'a [Profile], mdl: Option<&str>) -> Option<&'a Profile> {
    profiles.iter().find(|p| Some(p.mdl.as_str()) == mdl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_profiles_lint() {
        let p = builtin();
        let l800 = find(&p, Some("L800")).unwrap();
        assert_eq!(l800.nc_param, 0x00);
        assert_eq!(l800.cleaning.iter().map(|c| c.ch).collect::<Vec<_>>(), vec![0x00, 0x10]);
        assert!(!l800.cleaning[0].confirms());
        assert!(l800.cleaning[1].confirms());
        assert!(l800.l800_windows());
        assert!(!Profile::generic().l800_windows());
        assert_eq!(find(&p, Some("L800 Series")), None);
        assert!(lint(&[Profile::generic()]).is_err(), "generic has no mdl");
    }

    #[test]
    fn lint_rules() {
        let one = |mdl: &str, ch: &[u8]| Profile {
            mdl: mdl.into(),
            cleaning: ch.iter().map(|&ch| Cleaning { label: "x".into(), ch }).collect(),
            ..Profile::generic()
        };
        assert!(lint(&[one("A", &[0, 0x10]), one("B", &[0])]).is_ok());
        assert!(lint(&[one("A", &[0]), one("A", &[0])]).is_err());
        assert!(lint(&[one("A", &[0x10, 0])]).is_err());
        assert!(lint(&[one("A", &[])]).is_err());
        assert!(lint(&[one("A", &[0, 0x10, 0x10])]).is_err());
        assert!(lint(&[Profile { windows_family: "g5p".into(), ..one("A", &[0]) }]).is_err());
        assert!(lint(&[Profile { windows_family: "G5P".into(), ..one("A", &[0]) }]).is_ok());
        assert!(parse("[[printer]]\nmdl = \"A\"\ntested = \"\"\nwindows_family = \"\"\nnc_param = 0\ncleaning = []\nbogus = 1\n").is_err());
    }

    const OVERRIDE: &str = r#"
[[printer]]
mdl = "L800"
tested = "QA 2026-11-01"
windows_family = ""
nc_param = 0x01
cleaning = [{ label = "Only", ch = 0 }]

[[printer]]
mdl = "L805"
tested = ""
windows_family = "G5P"
nc_param = 0
cleaning = [{ label = "Cleaning", ch = 0 }, { label = "Power", ch = 0x10 }]
"#;

    #[test]
    fn generic_fallback_is_nc00_ch00_experimental() {
        let g = Profile::generic();
        assert_eq!(g.nc_param, 0);
        assert_eq!(g.cleaning, vec![Cleaning { label: "Cleaning".into(), ch: 0 }]);
        assert!(g.experimental());
        assert!(!g.cleaning[0].confirms());
        assert!(g.summary().starts_with("Profile: generic\nExperimental"));
        let p = builtin();
        assert_eq!(find(&p, None), None);
        assert_eq!(find(&p, Some("")), None, "empty MDL must not match");
        assert_eq!(find(&p, Some("XP-999")), None);
        assert_eq!(find(&p, None).cloned().unwrap_or_else(Profile::generic), g);
    }

    #[test]
    fn find_is_exact_and_case_sensitive() {
        let p = builtin();
        assert!(find(&p, Some("L800")).is_some());
        for m in ["l800", "L800 ", " L800", "L80", "L8000", "EPSON L800"] {
            assert_eq!(find(&p, Some(m)), None, "{m:?}");
        }
    }

    #[test]
    fn tested_flag_controls_experimental_and_summary() {
        let p = builtin();
        let l800 = find(&p, Some("L800")).unwrap();
        assert_eq!(l800.experimental(), l800.tested.is_empty());
        let t = Profile { tested: "Alto 2026-11-07".into(), ..l800.clone() };
        assert!(!t.experimental());
        let s = t.summary();
        assert!(s.contains("Profile: L800 (Windows family \"G5P\")"), "{s}");
        assert!(s.contains("Tested: Alto 2026-11-07"), "{s}");
        assert!(s.contains("NC 00; cleaning: Cleaning (CH 00), Power Cleaning (CH 10)"), "{s}");
        let e = Profile { tested: String::new(), ..l800.clone() };
        assert!(e.summary().contains("Experimental: not yet tested"));
    }

    #[test]
    fn override_file_parses_and_wins_over_builtin() {
        let mut ps = parse(OVERRIDE).unwrap();
        assert_eq!(ps.len(), 2);
        ps.extend(builtin()); // same order as main: --profile first
        let l800 = find(&ps, Some("L800")).unwrap();
        assert_eq!(l800.tested, "QA 2026-11-01");
        assert_eq!(l800.nc_param, 1);
        assert!(!l800.l800_windows());
        assert!(!l800.experimental());
        let l805 = find(&ps, Some("L805")).unwrap();
        assert!(l805.experimental() && l805.l800_windows());
        assert!(l805.cleaning[1].confirms());
    }

    #[test]
    fn override_file_rejects_bad_toml_and_unknown_fields() {
        let ok = "[[printer]]\nmdl = \"A\"\ntested = \"\"\nwindows_family = \"\"\nnc_param = 0\ncleaning = [{ label = \"c\", ch = 0 }]\n";
        assert!(parse(ok).is_ok());
        // Syntax errors, wrong types, out-of-range bytes, missing fields.
        for bad in [
            "[[printer]\nmdl = \"A\"",
            &ok.replace("nc_param = 0", "nc_param = \"0\""),
            &ok.replace("nc_param = 0", "nc_param = 256"),
            &ok.replace("ch = 0", "ch = -1"),
            &ok.replace("tested = \"\"\n", ""),
            &ok.replace("cleaning = [{ label = \"c\", ch = 0 }]\n", ""),
            &ok.replace("label = \"c\", ", ""),
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        // Unknown fields at every level.
        assert!(parse(&ok.replace("ch = 0 }", "ch = 0, confirm = true }")).is_err());
        assert!(parse(&format!("version = 1\n{ok}")).is_err());
        assert!(parse(&ok.replace("[[printer]]", "[[printers]]")).is_err());
        // An empty file has no [[printer]] table.
        assert!(parse("").is_err());
        // Lint runs on parse too.
        assert!(parse(&ok.replace("ch = 0", "ch = 0x10")).unwrap_err().contains("first cleaning must have ch 0"));
        assert!(parse(&format!("{ok}{ok}")).unwrap_err().contains("mdl appears twice"));
        assert!(parse(&ok.replace("mdl = \"A\"", "mdl = \"\"")).unwrap_err().contains("mdl is empty"));
    }

    #[test]
    fn lint_messages_name_the_rule() {
        let one = |ch: &[u8], fam: &str| Profile {
            mdl: "A".into(),
            windows_family: fam.into(),
            cleaning: ch.iter().map(|&ch| Cleaning { label: "x".into(), ch }).collect(),
            ..Profile::generic()
        };
        assert!(lint(&[one(&[0x10, 0], "")]).unwrap_err().contains("first cleaning must have ch 0"));
        assert!(lint(&[one(&[0, 0x10, 0x10], "")]).unwrap_err().contains("ch appears twice"));
        assert!(lint(&[one(&[0, 0], "")]).unwrap_err().contains("ch appears twice"));
        assert!(lint(&[one(&[], "")]).unwrap_err().contains("no cleaning"));
        for fam in ["G5", "G5P ", "L800", "g5p"] {
            assert!(lint(&[one(&[0], fam)]).unwrap_err().contains("windows_family"), "{fam:?}");
        }
        // MDLs differing only in case are distinct, matching `find`.
        let b = Profile { mdl: "a".into(), ..one(&[0], "") };
        assert!(lint(&[one(&[0], ""), b]).is_ok());
        // Every non-zero ch confirms; only 0 does not.
        assert!((1..=u8::MAX).all(|ch| Cleaning { label: String::new(), ch }.confirms()));
        assert!(!Cleaning { label: String::new(), ch: 0 }.confirms());
    }
}

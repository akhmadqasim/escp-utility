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

/// The first cleaning is the normal one (ch 0, no confirmation), and each
/// MDL appears once.
pub fn lint(profiles: &[Profile]) -> Result<(), String> {
    for (i, p) in profiles.iter().enumerate() {
        if p.mdl.is_empty() {
            return Err(format!("printer {}: mdl is empty", i + 1));
        }
        if profiles[..i].iter().any(|q| q.mdl == p.mdl) {
            return Err(format!("{}: mdl appears twice", p.mdl));
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
        assert!(parse("[[printer]]\nmdl = \"A\"\ntested = \"\"\nwindows_family = \"\"\nnc_param = 0\ncleaning = []\nbogus = 1\n").is_err());
    }
}

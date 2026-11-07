//! ESC/P2 "remote mode" maintenance jobs. The envelope is the same for every
//! Epson inkjet (Epson's Linux utility and escpr build the same one,
//! rea/MULTI-MODEL.md §1.3); the model only sets the last byte of `NC` and
//! of `CH` (profiles.toml).
//!
//! Every byte here comes from the Windows L800 driver (family "G5P",
//! `Windows/System32/spool/drivers/x64/3/E_*G5P.*`). The Windows utility
//! (`E_IHUTG5P.DLL`) does not hard-code the jobs. It runs a small script from
//! the model data DLL (`E_IHBRG5P.DLL`, RT_RCDATA). Each script record is three
//! little-endian u16 values `(opcode, flags, arg)`, and the interpreter is
//! `FUN_004083dc` in `E_IHUTG5P.DLL` (Ghidra 12.1.4 via REA). The utility opens
//! the queue with `StartDocPrinterW(datatype "RAW")` and `WritePrinter`s each
//! record's bytes in order (`FUN_00408008`, `FUN_004081a0`), so the job is a
//! plain raw spool job. Here we hand the same bytes to CUPS with `lp -o raw`.
//!
//! Opcode to bytes. The constants were read from `E_IHUTG5P.DLL` at the VAs
//! shown; the strings are from the `E_IHBRG5P.DLL` string table:
//!
//! | opcode | bytes                                   | source                          |
//! |--------|-----------------------------------------|---------------------------------|
//! | 0x104  | `00 00 00 1B 01 "@EJL 1284.4" 0A "@EJL     " 0A` | string 4300 (`FUN_0040aed8`), same as default at 0x4032d8 |
//! | 0x102  | `1B 40 1B 40` (ESC @ twice)             | 0x403358                        |
//! | 0x100  | `1B 28 52 08 00 00 "REMOTE1"`           | 0x4032f8, enter remote mode     |
//! | 0x106  | `"TI" 08 00 00 YY YY MM DD hh mm ss`    | template 0x4565a8 + `GetLocalTime` (`FUN_0040b3ac`) |
//! | 0x105  | `"JS" 04 00 00 00 00 00`                | string 4301 is absent, so default 0x402f38 |
//! | 0x101  | `1B 00 00 00`                           | 0x402b1c, exit remote mode      |
//! | 0x109  | `0D`                                    | 0x402db6                        |
//! | 0x108  | `0A`                                    | 0x402db7                        |
//! | 0x10a  | `"VI" 02 00 00 00`                      | 0x40335c                        |
//! | 0x10b  | `"LD" 00 00`                            | 0x403364                        |
//! | 0x103  | `0C` (form feed)                        | 0x402dc5                        |
//! | 0x0001 | string N: first byte = length, then bytes | `FUN_00408e50`                 |
//! | 0x216  | string N with byte `flags` replaced by the cleaning type | `FUN_00409b3c` |
//!
//! After the script, `FUN_004082ec` always appends the job-end trailer from
//! `FUN_0040b248`: `E_IHBRG5P` string 4302 (fallback copy at 0x403308). It is
//! skipped only when a script sets opcode 0x212, and neither script does.
//!
//! Cross-check with Gutenprint 5.3.5 `src/escputil/escputil.c`. It uses the
//! same "exit packet mode" header (`exit_packet_mode_old`), the same
//! `ESC ( R 08 00 00 REMOTE1` envelope (`start_remote_sequence`), and
//! `CH 02 00 00 00` / `NC 02 00 00 00` (`do_head_clean`, `do_nozzle_check`).
//! Gutenprint adds its own trailer (`ESC 00 00 00 ESC 00`, then `FF ESC 00 ESC 00`).
//! We follow the Windows L800 script exactly instead.

/// Exit IEEE 1284.4 packet mode (EJL). Opcode 0x104, `E_IHBRG5P` string 4300:
/// `1B,00,00,00,1B,01,40,45,4A,4C,20,31,32,38,34,2E,34,0A,40,45,4A,4C,20,20,20,20,20,0A`
/// (the leading 0x1B = 27 is the length byte).
pub const EXIT_PACKET_MODE: &[u8] = b"\x00\x00\x00\x1b\x01@EJL 1284.4\n@EJL     \n";
/// Opcode 0x102: `ESC @ ESC @` (printer reset, twice).
pub const RESET_TWICE: &[u8] = b"\x1b@\x1b@";
/// Opcode 0x100: `ESC ( R` with length 8, then `00 "REMOTE1"`, which enters remote mode.
pub const ENTER_REMOTE: &[u8] = b"\x1b(R\x08\x00\x00REMOTE1";
/// Opcode 0x101: `ESC 00 00 00`, which exits remote mode.
pub const EXIT_REMOTE: &[u8] = b"\x1b\x00\x00\x00";
/// Opcode 0x105: job start. `E_IHBRG5P` has no string 4301, so the default at 0x402f38 is used.
pub const JOB_START: &[u8] = b"JS\x04\x00\x00\x00\x00\x00";
/// Job-end trailer, `E_IHBRG5P` string 4302
/// `1A,1B,40,1B,40,1B,28,52,08,00,00,52,45,4D,4F,54,45,31,4A,45,01,00,00,1B,00,00,00`
/// (the leading 0x1A = 26 is the length byte): ESC @ ESC @, enter remote,
/// `JE 01 00 00` (job end), exit remote.
pub const JOB_END_TRAILER: &[u8] = b"\x1b@\x1b@\x1b(R\x08\x00\x00REMOTE1JE\x01\x00\x00\x1b\x00\x00\x00";
/// Opcode 0x10a.
pub const VI: &[u8] = b"VI\x02\x00\x00\x00";
/// Opcode 0x10b.
pub const LD: &[u8] = b"LD\x00\x00";
/// Opcode 0x103: form feed.
pub const FORM_FEED: &[u8] = b"\x0c";
/// `E_IHBRG5P` string 1528 `06, 4E, 43, 02, 00, 00, 00`: nozzle check.
/// The last byte is the profile's `nc_param`.
pub const NOZZLE_CHECK: [u8; 6] = *b"NC\x02\x00\x00\x00";
/// `E_IHBRG5P` string 1527 `06, 43, 48, 02, 00, 00, 00`: head cleaning.
/// The last byte is the cleaning type, the profile's `ch` (L800: RT_RCDATA
/// 10460 gives 0x00 Cleaning, 0x10 Power Cleaning; `FUN_00409b3c`, opcode
/// 0x216 flags 5, writes it into byte 5).
pub const HEAD_CLEANING: [u8; 6] = *b"CH\x02\x00\x00\x00";

/// Local time sent with the `TI` command (opcode 0x106, `FUN_0040b3ac`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timestamp {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Timestamp {
    pub fn now() -> Self {
        use chrono::{Datelike, Timelike};
        let t = chrono::Local::now();
        Timestamp {
            year: t.year() as u16,
            month: t.month() as u8,
            day: t.day() as u8,
            hour: t.hour() as u8,
            minute: t.minute() as u8,
            second: t.second() as u8,
        }
    }

    /// `"TI" 08 00 00` then year (big-endian), month, day, hour, minute, second.
    /// Windows copies the 12-byte template at 0x4565a8 and writes
    /// `wYear >> 8, wYear, wMonth, wDay, wHour, wMinute, wSecond` into bytes 5..=11.
    pub fn ti_command(&self) -> [u8; 12] {
        let [yh, yl] = self.year.to_be_bytes();
        [
            b'T', b'I', 0x08, 0x00, 0x00, yh, yl, self.month, self.day, self.hour, self.minute,
            self.second,
        ]
    }
}

/// Nozzle check job, `E_IHBRG5P` RT_RCDATA 1800 (17 records):
/// 0x104, 0x102, 0x100, 0x106, 0x105, (0x0001 string 1528), 0x101,
/// 0x109, 0x108, 0x109, 0x108, 0x100, 0x10a, 0x10b, 0x101, 0x103, end,
/// then the job-end trailer. `nc` is the last byte of `NC`.
pub fn nozzle_check_job(nc: u8, ts: Timestamp) -> Vec<u8> {
    let mut nc_cmd = NOZZLE_CHECK;
    nc_cmd[5] = nc;
    [
        EXIT_PACKET_MODE,
        RESET_TWICE,
        ENTER_REMOTE,
        &ts.ti_command(),
        JOB_START,
        &nc_cmd,
        EXIT_REMOTE,
        b"\r\n\r\n",
        ENTER_REMOTE,
        VI,
        LD,
        EXIT_REMOTE,
        FORM_FEED,
        JOB_END_TRAILER,
    ]
    .concat()
}

/// Head cleaning job, `E_IHBRG5P` RT_RCDATA 1802 (7 records):
/// 0x104, 0x102, 0x100, 0x106, (0x216 flags 5, string 1527), 0x101, end,
/// then the job-end trailer. There is no job start, VI/LD or form feed in
/// this script. `ch` is the last byte of `CH` (the cleaning type).
pub fn head_cleaning_job(ch_type: u8, ts: Timestamp) -> Vec<u8> {
    let mut ch = HEAD_CLEANING;
    ch[5] = ch_type;
    [EXIT_PACKET_MODE, RESET_TWICE, ENTER_REMOTE, &ts.ti_command(), &ch, EXIT_REMOTE, JOB_END_TRAILER].concat()
}

/// Classic 16-bytes-per-line hexdump with an ASCII column.
pub fn hexdump(bytes: &[u8]) -> String {
    let mut out = String::new();
    for (i, chunk) in bytes.chunks(16).enumerate() {
        out.push_str(&format!("{:08x}  ", i * 16));
        for j in 0..16 {
            match chunk.get(j) {
                Some(b) => out.push_str(&format!("{b:02x} ")),
                None => out.push_str("   "),
            }
            if j == 7 {
                out.push(' ');
            }
        }
        out.push_str(" |");
        for &b in chunk {
            out.push(if (0x20..0x7f).contains(&b) { b as char } else { '.' });
        }
        out.push_str("|\n");
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const TS: Timestamp = Timestamp { year: 2026, month: 11, day: 7, hour: 14, minute: 5, second: 9 };

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.split_whitespace().collect();
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// The clock in a job's `TI` command.
    fn ti_of(job: &[u8]) -> Timestamp {
        let i = job.windows(5).position(|w| w == b"TI\x08\x00\x00").expect("TI") + 5;
        let t = &job[i..i + 7];
        Timestamp { year: u16::from_be_bytes([t[0], t[1]]), month: t[2], day: t[3], hour: t[4], minute: t[5], second: t[6] }
    }

    /// Rebuild a golden job (`nozzle-check` or `head-cleaning-<ch>`) with
    /// the profile's parameters and the golden's own clock.
    fn rebuild(p: &crate::profile::Profile, job: &str, golden: &[u8]) -> Vec<u8> {
        let ts = ti_of(golden);
        if job == "nozzle-check" {
            return nozzle_check_job(p.nc_param, ts);
        }
        let ch = u8::from_str_radix(job.strip_prefix("head-cleaning-").expect(job), 16).unwrap();
        assert!(p.cleaning.iter().any(|c| c.ch == ch), "{}: no cleaning with ch {ch:02x}", p.mdl);
        head_cleaning_job(ch, ts)
    }

    /// The bytes of a `.hex` golden (`#` lines are comments).
    pub(crate) fn golden(path: &std::path::Path) -> Vec<u8> {
        hex(&std::fs::read_to_string(path).unwrap().lines().filter(|l| !l.starts_with('#')).collect::<Vec<_>>().join(" "))
    }

    /// tests/golden/<MDL>/<job>.hex: golden vectors (the L800 ones come from
    /// the independent verifier, phase2/verify/golden.json). Each is rebuilt
    /// from its model's profile and compared byte for byte.
    #[test]
    fn jobs_match_goldens() {
        let profiles = crate::profile::builtin();
        let mut seen = 0;
        for model in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden")).unwrap().flatten() {
            let mdl = model.file_name().to_string_lossy().into_owned();
            let p = crate::profile::find(&profiles, Some(&mdl)).unwrap_or_else(|| panic!("no profile for {mdl}"));
            for f in std::fs::read_dir(model.path()).unwrap().flatten() {
                let job = f.path().file_stem().unwrap().to_string_lossy().into_owned();
                let want = golden(&f.path());
                assert_eq!(rebuild(p, &job, &want), want, "{mdl}/{job}");
                seen += 1;
            }
        }
        assert!(seen >= 3);
    }

    /// The generic profile sends the same bytes as the L800's normal jobs.
    #[test]
    fn generic_equals_l800() {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/L800"));
        let g = crate::profile::Profile::generic();
        for job in ["nozzle-check", "head-cleaning-00"] {
            let want = golden(&dir.join(format!("{job}.hex")));
            assert_eq!(rebuild(&g, job, &want), want, "{job}");
        }
        let want = golden(&dir.join("nozzle-check.hex"));
        assert_eq!(want.len(), 132);
        assert_eq!(ti_of(&want), Timestamp { year: 2025, month: 1, day: 2, hour: 3, minute: 4, second: 5 });
    }

    fn golden_files() -> Vec<std::path::PathBuf> {
        let mut v = vec![];
        for model in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden")).unwrap().flatten() {
            v.extend(std::fs::read_dir(model.path()).unwrap().flatten().map(|f| f.path()));
        }
        v
    }

    /// Each golden is `.hex`, has a `#` header naming its byte count, and
    /// every data line is at most 16 two-digit hex tokens.
    #[test]
    fn goldens_are_well_formed_hex() {
        for path in golden_files() {
            let name = path.display().to_string();
            assert_eq!(path.extension().and_then(|e| e.to_str()), Some("hex"), "{name}");
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(text.starts_with('#'), "{name}: no header");
            for l in text.lines().filter(|l| !l.starts_with('#')) {
                let toks: Vec<&str> = l.split_whitespace().collect();
                assert!(!toks.is_empty() && toks.len() <= 16, "{name}: {l:?}");
                assert!(toks.iter().all(|t| t.len() == 2 && t.chars().all(|c| c.is_ascii_hexdigit())), "{name}: {l:?}");
            }
            let n = golden(&path).len();
            assert!(text.lines().next().unwrap().contains(&format!("{n} bytes")), "{name}: header lacks {n} bytes");
        }
    }

    /// Every cleaning of a profile with a golden directory has a golden.
    #[test]
    fn every_profiled_cleaning_has_a_golden() {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden"));
        for p in crate::profile::builtin() {
            let d = dir.join(&p.mdl);
            if !d.is_dir() {
                continue;
            }
            assert!(d.join("nozzle-check.hex").is_file(), "{}: nozzle-check", p.mdl);
            for c in &p.cleaning {
                assert!(d.join(format!("head-cleaning-{:02x}.hex", c.ch)).is_file(), "{}: ch {:02x}", p.mdl, c.ch);
            }
        }
    }

    /// A `--profile` file is searched before the built-ins (as in main), and
    /// its cleaning ch lands in byte 5 of `CH`; nothing else changes.
    #[test]
    fn profile_file_custom_ch() {
        let file = "[[printer]]\nmdl = \"L800\"\ntested = \"\"\nwindows_family = \"G5P\"\nnc_param = 0x00\n\
                    cleaning = [ { label = \"Cleaning\", ch = 0x00 }, { label = \"Deep\", ch = 0x20 } ]\n";
        let mut profiles = crate::profile::parse(file).unwrap();
        profiles.extend(crate::profile::builtin());
        let p = crate::profile::find(&profiles, Some("L800")).unwrap();
        assert!(p.cleaning.iter().any(|c| c.ch == 0x20 && c.label == "Deep"));
        let base = golden(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/L800/head-cleaning-00.hex")));
        let got = head_cleaning_job(0x20, ti_of(&base));
        let i = base.windows(5).position(|w| w == b"CH\x02\x00\x00").unwrap() + 5;
        let mut want = base.clone();
        want[i] = 0x20;
        assert_eq!(got, want);
    }

    #[test]
    fn exit_packet_mode_matches_string_4300() {
        let s4300 = hex("1B 00 00 00 1B 01 40 45 4A 4C 20 31 32 38 34 2E 34 0A 40 45 4A 4C 20 20 20 20 20 0A");
        assert_eq!(s4300[0] as usize, EXIT_PACKET_MODE.len());
        assert_eq!(EXIT_PACKET_MODE, &s4300[1..]);
    }

    #[test]
    fn trailer_matches_string_4302() {
        let s4302 = hex("1A 1B 40 1B 40 1B 28 52 08 00 00 52 45 4D 4F 54 45 31 4A 45 01 00 00 1B 00 00 00");
        assert_eq!(s4302[0] as usize, JOB_END_TRAILER.len());
        assert_eq!(JOB_END_TRAILER, &s4302[1..]);
    }

    #[test]
    fn ti_encodes_local_time_big_endian_year() {
        assert_eq!(TS.ti_command().to_vec(), hex("54 49 08 00 00 07 ea 0b 07 0e 05 09"));
    }

    #[test]
    fn clock_only_changes_the_ti_bytes() {
        let a = nozzle_check_job(0, Timestamp { year: 2025, month: 1, day: 2, hour: 3, minute: 4, second: 5 });
        let b = nozzle_check_job(0, TS);
        let differing: Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
        assert!(differing.iter().all(|i| (49..=55).contains(i)), "{differing:?}");
    }

    #[test]
    fn hexdump_format() {
        let d = hexdump(b"NC\x02\x00\x00\x00");
        assert_eq!(d, "00000000  4e 43 02 00 00 00                                 |NC....|\n");
    }

    #[test]
    fn ti_leap_day_2099_and_midnight() {
        let leap = Timestamp { year: 2028, month: 2, day: 29, hour: 23, minute: 59, second: 59 };
        assert_eq!(leap.ti_command().to_vec(), hex("54 49 08 00 00 07 ec 02 1d 17 3b 3b"));
        let y2099 = Timestamp { year: 2099, month: 12, day: 31, hour: 0, minute: 0, second: 0 };
        assert_eq!(y2099.ti_command().to_vec(), hex("54 49 08 00 00 08 33 0c 1f 00 00 00"));
        let midnight = Timestamp { year: 2026, month: 1, day: 1, hour: 0, minute: 0, second: 0 };
        assert_eq!(&midnight.ti_command()[7..], &[1, 1, 0, 0, 0]);
    }

    /// TI sits right after the first ENTER_REMOTE, at bytes 44..56 in both jobs.
    #[test]
    fn ti_position_in_jobs() {
        for job in [nozzle_check_job(0, TS), head_cleaning_job(0, TS)] {
            assert_eq!(&job[..44], [EXIT_PACKET_MODE, RESET_TWICE, ENTER_REMOTE].concat().as_slice());
            assert_eq!(job[44..56], TS.ti_command());
            assert_eq!(ti_of(&job), TS);
        }
    }

    /// Every built-in profile: NC/CH carry the parameter in byte 5 only, the
    /// lengths stay 132/92, and every job ends with the JE trailer.
    #[test]
    fn every_profile_param_lands_in_byte_5() {
        let profiles = crate::profile::builtin();
        assert!(!profiles.is_empty());
        for p in &profiles {
            let nc = nozzle_check_job(p.nc_param, TS);
            assert_eq!(nc.len(), 132, "{}", p.mdl);
            let at = nc.windows(5).position(|w| w == b"NC\x02\x00\x00").expect("NC");
            assert_eq!(at, 64, "{}", p.mdl);
            assert_eq!(nc[at + 5], p.nc_param, "{}", p.mdl);
            assert!(nc.ends_with(JOB_END_TRAILER), "{}", p.mdl);
            for c in &p.cleaning {
                let ch = head_cleaning_job(c.ch, TS);
                assert_eq!(ch.len(), 92, "{} {}", p.mdl, c.label);
                assert_eq!(&ch[56..61], b"CH\x02\x00\x00");
                assert_eq!(ch[61], c.ch, "{} {}", p.mdl, c.label);
                assert_eq!(&ch[62..66], EXIT_REMOTE);
                assert!(ch.ends_with(JOB_END_TRAILER), "{} {}", p.mdl, c.label);
            }
        }
    }

    /// Only byte 5 of NC/CH depends on the parameter, for all 256 values.
    #[test]
    fn param_changes_only_one_byte() {
        let nc0 = nozzle_check_job(0, TS);
        let ch0 = head_cleaning_job(0, TS);
        for v in 1..=255u8 {
            let nc = nozzle_check_job(v, TS);
            let d: Vec<usize> = (0..nc.len()).filter(|&i| nc[i] != nc0[i]).collect();
            assert_eq!(d, vec![69]);
            let ch = head_cleaning_job(v, TS);
            let d: Vec<usize> = (0..ch.len()).filter(|&i| ch[i] != ch0[i]).collect();
            assert_eq!(d, vec![61]);
        }
    }

    /// Remote-mode framing: every ENTER_REMOTE is closed by an EXIT_REMOTE
    /// before the next one, and the job ends outside remote mode.
    #[test]
    fn remote_mode_enter_exit_balanced() {
        for (job, want) in [(nozzle_check_job(0, TS), 3), (head_cleaning_job(0x10, TS), 2)] {
            let enters: Vec<usize> = job.windows(ENTER_REMOTE.len()).enumerate().filter(|(_, w)| *w == ENTER_REMOTE).map(|(i, _)| i).collect();
            let exits: Vec<usize> = job.windows(EXIT_REMOTE.len()).enumerate().filter(|(_, w)| *w == EXIT_REMOTE).map(|(i, _)| i).collect();
            // EXIT_PACKET_MODE starts with 00 00 00 1B, not ESC 00 00 00, so it is not counted.
            assert_eq!(enters.len(), want);
            assert_eq!(exits.len(), want);
            for (k, e) in enters.iter().enumerate() {
                assert!(exits[k] > *e);
                if let Some(next) = enters.get(k + 1) {
                    assert!(exits[k] < *next);
                }
            }
            assert_eq!(*exits.last().unwrap(), job.len() - EXIT_REMOTE.len());
        }
    }

    /// JE trailer: reset, enter remote, `JE 01 00 00`, exit remote, 26 bytes.
    #[test]
    fn je_trailer_structure() {
        assert_eq!(JOB_END_TRAILER.len(), 26);
        assert_eq!(JOB_END_TRAILER, [RESET_TWICE, ENTER_REMOTE, b"JE\x01\x00\x00", EXIT_REMOTE].concat().as_slice());
    }

    /// Each remote command's 2-byte little-endian length matches its payload.
    #[test]
    fn remote_command_lengths() {
        for cmd in [&ENTER_REMOTE[2..], JOB_START, VI, LD, &NOZZLE_CHECK[..], &HEAD_CLEANING[..], &TS.ti_command()[..]] {
            let (len_at, body) = if cmd[0] == b'R' { (1, 3) } else { (2, 4) };
            let n = u16::from_le_bytes([cmd[len_at], cmd[len_at + 1]]) as usize;
            assert_eq!(cmd.len() - body, n, "{:?}", String::from_utf8_lossy(cmd));
        }
    }
}

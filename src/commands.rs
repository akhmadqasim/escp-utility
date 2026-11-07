//! ESC/P2 "remote mode" maintenance jobs for the Epson L800.
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
pub const NOZZLE_CHECK: &[u8] = b"NC\x02\x00\x00\x00";
/// `E_IHBRG5P` string 1527 `06, 43, 48, 02, 00, 00, 00`: head cleaning.
/// The last byte is replaced by the cleaning type (see [`Cleaning`]).
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

/// Cleaning type. These are the radio buttons of the Windows Head Cleaning dialog.
/// `E_IHBRG5P` RT_RCDATA 10460 lists them as `(control, label string, default,
/// ..., value, ...)`: control 513 "Cleaning" (string 10470, default) has value
/// 0x0000, and control 514 "Power Cleaning" (string 10471) has value 0x0010.
/// `FUN_0041352c` stores the selected value in `DAT_00457cb0`, and
/// `FUN_00409b3c` (opcode 0x216, flags 5) writes it into byte 5 of `CH`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cleaning {
    Normal,
    Power,
}

impl Cleaning {
    pub fn type_byte(self) -> u8 {
        match self {
            Cleaning::Normal => 0x00,
            Cleaning::Power => 0x10,
        }
    }
}

/// Nozzle check job, `E_IHBRG5P` RT_RCDATA 1800 (17 records):
/// 0x104, 0x102, 0x100, 0x106, 0x105, (0x0001 string 1528), 0x101,
/// 0x109, 0x108, 0x109, 0x108, 0x100, 0x10a, 0x10b, 0x101, 0x103, end,
/// then the job-end trailer.
pub fn nozzle_check_job(ts: Timestamp) -> Vec<u8> {
    [
        EXIT_PACKET_MODE,
        RESET_TWICE,
        ENTER_REMOTE,
        &ts.ti_command(),
        JOB_START,
        NOZZLE_CHECK,
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
/// then the job-end trailer. There is no job start, VI/LD or form feed in this script.
pub fn head_cleaning_job(kind: Cleaning, ts: Timestamp) -> Vec<u8> {
    let mut ch = HEAD_CLEANING;
    ch[5] = kind.type_byte();
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
mod tests {
    use super::*;

    const TS: Timestamp = Timestamp { year: 2026, month: 11, day: 7, hour: 14, minute: 5, second: 9 };

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.split_whitespace().collect();
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// Golden vectors from the independent verifier (phase2/verify/golden.json).
    /// They emulate RCDATA 1800/1802 plus trailer 4302 with the clock pinned
    /// to 2025-01-02 03:04:05 (bytes 44..55).
    const GOLDEN_TS: Timestamp = Timestamp { year: 2025, month: 1, day: 2, hour: 3, minute: 4, second: 5 };
    const GOLDEN_NOZZLE_CHECK: &str = "0000001b0140454a4c20313238342e340a40454a4c20202020200a1b401b401b285208000052454d4f544531544908000007e901020304054a530400000000004e43020000001b0000000d0a0d0a1b285208000052454d4f5445315649020000004c4400001b0000000c1b401b401b285208000052454d4f5445314a450100001b000000";
    const GOLDEN_HEAD_CLEANING: &str = "0000001b0140454a4c20313238342e340a40454a4c20202020200a1b401b401b285208000052454d4f544531544908000007e901020304054348020000001b0000001b401b401b285208000052454d4f5445314a450100001b000000";
    const GOLDEN_POWER_CLEANING: &str = "0000001b0140454a4c20313238342e340a40454a4c20202020200a1b401b401b285208000052454d4f544531544908000007e901020304054348020000101b0000001b401b401b285208000052454d4f5445314a450100001b000000";

    #[test]
    fn nozzle_check_matches_golden() {
        let job = nozzle_check_job(GOLDEN_TS);
        assert_eq!(job.len(), 132);
        assert_eq!(job, hex(GOLDEN_NOZZLE_CHECK));
    }

    #[test]
    fn head_cleaning_matches_golden() {
        let job = head_cleaning_job(Cleaning::Normal, GOLDEN_TS);
        assert_eq!(job.len(), 92);
        assert_eq!(job, hex(GOLDEN_HEAD_CLEANING));
    }

    #[test]
    fn power_cleaning_matches_golden() {
        let job = head_cleaning_job(Cleaning::Power, GOLDEN_TS);
        assert_eq!(job.len(), 92);
        assert_eq!(job[61], 0x10);
        assert_eq!(job, hex(GOLDEN_POWER_CLEANING));
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
        let a = nozzle_check_job(GOLDEN_TS);
        let b = nozzle_check_job(TS);
        let differing: Vec<usize> = (0..a.len()).filter(|&i| a[i] != b[i]).collect();
        assert!(differing.iter().all(|i| (49..=55).contains(i)), "{differing:?}");
    }

    #[test]
    fn hexdump_format() {
        let d = hexdump(b"NC\x02\x00\x00\x00");
        assert_eq!(d, "00000000  4e 43 02 00 00 00                                 |NC....|\n");
    }
}

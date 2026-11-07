//! Parser for the binary `@BDC ST2` status reply.
//!
//! Layout (the same in Windows E_IAPRG5P `FUN_180079dc8`, escpr-lib
//! `serAnalyzeStatus` and ecbd `scan_bin_status`):
//!
//! ```text
//! 0..10   "@BDC ST2\r\n"
//! 10..12  payload length, u16 little-endian
//! 12..    records: [id u8][len u8][data; len]
//! ```

pub const HEADER: &[u8] = b"@BDC ST2\r\n";

/// Field ids (names from ecbd's `STBIN_*` enum and the Windows IHUT tags).
pub mod id {
    pub const ST: u8 = 0x01; // status
    pub const ER: u8 = 0x02; // error code
    pub const TP: u8 = 0x03; // makes the Windows pre-check refuse
    pub const WR: u8 = 0x04; // warning codes
    pub const TC: u8 = 0x0C; // cleaning time in seconds, u16 LE
    pub const IC: u8 = 0x0F; // ink levels
    pub const SN: u8 = 0x1F; // serial number
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub id: u8,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub declared_len: usize,
    /// Payload bytes present after the length word.
    pub available_len: usize,
    /// True if a record ran past the end of the payload.
    pub truncated: bool,
    pub fields: Vec<Field>,
}

impl Reply {
    pub fn get(&self, id: u8) -> Option<&[u8]> {
        self.fields.iter().find(|f| f.id == id).map(|f| f.data.as_slice())
    }
    pub fn complete(&self) -> bool {
        self.available_len >= self.declared_len && !self.truncated
    }
}

/// Find and parse an ST2 reply in `buf`. Tolerates leading bytes and a
/// payload shorter than declared (see `complete`).
pub fn parse(buf: &[u8]) -> Option<Reply> {
    let offset = buf.windows(HEADER.len()).position(|w| w == HEADER)?;
    let b = &buf[offset..];
    if b.len() < 12 {
        return None;
    }
    let declared_len = u16::from_le_bytes([b[10], b[11]]) as usize;
    let payload = &b[12..(12 + declared_len).min(b.len())];
    let mut fields = Vec::new();
    let mut i = 0;
    let mut truncated = false;
    while i < payload.len() {
        if i + 2 > payload.len() {
            truncated = true;
            break;
        }
        let (fid, len) = (payload[i], payload[i + 1] as usize);
        if i + 2 + len > payload.len() {
            truncated = true;
            fields.push(Field { id: fid, data: payload[i + 2..].to_vec() });
            break;
        }
        fields.push(Field { id: fid, data: payload[i + 2..i + 2 + len].to_vec() });
        i += 2 + len;
    }
    Some(Reply { declared_len, available_len: payload.len(), truncated, fields })
}

/// Overwrite the serial number (field 0x1F) with 'X', for reports.
pub fn mask_serial(buf: &mut [u8]) {
    let Some(offset) = buf.windows(HEADER.len()).position(|w| w == HEADER) else { return };
    let Some(len) = buf.get(offset + 10..offset + 12) else { return };
    let stop = (offset + 12 + u16::from_le_bytes([len[0], len[1]]) as usize).min(buf.len());
    let mut i = offset + 12;
    while i + 2 <= stop {
        let (fid, len) = (buf[i], buf[i + 1] as usize);
        let end = (i + 2 + len).min(stop);
        if fid == id::SN {
            buf[i + 2..end].fill(b'X');
        }
        i = end;
    }
}

/// One entry of field 0x0F.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ink {
    /// Windows `GetInkStatusInfo` ids: 1 BK, 3 C, 4 M, 5 Y, 6 LC, 7 LM.
    pub ink_id: u8,
    /// 0 black, 1 cyan, 2 magenta, 3 yellow, 4 light cyan, 5 light magenta.
    pub color: u8,
    /// 0..=100 percent, or a special code (see `percent`).
    pub raw_level: u8,
}

impl Ink {
    /// Percent, or None for the special codes (0x6E "n", 0x72/0x77, 0xFB..0xFF).
    pub fn percent(&self) -> Option<u8> {
        (self.raw_level <= 100).then_some(self.raw_level)
    }
}

/// Field 0x0F: `data[0]` is the entry size (4 on the L800:
/// `[ink id, colour, level, 0x02]`), then the entries.
pub fn inks(data: &[u8]) -> Vec<Ink> {
    let Some((&size, rest)) = data.split_first() else { return vec![] };
    if size < 3 {
        return vec![];
    }
    rest.chunks_exact(size as usize)
        .map(|e| Ink { ink_id: e[0], color: e[1], raw_level: e[2] })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Synthetic reply built from the documented layout.
    pub(crate) fn sample(st: u8, extra: &[u8]) -> Vec<u8> {
        let mut p = vec![0x01, 0x01, st];
        p.extend_from_slice(extra);
        let mut r = HEADER.to_vec();
        r.extend((p.len() as u16).to_le_bytes());
        r.extend(p);
        r
    }

    /// Live L800 reply, 2026-11-07 (serial field 0x1F replaced with 'X').
    pub(crate) fn live() -> Vec<u8> {
        include_str!("l800_st2_live_masked.hex")
            .lines()
            .filter(|l| !l.trim_start().starts_with('#'))
            .flat_map(|l| l.split_whitespace())
            .map(|t| u8::from_str_radix(t, 16).unwrap())
            .collect()
    }

    #[test]
    fn live_l800_reply() {
        let raw = live();
        let r = parse(&raw).unwrap();
        assert_eq!(raw.len(), 147);
        assert_eq!(r.declared_len, 135);
        assert!(r.complete());
        assert_eq!(r.get(id::ST), Some(&[0x04][..]));
        assert_eq!(r.get(id::ER), None);
        assert_eq!(r.get(id::WR), Some(&[0x74][..]));
        assert_eq!(r.get(id::TC), None);
        let ic = r.get(id::IC).unwrap();
        assert_eq!(ic[0], 4);
        let got: Vec<(u8, u8, u8)> = inks(ic).iter().map(|i| (i.ink_id, i.color, i.raw_level)).collect();
        assert_eq!(got, vec![(3, 1, 47), (4, 2, 37), (7, 5, 18), (6, 4, 3), (1, 0, 81), (5, 3, 27)]);
        let ids: Vec<u8> = r.fields.iter().map(|f| f.id).collect();
        assert_eq!(ids, vec![0x01, 0x04, 0x06, 0x0F, 0x10, 0x13, 0x19, 0x1B, 0x1F, 0x20, 0x24, 0x2F]);
    }

    #[test]
    fn tolerates_leading_bytes_and_truncation() {
        let mut b = vec![0x00, 0x00];
        b.extend(sample(4, &[0x04, 0x02, 0x71, 0x20]));
        b.truncate(b.len() - 1);
        let r = parse(&b).unwrap();
        assert!(!r.complete());
        assert_eq!(r.get(id::ST), Some(&[0x04][..]));
    }

    #[test]
    fn rejects_non_st2() {
        assert_eq!(parse(b"@BDC ST\r\nST:04;"), None);
        assert_eq!(parse(b"@BDC ST2\r\n\x01"), None);
    }

    #[test]
    fn serial_is_masked() {
        // The live reply's serial is masked already; put one in and mask it.
        let mut raw = live();
        let at = raw.windows(2).position(|w| w == [id::SN, 0x0A]).unwrap() + 2;
        raw[at..at + 10].copy_from_slice(b"S3RIAL0123");
        assert_ne!(raw, live());
        mask_serial(&mut raw);
        assert_eq!(raw, live());
    }

    #[test]
    fn special_ink_levels() {
        let i = inks(&[3, 1, 0, 0x6E, 3, 1, 100]);
        assert_eq!(i[0].percent(), None);
        assert_eq!(i[1].percent(), Some(100));
    }
}

#[cfg(test)]
mod parser_edge_tests {
    use super::*;

    fn reply(payload: &[u8]) -> Vec<u8> {
        let mut r = HEADER.to_vec();
        r.extend((payload.len() as u16).to_le_bytes());
        r.extend_from_slice(payload);
        r
    }

    #[test]
    fn empty_payload_is_complete() {
        let r = parse(&reply(&[])).unwrap();
        assert_eq!((r.declared_len, r.available_len), (0, 0));
        assert!(r.fields.is_empty() && r.complete());
    }

    #[test]
    fn length_is_little_endian_and_bytes_past_it_ignored() {
        let mut b = HEADER.to_vec();
        b.extend([0x03, 0x00, 0x01, 0x01, 0x04, 0xAA, 0xBB]);
        let r = parse(&b).unwrap();
        assert_eq!(r.declared_len, 3);
        assert_eq!(r.available_len, 3);
        assert_eq!(r.fields, vec![Field { id: 0x01, data: vec![0x04] }]);
        assert!(r.complete());
    }

    #[test]
    fn short_payload_not_complete_but_not_truncated() {
        let mut b = HEADER.to_vec();
        b.extend([0x10, 0x00, 0x01, 0x01, 0x04]);
        let r = parse(&b).unwrap();
        assert_eq!((r.declared_len, r.available_len), (16, 3));
        assert!(!r.truncated && !r.complete());
        assert_eq!(r.get(id::ST), Some(&[0x04][..]));
    }

    #[test]
    fn dangling_id_byte_marks_truncated() {
        let r = parse(&reply(&[0x01, 0x01, 0x04, 0x02])).unwrap();
        assert!(r.truncated && !r.complete());
        assert_eq!(r.fields.len(), 1);
    }

    #[test]
    fn overlong_record_keeps_partial_data() {
        let r = parse(&reply(&[0x1F, 0x0A, b'A', b'B'])).unwrap();
        assert!(r.truncated);
        assert_eq!(r.get(id::SN), Some(&b"AB"[..]));
    }

    #[test]
    fn unknown_and_zero_length_fields_kept_in_order() {
        let r = parse(&reply(&[0x7E, 0x00, 0xFF, 0x02, 9, 9, 0x01, 0x01, 0x04])).unwrap();
        let ids: Vec<u8> = r.fields.iter().map(|f| f.id).collect();
        assert_eq!(ids, vec![0x7E, 0xFF, 0x01]);
        assert_eq!(r.get(0x7E), Some(&[][..]));
        assert_eq!(r.get(0xFF), Some(&[9, 9][..]));
        assert!(r.complete());
    }

    #[test]
    fn get_returns_first_duplicate() {
        let r = parse(&reply(&[0x01, 0x01, 0x04, 0x01, 0x01, 0x03])).unwrap();
        assert_eq!(r.get(id::ST), Some(&[0x04][..]));
    }

    #[test]
    fn header_after_garbage_and_header_only() {
        let mut b = b"\x00garbage@BDC ST".to_vec();
        b.extend(reply(&[0x01, 0x01, 0x04]));
        assert_eq!(parse(&b).unwrap().get(id::ST), Some(&[0x04][..]));
        assert_eq!(parse(HEADER), None);
        assert_eq!(parse(b""), None);
        assert_eq!(parse(b"@bdc st2\r\n\x00\x00"), None);
    }

    #[test]
    fn garbage_and_truncations_never_panic() {
        let full = super::tests::live();
        for n in 0..=full.len() {
            let _ = parse(&full[..n]);
            let mut m = full[..n].to_vec();
            mask_serial(&mut m);
        }
        let mut seed: u32 = 0x1234_5678;
        for _ in 0..2000 {
            let mut b = HEADER.to_vec();
            for _ in 0..(seed % 300) {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                b.push((seed >> 16) as u8);
            }
            if let Some(r) = parse(&b) {
                assert!(r.available_len <= r.declared_len);
                for f in &r.fields {
                    let _ = inks(&f.data);
                }
            }
            mask_serial(&mut b);
        }
    }

    #[test]
    fn max_declared_length() {
        let mut b = HEADER.to_vec();
        b.extend([0xFF, 0xFF, 0x01, 0x01, 0x04]);
        let r = parse(&b).unwrap();
        assert_eq!(r.declared_len, 65535);
        assert!(!r.complete());
    }

    #[test]
    fn inks_n_entry_layouts() {
        let four = [4, 1, 0, 81, 2, 3, 1, 47, 2, 4, 2, 37, 2, 5, 3, 27, 2];
        assert_eq!(inks(&four).len(), 4);
        assert_eq!(inks(&four)[3], Ink { ink_id: 5, color: 3, raw_level: 27 });
        // Trailing partial entry is dropped.
        assert_eq!(inks(&four[..four.len() - 1]).len(), 3);
        // 3-byte entries accepted; sizes < 3 and empty data rejected.
        assert_eq!(inks(&[3, 1, 0, 50]).len(), 1);
        assert!(inks(&[2, 1, 0, 1, 0]).is_empty());
        assert!(inks(&[0, 1, 2]).is_empty());
        assert!(inks(&[]).is_empty());
        assert!(inks(&[4]).is_empty());
        // Larger entry size: extra bytes ignored.
        let six = inks(&[6, 1, 0, 10, 9, 9, 9]);
        assert_eq!(six, vec![Ink { ink_id: 1, color: 0, raw_level: 10 }]);
    }

    #[test]
    fn percent_boundaries() {
        let p = |raw| Ink { ink_id: 1, color: 0, raw_level: raw }.percent();
        assert_eq!(p(0), Some(0));
        assert_eq!(p(100), Some(100));
        for raw in [101, 0x6E, 0x72, 0x77, 0xFB, 0xFF] {
            assert_eq!(p(raw), None);
        }
    }

    #[test]
    fn mask_serial_only_touches_sn_and_respects_bounds() {
        let mut b = reply(&[0x01, 0x01, 0x04, 0x1F, 0x03, b'1', b'2', b'3', 0x20, 0x01, b'7']);
        b.extend(b"\x1F\x02ZZ"); // past declared length: untouched
        mask_serial(&mut b);
        let r = parse(&b).unwrap();
        assert_eq!(r.get(id::SN), Some(&b"XXX"[..]));
        assert_eq!(r.get(0x20), Some(&b"7"[..]));
        assert!(b.ends_with(b"\x1F\x02ZZ"));
        // Truncated serial: mask what is there.
        let mut t = reply(&[0x1F, 0x0A, b'S', b'N']);
        let len = t.len();
        t[10] = 0x0C; // declare more than present
        mask_serial(&mut t);
        assert_eq!(&t[len - 2..], b"XX");
        // No header / no SN: unchanged.
        let mut g = b"no header here".to_vec();
        mask_serial(&mut g);
        assert_eq!(g, b"no header here");
    }
}

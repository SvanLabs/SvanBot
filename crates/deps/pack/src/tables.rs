//! Raw DEFLATE format tables, RFC 1951 section 3.2 (0258).

pub(super) const LEN_BASE: [u16; 29] =
    [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
pub(super) const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
pub(super) const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385,
    24577,
];
pub(super) const DIST_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
/// Order in which code-length code lengths are sent.
pub(super) const CL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
pub(super) const MAX_BITS: usize = 15;
pub(super) const END_OF_BLOCK: usize = 256;
/// Literal/length symbols that may appear in a stream (286 and 287 are reserved).
pub(super) const LITLEN_CODES: usize = 286;
/// Distance symbols that may appear in a stream (30 and 31 are reserved).
pub(super) const DIST_CODES: usize = 30;

pub(super) fn fixed_litlen_lengths() -> [u8; 288] {
    let mut l = [0u8; 288];
    for (i, v) in l.iter_mut().enumerate() {
        *v = match i {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    l
}

/// Length symbol index (0..29, symbol 257 + index) of a match length 3..=258.
pub(super) fn len_code(len: usize) -> usize {
    if len == 258 {
        return 28;
    }
    let l = (len - 3) as u32;
    if l < 8 {
        return l as usize;
    }
    let msb = 31 - l.leading_zeros();
    ((msb - 1) * 4 + ((l >> (msb - 2)) & 3)) as usize
}

/// Distance symbol (0..30) of a distance 1..=32768.
pub(super) fn dist_code(dist: usize) -> usize {
    let d = (dist - 1) as u32;
    if d < 4 {
        return d as usize;
    }
    let msb = 31 - d.leading_zeros();
    (msb * 2 + ((d >> (msb - 1)) & 1)) as usize
}

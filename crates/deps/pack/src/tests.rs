use super::deflate::{BLOCK_SYMBOLS, WINDOW, code_lengths};
use super::tables::{DIST_BASE, DIST_EXTRA, LEN_BASE, LEN_EXTRA, dist_code, len_code};
use super::*;

/// Deterministic pseudo-random bytes (xorshift), so the crate needs no dev-dependencies.
fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

/// JSON-like text resembling the stored rows: repeated keys, varying numbers.
fn json_rows(n: usize) -> Vec<u8> {
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!(
            "{{\"hand_id\":\"{:08x}-e6db-4db1-b2a4-{:012x}\",\"actions\":[{{\"action\":\"call\",\"amount\":{},\"street\":\"preflop\"}},{{\"action\":\"raise\",\"amount\":{},\"street\":\"flop\"}}],\"big_blind\":20,\"board\":[\"7s\",\"{}d\"],\"is_winner\":{}}}\n",
            i * 2654435761usize % 0xFFFF_FFFF,
            i * 97,
            i % 400,
            (i * 7) % 3000,
            2 + i % 8,
            i % 3 == 0
        ));
    }
    s.into_bytes()
}

fn round_trip(data: &[u8]) {
    for level in 0..=9 {
        let z = deflate(data, level);
        let back = inflate(&z, data.len()).unwrap_or_else(|e| panic!("level {level}, {} bytes: {e}", data.len()));
        assert_eq!(back, data, "level {level}, {} bytes", data.len());
    }
}

#[test]
fn round_trips_every_level_and_shape() {
    round_trip(b"");
    round_trip(b"a");
    round_trip(b"abc");
    round_trip(&[0u8; 1000]);
    round_trip(&[7u8; 70_000]);
    round_trip(b"abcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabcabc");
    round_trip(&noise(1, 1));
    round_trip(&noise(300, 2));
    round_trip(&noise(70_000, 3));
    round_trip(&json_rows(1));
    round_trip(&json_rows(40));
    round_trip(&json_rows(2_000));
    // Long matches at the far edge of the window: a 40 KB random block, repeated.
    let block = noise(40_000, 4);
    let mut far = block.clone();
    far.extend_from_slice(&block[block.len() - 32_768..]);
    far.extend_from_slice(&block);
    round_trip(&far);
    // Every byte value, runs of every length up to 300 (length codes and the 258 edge).
    let mut runs = Vec::new();
    for len in 1..300usize {
        runs.extend(std::iter::repeat_n((len % 251) as u8, len));
    }
    round_trip(&runs);
}

#[test]
fn many_random_inputs_round_trip() {
    for seed in 0..300u64 {
        let n = (seed as usize * 7919) % 5_000;
        // Mix noise with copied spans so matches of every distance occur.
        let mut data = noise(n, seed + 11);
        if n > 64 {
            for k in 0..(n / 64) {
                let from = (k * 131 + seed as usize) % (n - 32);
                let to = (k * 71 + 17) % (n - 32);
                let span: Vec<u8> = data[from..from + 32].to_vec();
                data[to..to + 32].copy_from_slice(&span);
            }
        }
        let level = (seed % 10) as u8;
        let z = deflate(&data, level);
        assert_eq!(inflate(&z, n).unwrap(), data, "seed {seed} level {level}");
    }
}

#[test]
fn json_compresses_well_and_levels_order_sensibly() {
    let data = json_rows(3_000);
    let stored = deflate(&data, 0).len();
    let fast = deflate(&data, 1).len();
    let default = deflate(&data, 6).len();
    let best = deflate(&data, 9).len();
    assert!(stored > data.len(), "stored adds framing");
    assert!(fast * 3 < data.len(), "level 1 ratio {:.3}", fast as f64 / data.len() as f64);
    assert!(default <= fast && best <= default + default / 50, "1: {fast}, 6: {default}, 9: {best}");
}

#[test]
fn incompressible_data_falls_back_to_stored_blocks() {
    let data = noise(200_000, 9);
    let z = deflate(&data, 6);
    // Each block of up to 16,384 symbols falls back to stored framing: 5 bytes per block.
    let blocks = data.len().div_ceil(BLOCK_SYMBOLS) + 1;
    assert!(z.len() <= data.len() + 5 * blocks, "{} > {} + framing", z.len(), data.len());
    assert_eq!(inflate(&z, data.len()).unwrap(), data);
}

#[test]
fn limits_are_enforced_before_output_grows() {
    let data = vec![b'x'; 100_000];
    let z = deflate(&data, 6);
    assert!(z.len() < 1_000);
    assert_eq!(inflate(&z, 99_999), Err(Error::TooLarge(99_999)));
    assert_eq!(inflate(&z, 100_000).unwrap().len(), 100_000);
    let frame = pack(&data);
    assert_eq!(unpack(&frame, 50_000), Err(Error::TooLarge(50_000)));
}

#[test]
fn corrupt_streams_error_and_never_panic() {
    let data = json_rows(200);
    let z = deflate(&data, 6);
    // Truncations.
    for cut in [0, 1, 2, z.len() / 3, z.len() / 2, z.len() - 1] {
        assert!(inflate(&z[..cut], data.len()).is_err() || cut == z.len(), "cut {cut}");
    }
    // Bit flips: any result is acceptable except a panic or exceeding the limit.
    for i in 0..z.len().min(400) {
        for bit in [0u8, 3, 7] {
            let mut bad = z.clone();
            bad[i] ^= 1 << bit;
            if let Ok(out) = inflate(&bad, data.len() * 2) {
                assert!(out.len() <= data.len() * 2);
            }
        }
    }
    // Random garbage.
    for seed in 0..500 {
        let junk = noise(1 + seed as usize % 200, seed + 1000);
        let _ = inflate(&junk, 10_000);
    }
    // Reserved block type 3.
    assert_eq!(inflate(&[0b0000_0111], 10), Err(Error::BadBlockType));
    // Stored block with a wrong complement.
    assert_eq!(inflate(&[0x01, 0x05, 0x00, 0x00, 0x00], 10), Err(Error::BadStoredLength));
}

#[test]
fn frames_check_length_and_crc() {
    let data = json_rows(50);
    let mut frame = pack(&data);
    assert!(is_packed(&frame));
    assert_eq!(unpack(&frame, usize::MAX).unwrap(), data);
    assert_eq!(text(&frame, usize::MAX).unwrap().as_bytes(), &data[..]);
    assert_eq!(text(b"{\"plain\":1}", 100).unwrap(), "{\"plain\":1}");
    assert_eq!(unpack(b"{\"plain\":1}", 100), Err(Error::NotAFrame));
    // Wrong CRC.
    frame[12] ^= 0xFF;
    assert_eq!(unpack(&frame, usize::MAX), Err(Error::Corrupt));
    frame[12] ^= 0xFF;
    // Wrong length.
    frame[4] ^= 1;
    assert!(unpack(&frame, usize::MAX).is_err());
    let empty = pack(b"");
    assert_eq!(unpack(&empty, 0).unwrap(), b"");
}

#[test]
fn crc32_matches_the_standard_check_values() {
    assert_eq!(crc32(b""), 0);
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b"The quick brown fox jumps over the lazy dog"), 0x414F_A339);
    let data = noise(10_001, 5);
    let (a, b) = data.split_at(4_321);
    assert_eq!(crc32_update(crc32(a), b), crc32(&data));
    // Slicing-by-8 equals the bytewise definition.
    let bytewise = |d: &[u8]| {
        let mut c = !0u32;
        for &x in d {
            c ^= x as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    };
    for n in [0, 1, 7, 8, 9, 63, 64, 1000] {
        assert_eq!(crc32(&data[..n]), bytewise(&data[..n]), "n {n}");
    }
}

#[test]
fn symbol_tables_match_the_rfc() {
    for len in 3..=258 {
        let s = len_code(len);
        assert!(len >= LEN_BASE[s] as usize && len - LEN_BASE[s] as usize <= (1 << LEN_EXTRA[s]) - 1 + (s == 28) as usize, "len {len}");
        if s < 28 {
            assert!(len < LEN_BASE[s + 1] as usize || (s == 27 && len == 258), "len {len} code {s}");
        }
    }
    assert_eq!(len_code(258), 28);
    assert_eq!(len_code(257), 27);
    for dist in 1..=32_768 {
        let s = dist_code(dist);
        assert!(dist >= DIST_BASE[s] as usize && dist - (DIST_BASE[s] as usize) < 1 << DIST_EXTRA[s], "dist {dist}");
    }
}

#[test]
fn length_limited_codes_are_complete_and_bounded() {
    // Fibonacci frequencies force deep unrestricted Huffman trees.
    let mut fib = vec![1u32, 1];
    while fib.len() < 40 {
        let n = fib.len();
        fib.push(fib[n - 1].saturating_add(fib[n - 2]));
    }
    for limit in [7usize, 15] {
        let lengths = code_lengths(&fib, limit);
        assert!(lengths.iter().all(|&l| l as usize <= limit && l > 0));
        let kraft: f64 = lengths.iter().map(|&l| 0.5f64.powi(l as i32)).sum();
        assert!((kraft - 1.0).abs() < 1e-12, "limit {limit}: kraft {kraft}");
    }
    assert_eq!(code_lengths(&[0, 5, 0], 15), vec![0, 1, 0]);
    assert_eq!(code_lengths(&[0, 0], 15), vec![0, 0]);
}

#[test]
fn preset_dictionaries_round_trip_and_shrink_similar_rows() {
    let dict = json_rows(200);
    let row = &json_rows(203)[json_rows(200).len()..];
    for level in 0..=9 {
        let z = deflate_dict(row, &dict, level);
        assert_eq!(inflate_dict(&z, &dict, row.len()).unwrap(), row, "level {level}");
    }
    let plain = deflate(row, 6).len();
    let with = deflate_dict(row, &dict, 6).len();
    assert!(with * 2 < plain, "dictionary {with} vs plain {plain}");
    // A dictionary longer than the window: only its last 32 KB matter, on both sides.
    let long: Vec<u8> = noise(50_000, 21).into_iter().chain(json_rows(100)).collect();
    let z = deflate_dict(row, &long, 6);
    assert_eq!(inflate_dict(&z, &long, row.len()).unwrap(), row);
    assert_eq!(inflate_dict(&z, &long[long.len() - WINDOW..], row.len()).unwrap(), row);
    // Wrong dictionary: an error or wrong bytes, never a panic; frames catch it by CRC.
    let other = noise(dict.len(), 22);
    if let Ok(out) = inflate_dict(&z, &other, row.len()) {
        assert_ne!(out, row);
    }
    // Empty data and empty dictionary.
    assert_eq!(inflate_dict(&deflate_dict(b"", &dict, 6), &dict, 0).unwrap(), b"");
    assert_eq!(inflate_dict(&deflate_dict(row, b"", 6), b"", row.len()).unwrap(), row);
}

#[test]
fn frames_carry_their_dictionary_id() {
    let dict = json_rows(100);
    let row = json_rows(3);
    let frame = pack_with(&row, 7, &dict);
    assert_eq!(dictionary_id(&frame), Some(7));
    assert_eq!(dictionary_id(&pack(&row)), Some(0));
    assert_eq!(dictionary_id(b"{}"), None);
    let lookup = |id: u8| (id == 7).then_some(&dict[..]);
    assert_eq!(unpack_with(&frame, 1 << 20, lookup).unwrap(), row);
    assert_eq!(text_with(&frame, 1 << 20, lookup).unwrap().as_bytes(), &row[..]);
    assert_eq!(unpack(&frame, 1 << 20), Err(Error::MissingDictionary(7)));
    let wrong = noise(dict.len(), 3);
    assert!(unpack_with(&frame, 1 << 20, |_| Some(&wrong[..])).is_err());
}

#[test]
fn prepared_decoding_matches_across_alternating_dictionaries_and_errors() {
    let (a, b) = (json_rows(40), noise(9_000, 5));
    let (da, db) = (Dictionary::new(&a), Dictionary::new(&b));
    let rows: Vec<Vec<u8>> = (0..30).map(|i| json_rows(3 + i % 5)[i * 7..].to_vec()).collect();
    for (i, row) in rows.iter().enumerate() {
        // Alternate dictionaries so the per-thread window is replaced and restored.
        let (d, raw) = if i % 3 == 0 { (&db, &b) } else { (&da, &a) };
        let mut z = Vec::new();
        deflate_prepared_into(row, d, 6, &mut z);
        let got = inflate_prepared(&z, d, 1 << 20).unwrap();
        assert_eq!(got, *row);
        assert_eq!(got, inflate_dict(&z, raw, 1 << 20).unwrap());
        assert_eq!(got.capacity(), got.len(), "exact-size values, not the window's capacity");
        assert_eq!(inflate_dict(&z, raw, 1 << 20).unwrap().capacity(), row.len());
        // A damaged stream fails and leaves the window intact for the next value.
        let mut bad = z.clone();
        bad.truncate(z.len() / 2);
        assert!(inflate_prepared(&bad, d, 1 << 20).is_err() || inflate_prepared(&bad, d, 1 << 20).unwrap() != *row);
        assert_eq!(inflate_prepared(&z, d, 1 << 20).unwrap(), *row);
        // Frames through the prepared path check length and CRC like the plain path.
        let frame = pack_prepared(row, 7, d);
        let lookup = |id: u8| (id == 7).then_some(d);
        assert_eq!(unpack_prepared(&frame, 1 << 20, lookup).unwrap(), *row);
        assert_eq!(unpack_prepared(&frame, 1 << 20, |_| None), Err(Error::MissingDictionary(7)));
        let mut corrupt = frame.clone();
        let last = corrupt.len() - 1;
        corrupt[last] ^= 1;
        assert!(unpack_prepared(&corrupt, 1 << 20, lookup).is_err());
    }
    assert!(text_prepared(b"plain text", 100, |_| None).unwrap() == "plain text");
}

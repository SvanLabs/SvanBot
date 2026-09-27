//! The decoder against streams zlib produced (fixtures from `tests/fixtures/make.py`): every block
//! type, strategy and level zlib emits, many small blocks, and a preset dictionary. The encoder is
//! held to zlib's size on the same inputs.

use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn read(name: &str) -> Vec<u8> {
    std::fs::read(fixtures().join(name)).unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

/// (case, input name) for every `CASE-NAME.z`.
fn streams() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = std::fs::read_dir(fixtures())
        .expect("fixtures directory")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|f| {
            let stem = f.strip_suffix(".z")?;
            let (case, name) = stem.split_once('-')?;
            Some((case.to_string(), name.to_string()))
        })
        .collect();
    out.sort();
    out
}

#[test]
fn every_zlib_stream_decodes_to_its_input() {
    let dict = read("dict.bin");
    let all = streams();
    assert!(all.len() >= 25, "fixtures missing: {all:?}");
    for (case, name) in &all {
        let stream = read(&format!("{case}-{name}.z"));
        let want = read(&format!("{name}.in"));
        let got = if case == "dict" { sv10_pack::inflate_dict(&stream, &dict, 1 << 20) } else { sv10_pack::inflate(&stream, 1 << 20) };
        assert_eq!(got.as_deref(), Ok(&want[..]), "{case}-{name}");
        // A limit one byte short of the output is refused, never truncated silently.
        if !want.is_empty() && case != "dict" {
            assert_eq!(sv10_pack::inflate(&stream, want.len() - 1), Err(sv10_pack::Error::TooLarge(want.len() - 1)), "{case}-{name} limit");
        }
    }
}

#[test]
fn every_truncation_of_a_zlib_stream_is_an_error() {
    for (case, name) in streams().into_iter().filter(|(c, _)| c != "dict") {
        let stream = read(&format!("{case}-{name}.z"));
        let want = read(&format!("{name}.in"));
        for cut in (0..stream.len()).step_by((stream.len() / 40).max(1)) {
            // A stream cut short either errors or (when the cut falls in the final byte's padding)
            // still yields exactly the input; it never yields something else.
            if let Ok(out) = sv10_pack::inflate(&stream[..cut], 1 << 20) {
                assert_eq!(out, want, "{case}-{name} cut at {cut}");
            }
        }
    }
}

#[test]
fn the_encoder_matches_zlib_size_at_its_default_level() {
    for name in ["rows", "long", "text", "runs"] {
        let input = read(&format!("{name}.in"));
        let zlib = read(&format!("default-{name}.z")).len();
        let ours = sv10_pack::deflate(&input, 6);
        assert_eq!(sv10_pack::inflate(&ours, 1 << 20).as_deref(), Ok(&input[..]));
        assert!(ours.len() * 100 <= zlib * 102, "{name}: ours {} bytes vs zlib {zlib}", ours.len());
    }
    let dict = read("dict.bin");
    for name in ["rows", "long", "text"] {
        let input = read(&format!("{name}.in"));
        let zlib = read(&format!("dict-{name}.z")).len();
        let ours = sv10_pack::deflate_dict(&input, &dict, 6);
        let prepared = {
            let mut out = Vec::new();
            sv10_pack::deflate_prepared_into(&input, &sv10_pack::Dictionary::new(&dict), 6, &mut out);
            out
        };
        assert_eq!(ours, prepared, "{name}: a prepared dictionary must give the same stream");
        assert_eq!(sv10_pack::inflate_dict(&ours, &dict, 1 << 20).as_deref(), Ok(&input[..]));
        assert!(ours.len() * 100 <= zlib * 102, "{name} with dictionary: ours {} bytes vs zlib {zlib}", ours.len());
    }
}

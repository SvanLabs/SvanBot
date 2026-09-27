//! zcli MODE LEVEL [DICT] < in > out: the codec on stdin/stdout, for the zlib cross-check in
//! `scripts/tests/test_pack.py` (MODE `d` deflates, `i` inflates, `p` packs a frame with dictionary
//! id 1 when DICT is given, `u` unpacks a frame).
use std::io::{Read, Write};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input).expect("stdin");
    let dict = a.get(3).map(|p| std::fs::read(p).expect("dictionary file")).unwrap_or_default();
    let level: u8 = a.get(2).and_then(|l| l.parse().ok()).unwrap_or(6);
    let out = match a.get(1).map(String::as_str) {
        Some("d") => sv10_pack::deflate_dict(&input, &dict, level),
        Some("i") => sv10_pack::inflate_dict(&input, &dict, 1 << 30).expect("inflate"),
        Some("p") if dict.is_empty() => sv10_pack::pack(&input),
        Some("p") => sv10_pack::pack_with(&input, 1, &dict),
        Some("u") => sv10_pack::unpack_with(&input, 1 << 30, |id| (id == 1).then_some(&dict[..])).expect("unpack"),
        _ => {
            eprintln!("usage: zcli d|i|p|u LEVEL [DICT]");
            std::process::exit(2);
        }
    };
    std::io::stdout().write_all(&out).expect("stdout");
}

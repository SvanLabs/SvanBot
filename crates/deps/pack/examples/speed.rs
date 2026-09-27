//! speed ROWS_FILE DICT: compress and decompress length-prefixed rows, report MB/s.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let buf = std::fs::read(&a[1]).unwrap();
    let dict = std::fs::read(&a[2]).unwrap();
    let mut rows = Vec::new();
    let mut i = 0;
    while i + 4 <= buf.len() {
        let n = u32::from_le_bytes(buf[i..i + 4].try_into().unwrap()) as usize;
        rows.push(&buf[i + 4..i + 4 + n]);
        i += 4 + n;
    }
    let total: usize = rows.iter().map(|r| r.len()).sum();
    for prepared in [false, true] {
        let t = std::time::Instant::now();
        let d = sv10_pack::Dictionary::new(&dict);
        let packed: Vec<Vec<u8>> =
            rows.iter().map(|r| if prepared { sv10_pack::pack_prepared(r, 1, &d) } else { sv10_pack::pack_with(r, 1, &dict) }).collect();
        let c = t.elapsed().as_secs_f64();
        let t = std::time::Instant::now();
        let mut n = 0;
        for p in &packed {
            n += sv10_pack::unpack_with(p, 1 << 24, |_| Some(&dict[..])).unwrap().len();
        }
        let dt = t.elapsed().as_secs_f64();
        let z: usize = packed.iter().map(|p| p.len()).sum();
        println!(
            "prepared={prepared}: {} rows, {:.1} MB -> {:.1} MB ({:.3}); pack {:.0} MB/s, unpack {:.0} MB/s ({n})",
            rows.len(),
            total as f64 / 1e6,
            z as f64 / 1e6,
            z as f64 / total as f64,
            total as f64 / 1e6 / c,
            total as f64 / 1e6 / dt
        );
    }
}

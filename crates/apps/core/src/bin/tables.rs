//! `tables build [dir]` — precompute exact flop and turn board-strength tables (see `tables`).
//! `tables check [dir]` — verify the files and spot-check rows against direct computation.
//! `tables preflop-data` — regenerate the compiled-in preflop tables (`crates/equity/src/preflop_data.rs`).

use rayon::prelude::*;
use sv10_core::equity::exact_strengths;
use sv10_core::range::NUM_COMBOS;
use sv10_core::tables::{Table, canonical_boards, cards_of, table_path, tables_dir};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(2).map(std::path::PathBuf::from).unwrap_or_else(tables_dir);
    match args.get(1).map(String::as_str) {
        Some("build") => {
            std::fs::create_dir_all(&dir).expect("create tables dir");
            for len in [3usize, 4] {
                let t0 = std::time::Instant::now();
                let keys = canonical_boards(len);
                let rows: Vec<f32> = keys.par_iter().flat_map_iter(|&k| exact_strengths(&cards_of(k))).collect();
                let path = table_path(&dir, len);
                Table::new(keys.clone(), rows).write(&path, len).expect("write table");
                println!("{}: {} boards in {:.1}s", path.display(), keys.len(), t0.elapsed().as_secs_f64());
            }
        }
        Some("check") => {
            for len in [3usize, 4] {
                let path = table_path(&dir, len);
                let t0 = std::time::Instant::now();
                let t = Table::read(&path, len).unwrap_or_else(|e| panic!("{e}"));
                let keys = canonical_boards(len);
                for &k in keys.iter().step_by(keys.len() / 7) {
                    let b = cards_of(k);
                    let (got, want) = (t.lookup(&b).unwrap(), exact_strengths(&b));
                    assert!((0..NUM_COMBOS).all(|i| (got[i] - want[i]).abs() < 1e-6), "row mismatch for {b:?}");
                }
                println!("{}: ok ({} boards, loaded and spot-checked in {:.2}s)", path.display(), keys.len(), t0.elapsed().as_secs_f64());
            }
        }
        Some("preflop-data") => {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../libs/equity/src/preflop_data.rs");
            assert!(path.exists(), "{} not found: the equity crate moved?", path.display());
            std::fs::write(&path, sv10_core::preflop::render_data()).expect("write preflop_data.rs");
            println!("{}: regenerated", path.display());
        }
        _ => eprintln!("usage: tables build|check [dir] | preflop-data"),
    }
}

//! `review storage` (0229): the compressed columns — rows still text, free pages, the data format
//! and the last compaction pass — over both `svanbot10.db` and `history.db`.
//!
//! Split out of `bin/review.rs` when the `unrecorded` command (#739) pushed that file past the
//! 500-line limit; a pure move, the printed lines are byte-identical.

use anyhow::Result;
use std::path::Path;
use sv10_store::store::Store;

/// Print the storage report for the checkout rooted at `root`.
pub fn report(store: &Store, root: &Path) -> Result<()> {
    let history = crate::history::HistoryDb::open(&root.join("artifacts").join("history.db"))?;
    let mb = |b: u64| format!("{:.1} MB", b as f64 / 1e6);
    let size = |f: &str| std::fs::metadata(root.join("artifacts").join(f)).map(|m| m.len()).unwrap_or(0);
    println!(
        "data format {} (this build reads {})",
        sv10_store::packed::data_format(&root.join("artifacts")),
        sv10_store::packed::DATA_FORMAT
    );
    println!("svanbot10.db {}, {} in free pages", mb(size("svanbot10.db")), mb(store.free_bytes()?));
    for (family, n) in store.text_rows()? {
        println!("  {family:20} {n} rows still text");
    }
    println!("history.db   {}, {} in free pages", mb(size("history.db")), mb(history.free_bytes()?));
    for (family, n) in history.text_rows()? {
        println!("  {family:20} {n} rows still text");
    }
    match store.get_kv(crate::compaction::STATUS_KEY)? {
        Some(status) => println!("last compaction pass: {status}"),
        None => println!("no compaction pass recorded yet (the fleet starts one 90 s after it starts)"),
    }
    Ok(())
}

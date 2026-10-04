//! The data format a build reads and the one the databases hold (`build_data_format`,
//! `store_data_format`): a build that cannot read the stored rows is never installed.

use crate::gitops;
use std::path::Path;

/// The store codec file whose `DATA_FORMAT` constant names the format a build reads.
const CODEC: &str = "crates/libs/store/src/packed.rs";

/// The `DATA_FORMAT` of `commit`'s store codec; 1 for a build from before the codec.
pub fn of_build(root: &Path, commit: &str) -> u64 {
    let Some(text) = gitops::show(root, commit, CODEC) else { return 1 };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("pub const DATA_FORMAT: u32 = ").and_then(|r| r.strip_suffix(';'))
            && !rest.is_empty()
            && rest.bytes().all(|b| b.is_ascii_digit())
        {
            return rest.parse().unwrap_or(1);
        }
    }
    1
}

/// The highest format written next to the databases (`artifacts/data-format`); 1 when absent or odd.
pub fn of_store(root: &Path) -> u64 {
    let text = std::fs::read_to_string(root.join("artifacts/data-format")).unwrap_or_default();
    let digits: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) { digits.parse().unwrap_or(1) } else { 1 }
}

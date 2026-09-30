//! Archive verification: does the directory hold what its sealed manifest says (#603).
//!
//! Split out of `archive.rs`, which `check-file-size.sh` caps at its baseline.

use super::*;

/// Problems with the archive at `dir`: manifest seal, stored sizes and hashes, and with `deep`
/// the decompressed content hashes. Empty means verified.
pub fn verify(dir: &Path, deep: bool) -> Vec<String> {
    let mut problems = Vec::new();
    let manifest = match load_manifest(dir) {
        Ok(m) => m,
        Err(e) => return vec![format!("{}: {e}", dir.display())],
    };
    for e in &manifest.files {
        let Ok(path) = crate::paths::confined(dir, &e.name) else {
            problems.push(format!("{}: the name leaves the archive directory", e.name));
            continue;
        };
        let Ok(meta) = std::fs::metadata(&path) else {
            problems.push(format!("{}: missing", e.name));
            continue;
        };
        if meta.len() != e.bytes {
            problems.push(format!("{}: {} bytes, manifest says {}", e.name, meta.len(), e.bytes));
            continue;
        }
        match crate::integrity::file_sha256(&path) {
            Ok(h) if h == e.sha256 => {}
            Ok(_) => {
                problems.push(format!("{}: SHA-256 mismatch", e.name));
                continue;
            }
            Err(err) => {
                problems.push(format!("{}: {err}", e.name));
                continue;
            }
        }
        if deep && e.role != Role::RepoBundle {
            match zstd_content_sha(&path) {
                Ok((h, n)) if h == e.raw_sha256 && n == e.raw_bytes => {}
                Ok(_) => problems.push(format!("{}: decompressed content does not match", e.name)),
                Err(err) => problems.push(format!("{}: {err}", e.name)),
            }
        }
    }
    problems
}

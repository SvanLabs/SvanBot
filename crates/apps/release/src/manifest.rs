//! `SHA256SUMS` manifests: the file the installer writes next to a snapshot and checks before it
//! trusts one. The format is `sha256sum`'s (`<64 hex>  <path>`), so either side can verify the other's.

use crate::error::{ReleaseError, Result};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The two trees a release holds, relative to the set's base.
pub const TREES: [&str; 2] = ["target/release", "web/dist"];

/// One manifest line.
#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub hash: String,
    pub path: String,
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `manifest_paths_are_safe`: every line is a hash and a path under one of the two trees, with no
/// `..` and no leading `/`. The line is split like `read -r hash path`: the hash is the first word,
/// the path the rest with its surrounding blanks removed and one leading `*` (binary mode) dropped.
pub fn parse_safe(text: &str) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_start_matches([' ', '\t']);
        let (hash, rest) = line.split_once([' ', '\t']).unwrap_or((line, ""));
        if !is_hex64(hash) {
            return Err(ReleaseError::ManifestHashEntry);
        }
        let path = rest.trim_matches([' ', '\t']);
        let path = path.strip_prefix('*').unwrap_or(path);
        if !(path.starts_with("target/release/") || path.starts_with("web/dist/")) || path.contains("..") || path.starts_with('/') {
            return Err(ReleaseError::ManifestPathUnsafe(path.to_string()));
        }
        out.push(Entry { hash: hash.to_string(), path: path.to_string() });
    }
    Ok(out)
}

/// Hex SHA-256 of a file, read in blocks.
pub fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = sv10_digest::Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(sv10_digest::hex(hasher.finalize()))
}

/// Every regular file under the two trees of `base`, as base-relative paths (`find … -type f`).
pub fn actual_paths(base: &Path) -> Result<BTreeSet<String>> {
    fn walk(dir: &Path, rel: &str, out: &mut BTreeSet<String>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = format!("{rel}/{name}");
            let ty = entry.file_type()?;
            if ty.is_dir() {
                walk(&entry.path(), &rel, out)?;
            } else if ty.is_file() {
                out.insert(rel);
            }
        }
        Ok(())
    }
    let mut out = BTreeSet::new();
    for tree in TREES {
        walk(&base.join(tree), tree, &mut out).map_err(|e| ReleaseError::Io(format!("find: {}: {e}", base.join(tree).display())))?;
    }
    Ok(out)
}

/// `verify_tree_against_manifest`: the manifest lists exactly the files under the trees, and every
/// listed file hashes to its line (`sha256sum --check --strict`: a malformed line fails too).
pub fn verify_tree(base: &Path, manifest_text: &str) -> Result<()> {
    let mut listed = BTreeSet::new();
    let mut checks: Vec<(String, String)> = Vec::new();
    for line in manifest_text.lines() {
        let (hash, rest) = (line.get(..64).unwrap_or(""), line.get(64..).unwrap_or(""));
        let strict = is_hex64(hash) && matches!(rest.as_bytes(), [b' ', b' ' | b'*', ..]);
        let path = if strict { &rest[2..] } else { line };
        listed.insert(path.to_string());
        if strict {
            checks.push((hash.to_ascii_lowercase(), path.to_string()));
        }
    }
    if listed != actual_paths(base)? {
        return Err(ReleaseError::ManifestIncomplete);
    }
    if checks.len() != manifest_text.lines().count() {
        return Err(ReleaseError::ManifestMismatch);
    }
    for (hash, path) in checks {
        match hash_file(&base.join(&path)) {
            Ok(actual) if actual == hash => {}
            _ => return Err(ReleaseError::ManifestMismatch),
        }
    }
    Ok(())
}

/// `reject_special_files`: no symlink and nothing that is neither a regular file nor a directory
/// anywhere under `dir`, `dir` itself included. Returns the first offender's path.
pub fn find_special(dir: &Path) -> std::io::Result<Option<PathBuf>> {
    let meta = std::fs::symlink_metadata(dir)?;
    let ty = meta.file_type();
    if ty.is_symlink() || !(ty.is_file() || ty.is_dir()) {
        return Ok(Some(dir.to_path_buf()));
    }
    if ty.is_dir() {
        for entry in std::fs::read_dir(dir)? {
            if let Some(bad) = find_special(&entry?.path())? {
                return Ok(Some(bad));
            }
        }
    }
    Ok(None)
}

/// Refuse `dir` when it holds a symlink or special file.
pub fn reject_special(dir: &Path) -> Result<()> {
    match find_special(dir) {
        Ok(None) => Ok(()),
        Ok(Some(bad)) => Err(ReleaseError::SpecialFile(bad.display().to_string())),
        Err(e) => Err(ReleaseError::Io(format!("find: {}: {e}", dir.display()))),
    }
}

/// `write_manifest`: `SHA256SUMS` in `base` listing every file under the two trees in byte order, in
/// `sha256sum`'s format, then checked against the files it was just made from.
pub fn write(base: &Path) -> Result<()> {
    let mut text = String::new();
    for path in actual_paths(base)? {
        if path.chars().any(char::is_whitespace) {
            return Err(ReleaseError::ManifestWhitespace(path));
        }
        let hash = hash_file(&base.join(&path)).map_err(|e| ReleaseError::Io(format!("cannot hash {path}: {e}")))?;
        text.push_str(&format!("{hash}  {path}\n"));
    }
    if text.is_empty() {
        return Err(ReleaseError::ManifestEmpty);
    }
    let file = base.join("SHA256SUMS");
    std::fs::write(&file, &text).map_err(|e| ReleaseError::Io(format!("cannot write {}: {e}", file.display())))?;
    verify_tree(base, &text)
}

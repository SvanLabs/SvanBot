//! The file operations the swap and the restore are built from, each with the semantics of the shell
//! command it replaces: `rm -rf`, `cp -a` (modes and times kept, nothing followed) and `mktemp -d`.

use crate::error::{ReleaseError, Result};
use std::fs::FileTimes;
use std::path::{Path, PathBuf};

fn io(what: &str, path: &Path, e: std::io::Error) -> ReleaseError {
    ReleaseError::Io(format!("{what} {}: {e}", path.display()))
}

/// `rm -rf -- path`: a missing path is fine, a symlink is removed rather than followed.
pub fn remove_all(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io("cannot inspect", path, e)),
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path).map_err(|e| io("cannot remove", path, e)),
        Ok(_) => std::fs::remove_file(path).map_err(|e| io("cannot remove", path, e)),
    }
}

/// `mv from to`, as an error that names both ends.
pub fn rename(from: &Path, to: &Path) -> Result<()> {
    std::fs::rename(from, to).map_err(|e| ReleaseError::Io(format!("cannot move {} to {}: {e}", from.display(), to.display())))
}

/// `cp -a from/. to/`: the contents of `from` copied into the existing directory `to`, keeping
/// permission bits and modification times. A symlink or special file is an error (release trees are
/// checked for them before they are copied, so meeting one here means the tree changed underneath).
pub fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    for entry in std::fs::read_dir(from).map_err(|e| io("cannot read", from, e))? {
        let entry = entry.map_err(|e| io("cannot read", from, e))?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        let meta = std::fs::symlink_metadata(&src).map_err(|e| io("cannot inspect", &src, e))?;
        if meta.is_dir() {
            std::fs::create_dir(&dst).map_err(|e| io("cannot create", &dst, e))?;
            copy_tree(&src, &dst)?;
            std::fs::set_permissions(&dst, meta.permissions()).map_err(|e| io("cannot set the mode of", &dst, e))?;
        } else if meta.is_file() {
            std::fs::copy(&src, &dst).map_err(|e| io("cannot copy", &src, e))?;
            let file = std::fs::File::options().write(true).open(&dst).map_err(|e| io("cannot open", &dst, e))?;
            let modified = meta.modified().map_err(|e| io("cannot read the time of", &src, e))?;
            file.set_times(FileTimes::new().set_modified(modified)).map_err(|e| io("cannot set the time of", &dst, e))?;
        } else {
            return Err(ReleaseError::SpecialFile(src.display().to_string()));
        }
    }
    Ok(())
}

/// `mktemp -d <parent>/<prefix>XXXXXX`: a new directory with six random characters after `prefix`.
pub fn make_temp_dir(parent: &Path, prefix: &str) -> Result<PathBuf> {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    for _ in 0..100 {
        let id = sv10_rt::uuid_v4();
        let suffix: String =
            id.bytes().filter(u8::is_ascii_hexdigit).take(6).map(|b| ALPHABET[usize::from(b) % ALPHABET.len()] as char).collect();
        let path = parent.join(format!("{prefix}{suffix}"));
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(io("cannot create", &path, e)),
        }
    }
    Err(ReleaseError::Io(format!("cannot create a unique directory under {}", parent.display())))
}

/// A staging directory that is removed when it goes out of scope (the shell's EXIT trap). A kill
/// skips it, as it skips the trap; the swap journal is what covers that case.
pub struct Scratch(pub PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = remove_all(&self.0);
    }
}

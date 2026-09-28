//! Read-only shared memory maps of whole files.
//!
//! Part of the svanbot10 workspace (0228). The 96 MB board-strength tables were read into every
//! process's heap (fleet head, split-fleet workers, learner, analyst, tools, every test binary): one
//! private copy each, and twice the size at peak while loading. A read-only `MAP_SHARED` mapping of
//! the same file is one copy in the page cache, shared by every process that maps it, paged in on
//! demand and dropped under memory pressure without swap.
//!
//! Safety contract (why the `unsafe` here is sound): the mapping is `PROT_READ`, so this process
//! cannot write it; it lives exactly as long as the [`Mmap`]; and the files mapped here are replaced
//! only by writing a new file and renaming it over the old one (see `sv10_equity::tables::Table::write`),
//! which leaves this mapping on the old inode. Truncating a mapped file in place would make later
//! reads fault (SIGBUS); nothing in this workspace does that.

#![warn(missing_docs)]
// The purpose of this crate: mmap(2)/munmap(2)/madvise(2) and viewing the mapping as a slice.
#![allow(unsafe_code)]

use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

/// A read-only, shared mapping of a whole file.
pub struct Mmap {
    ptr: *const u8,
    len: usize,
}

// SAFETY: the mapping is immutable for its whole life (PROT_READ, never remapped), so shared
// references to its bytes may cross and be used from any thread.
unsafe impl Send for Mmap {}
// SAFETY: as above; `&Mmap` only ever yields `&[u8]`.
unsafe impl Sync for Mmap {}

impl Mmap {
    /// Map `path` read-only and shared. An empty file maps to an empty slice without a mapping.
    pub fn open(path: &Path) -> io::Result<Mmap> {
        let file = File::open(path)?;
        let len =
            usize::try_from(file.metadata()?.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "file too large to map"))?;
        if len == 0 {
            // Dangling but aligned for any view this type offers (an empty `&[f32]` included).
            return Ok(Mmap { ptr: std::ptr::NonNull::<f32>::dangling().as_ptr() as *const u8, len: 0 });
        }
        // SAFETY: a fresh read-only mapping of an open file descriptor; the kernel picks the address.
        // The descriptor may be closed afterwards (the mapping keeps the file referenced).
        let ptr = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, file.as_raw_fd(), 0) };
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(Mmap { ptr: ptr as *const u8, len })
    }

    /// The mapped bytes.
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: `ptr` is a live PROT_READ mapping of `len` bytes (or dangling with `len == 0`),
        // valid for the lifetime of `self`.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the file was empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// `count` little-endian `f32` values starting at byte `offset`, viewed in place. `None` when the
    /// range is out of bounds, misaligned for `f32`, or the target is big-endian (the files are
    /// little-endian; a big-endian caller decodes a copy instead).
    pub fn f32s(&self, offset: usize, count: usize) -> Option<&[f32]> {
        if cfg!(target_endian = "big") {
            return None;
        }
        let end = offset.checked_add(count.checked_mul(4)?)?;
        if end > self.len || !(self.ptr as usize + offset).is_multiple_of(align_of::<f32>()) {
            return None;
        }
        // SAFETY: in bounds and aligned (checked above); every bit pattern is a valid f32; the
        // bytes are immutable for the lifetime of `self` and in the target's (little) endianness.
        Some(unsafe { std::slice::from_raw_parts(self.ptr.add(offset) as *const f32, count) })
    }
}

impl Drop for Mmap {
    fn drop(&mut self) {
        if self.len > 0 {
            // SAFETY: unmapping the mapping this value owns, exactly once.
            unsafe {
                libc::munmap(self.ptr as *mut libc::c_void, self.len);
            }
        }
    }
}

impl std::fmt::Debug for Mmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mmap").field("len", &self.len).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("sv10-mmap-{}-{name}", std::process::id()));
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn maps_bytes_and_views_aligned_floats() {
        let vals = [1.5f32, -2.25, 3.0e9, f32::MIN_POSITIVE];
        let mut bytes = vec![0xAAu8; 8];
        for v in vals {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        let p = temp("floats", &bytes);
        let m = Mmap::open(&p).unwrap();
        assert_eq!(m.bytes(), &bytes[..]);
        assert_eq!(m.len(), bytes.len());
        assert_eq!(m.f32s(8, 4).unwrap(), &vals);
        assert!(m.f32s(9, 1).is_none(), "misaligned");
        assert!(m.f32s(8, 5).is_none(), "out of bounds");
        assert!(m.f32s(usize::MAX - 2, 1).is_none(), "overflow");
        std::fs::remove_file(&p).unwrap();
        // Unlinked while mapped: the mapping keeps the old inode.
        assert_eq!(m.bytes()[8..12], 1.5f32.to_le_bytes());
    }

    #[test]
    fn replacing_by_rename_leaves_the_mapping_intact() {
        let p = temp("rename", b"old contents");
        let m = Mmap::open(&p).unwrap();
        let new = p.with_extension("tmp");
        std::fs::write(&new, b"new contents, longer").unwrap();
        std::fs::rename(&new, &p).unwrap();
        assert_eq!(m.bytes(), b"old contents");
        assert_eq!(Mmap::open(&p).unwrap().bytes(), b"new contents, longer");
        std::fs::remove_file(&p).unwrap();
    }

    #[test]
    fn empty_and_missing_files() {
        let p = temp("empty", b"");
        let m = Mmap::open(&p).unwrap();
        assert!(m.is_empty());
        assert_eq!(m.bytes(), b"");
        assert!(m.f32s(0, 0).is_some());
        std::fs::remove_file(&p).unwrap();
        assert!(Mmap::open(&p).is_err());
    }

    #[test]
    fn mappings_are_shared_between_threads() {
        let p = temp("threads", &[7u8; 4096]);
        let m = std::sync::Arc::new(Mmap::open(&p).unwrap());
        let sums: Vec<u64> = (0..4)
            .map(|_| {
                let m = m.clone();
                std::thread::spawn(move || m.bytes().iter().map(|&b| b as u64).sum::<u64>())
            })
            .map(|h| h.join().unwrap())
            .collect();
        assert!(sums.iter().all(|&s| s == 7 * 4096));
        std::fs::remove_file(&p).unwrap();
    }
}

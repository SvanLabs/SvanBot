//! Small runtime helpers built in-house instead of pulled in as dependencies: `.env` loading,
//! random v4 identifiers, and filesystem free space.

#![warn(missing_docs)]
// The only unsafe code in the workspace: process environment setup and statvfs(2).
#![allow(unsafe_code)]

use std::path::Path;

/// Load `KEY=VALUE` lines from a `.env` file into the process environment without overriding
/// variables that are already set. Supports `export ` prefixes, `#` comments, blank lines, and
/// single- or double-quoted values (double quotes understand `\n`, `\"` and `\\`).
pub fn load_env_file(path: &Path) -> std::io::Result<usize> {
    let text = std::fs::read_to_string(path)?;
    let mut loaded = 0;
    for (key, value) in parse_env(&text) {
        if std::env::var_os(&key).is_none() {
            // SAFETY: called once at startup before any other thread reads the environment.
            unsafe { std::env::set_var(&key, value) };
            loaded += 1;
        }
    }
    Ok(loaded)
}

/// Parse `.env` text into (key, value) pairs: `export` prefixes, comments, single and double quotes
/// (with escapes in double quotes); invalid lines are skipped.
pub fn parse_env(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, rest)) = line.split_once('=') else { continue };
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let rest = rest.trim();
        let value = if let Some(inner) = rest.strip_prefix('"').and_then(|r| r.rfind('"').map(|i| &r[..i])) {
            let mut v = String::with_capacity(inner.len());
            let mut chars = inner.chars();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    match chars.next() {
                        Some('n') => v.push('\n'),
                        Some(o) => v.push(o),
                        None => v.push('\\'),
                    }
                } else {
                    v.push(c);
                }
            }
            v
        } else if let Some(inner) = rest.strip_prefix('\'').and_then(|r| r.rfind('\'').map(|i| &r[..i])) {
            inner.to_string()
        } else {
            // Unquoted: an inline comment starts at " #".
            rest.split(" #").next().unwrap_or("").trim().to_string()
        };
        out.push((key.to_string(), value));
    }
    out
}

/// Whether `value` can be written to `.env` unquoted and read back identically by [`parse_env`] and
/// by a shell `.` (letters, digits and `_ . , / : @ + = -`, at most 512 bytes).
pub fn env_value_is_safe(value: &str) -> bool {
    value.len() <= 512 && value.chars().all(|c| c.is_ascii_alphanumeric() || "_.,/:@+=-".contains(c))
}

/// `.env` text with `updates` applied: `Some(v)` replaces the first assignment of the key in place
/// (later duplicates are dropped) or appends it; `None` removes every assignment. Comments, blank
/// lines, ordering and every other key are preserved. Panics on unsafe values (see
/// [`env_value_is_safe`]); callers validate first.
pub fn update_env_text(text: &str, updates: &[(&str, Option<&str>)]) -> String {
    for (k, v) in updates {
        assert!(k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "bad key {k}");
        assert!(v.is_none_or(env_value_is_safe), "unsafe value for {k}");
    }
    let key_of = |line: &str| -> Option<String> {
        let t = line.trim();
        if t.starts_with('#') {
            return None;
        }
        let t = t.strip_prefix("export ").unwrap_or(t);
        t.split_once('=').map(|(k, _)| k.trim().to_string())
    };
    let mut written: Vec<&str> = Vec::new();
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        match key_of(line).and_then(|k| updates.iter().find(|(u, _)| *u == k)) {
            Some((k, Some(v))) if !written.contains(k) => {
                out.push(format!("{k}={v}"));
                written.push(k);
            }
            Some(_) => {}
            None => out.push(line.to_string()),
        }
    }
    for (k, v) in updates {
        if let Some(v) = v
            && !written.contains(k)
        {
            out.push(format!("{k}={v}"));
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Replace a private file (such as `.env`) atomically: write a mode-600 temporary in the same
/// directory, fsync, then rename over `path`.
pub fn write_private_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    write_atomic_mode(path, contents, Some(0o600))
}

/// Replace `path` atomically and durably: write a temporary in the same directory, fsync it, rename
/// it over `path`, then fsync the directory. Meaningful for a file a later step verifies or reads
/// back (issue #323): a plain `fs::write` reports success from the page cache, so a write the disk
/// dropped is discovered when the file is read for real, and an interrupted one leaves half a file
/// under the name a reader trusts. Concurrent callers own separate temporary files: the last
/// rename wins, with one complete payload. Failures remove only this call's temporary.
pub fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    write_atomic_mode(path, contents, None)
}

fn write_atomic_mode(path: &Path, contents: &str, mode: Option<u32>) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "file name required"))?;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    // Each call owns its staging file (#501): concurrent saves or destinations sharing a stem
    // must never truncate or rename another writer's bytes. create_new also rejects stale files.
    let (tmp, mut f) = loop {
        let mut staging = name.to_os_string();
        staging.push(format!(".{}.{}.tmp-write", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let tmp = path.with_file_name(staging);
        match opts.open(&tmp) {
            Ok(f) => break (tmp, f),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let result = (|| {
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)?;
        // The rename itself must be durable: without syncing the directory, a power cut can leave the
        // old `.env` (or the old data-format marker) in place after a write we reported as done. This
        // writes the API keys, so a lost rename means a fleet that restarts with none (0255).
        sync_dir(path)
    })();
    if result.is_err() {
        // Best-effort cleanup only: preserve the original write/rename/fsync failure for callers.
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// `fsync` one file. On Linux this is where a failed write to the disk surfaces (`EIO`): data the
/// kernel could not store is otherwise reported as written and reads back as zeros once its
/// cached pages are gone (2026-09-27: the backup mirror's seals on a failing HDD, 0308).
pub fn sync_file(path: &Path) -> std::io::Result<()> {
    std::fs::OpenOptions::new().write(true).open(path)?.sync_all()
}

/// Drop `path`'s cached pages, so the next read comes from the disk rather than from memory.
/// Only clean pages are dropped: call it after [`sync_file`]. A verification that reads a file it
/// just wrote without this reads the page cache and cannot see a failed write.
#[cfg(unix)]
pub fn evict_cache(path: &Path) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let f = std::fs::File::open(path)?;
    // SAFETY: `f` is an open descriptor for the whole call; POSIX_FADV_DONTNEED is advisory and
    // only affects this file's cached pages.
    let rc = unsafe { libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };
    if rc == 0 { Ok(()) } else { Err(std::io::Error::from_raw_os_error(rc)) }
}

/// Copy `from` to `to` and make the copy durable: the data synced (a failed disk write is an
/// error here, not a silent zero later) and the directory entry synced.
pub fn copy_durable(from: &Path, to: &Path) -> std::io::Result<u64> {
    let n = std::fs::copy(from, to)?;
    sync_file(to)?;
    sync_dir(to)?;
    Ok(n)
}

/// `fsync` the directory holding `path`, so a rename into it survives a power cut.
pub fn sync_dir(path: &Path) -> std::io::Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    // SAFETY: `dir` is a valid path; `File::open` on a directory is the documented way to get a
    // handle that can be fsynced on Linux, and it is read-only.
    std::fs::File::open(dir)?.sync_all()
}

/// Remove the file at `path`, counting one that is not there as success: every caller uses this to
/// clear the way for a write, and a file that is already gone has already cleared it. Any other
/// failure is returned rather than discarded — the caller is about to write at `path`, and a file
/// that survived this is a file the write lands in (issue #326).
pub fn remove_stale_file(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// [`remove_stale_file`] for a whole tree: the staging directory an interrupted run left behind is
/// cleared before the next run stages into it, and a tree that cannot be cleared is an error rather
/// than something to write on top of. A path that is not a directory is an error too.
pub fn remove_stale_dir(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// A random RFC 4122 version-4 identifier, lowercase hyphenated.
pub fn uuid_v4() -> String {
    let mut b: [u8; 16] = sv10_rng::os_bytes();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = sv10_digest::hex(b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// Free bytes available to unprivileged users on the filesystem holding `path`.
#[cfg(unix)]
pub fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: statvfs is a plain C struct of integers, for which all-zero bytes are a valid value.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `st` a writable statvfs.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

#[cfg(not(unix))]
pub fn free_bytes(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod durable_tests {
    use super::*;

    #[test]
    fn concurrent_atomic_writes_to_same_stem_files_keep_their_own_contents() {
        let dir = std::env::temp_dir().join(format!("sv10-rt-concurrent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
        let writes: Vec<_> = ["snapshot.json", "snapshot.txt"]
            .into_iter()
            .enumerate()
            .map(|(i, name)| {
                let path = dir.join(name);
                let gate = gate.clone();
                std::thread::spawn(move || {
                    let contents = char::from(b'A' + i as u8).to_string().repeat(10_000_000);
                    gate.wait();
                    write_atomic(&path, &contents).expect("distinct destinations must not share a staging file");
                    assert!(std::fs::read_to_string(&path).unwrap() == contents, "each destination must hold its own whole payload");
                })
            })
            .collect();
        for write in writes {
            write.join().unwrap();
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_atomic_write_never_reuses_a_preexisting_staging_file() {
        let dir = std::env::temp_dir().join(format!("sv10-rt-unowned-staging-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("snapshot.json");
        let unowned = path.with_extension("tmp-write");
        std::fs::write(&unowned, "another writer owns these bytes").unwrap();
        write_atomic(&path, "our complete snapshot").unwrap();
        assert_eq!(std::fs::read_to_string(&unowned).unwrap(), "another writer owns these bytes");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "our complete snapshot");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn concurrent_atomic_writes_to_one_destination_leave_one_complete_payload() {
        let dir = std::env::temp_dir().join(format!("sv10-rt-shared-destination-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("snapshot.json");
        let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
        std::thread::scope(|scope| {
            for byte in *b"AB" {
                let (path, gate) = (&path, &gate);
                scope.spawn(move || {
                    let contents = char::from(byte).to_string().repeat(1_000_000);
                    gate.wait();
                    write_atomic(path, &contents).expect("each writer owns its staging file");
                });
            }
        });
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 1_000_000);
        assert!(bytes.iter().all(|b| *b == bytes[0]), "one whole write must win, without mixed bytes");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_atomic_write_cleans_up_its_temporary_after_a_failed_rename() {
        let dir = std::env::temp_dir().join(format!("sv10-rt-failed-rename-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("destination")).unwrap();
        assert!(write_atomic(&dir.join("destination"), "cannot replace a directory").is_err());
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1, "only the original destination remains");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_durable_copy_reads_back_from_disk_after_eviction() {
        let dir = std::env::temp_dir().join(format!("sv10-rt-durable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::write(&a, b"sealed 64 bytes of hex would go here").unwrap();
        assert_eq!(copy_durable(&a, &b).unwrap(), 36);
        evict_cache(&b).unwrap();
        assert_eq!(std::fs::read(&b).unwrap(), std::fs::read(&a).unwrap());
        assert!(sync_file(&dir.join("missing")).is_err(), "a missing file is an error, never a silent pass");
    }

    #[test]
    fn an_atomic_write_leaves_the_whole_new_file_and_no_temporary() {
        // #323: the snapshot a `review season-snapshot` run reports as written is read back by
        // `season-compare` later, so what the report claims has to be what the disk holds.
        let dir = std::env::temp_dir().join(format!("sv10-rt-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("snapshot.json");
        write_atomic(&path, "{\"first\":1}").unwrap();
        write_atomic(&path, "{\"second\":2}").unwrap();
        evict_cache(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"second\":2}");
        let left: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, vec!["snapshot.json".to_string()], "the temporary is renamed, not left beside it");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn removing_what_is_already_gone_is_success_and_what_stays_is_an_error() {
        let dir = std::env::temp_dir().join(format!("sv10-rt-remove-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("tree")).unwrap();
        std::fs::write(dir.join("tree/held"), b"x").unwrap();
        std::fs::write(dir.join("gone"), b"x").unwrap();
        std::fs::write(dir.join("plain"), b"x").unwrap();
        // A path that is not there is already what the caller asked for.
        remove_stale_file(&dir.join("never-there")).unwrap();
        remove_stale_dir(&dir.join("also-never-there")).unwrap();
        remove_stale_file(&dir.join("gone")).unwrap();
        assert!(!dir.join("gone").exists());
        // A tree is not a file and a file is not a tree: each mix-up is a real failure, returned
        // rather than reported as done, because the caller is about to write at that path.
        assert!(remove_stale_file(&dir.join("tree")).is_err(), "a directory is not removed as a file");
        assert!(remove_stale_dir(&dir.join("plain")).is_err(), "a file is not removed as a directory");
        remove_stale_dir(&dir.join("tree")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_parsing_covers_quotes_comments_and_exports() {
        let text =
            "# comment\nA=1\nexport B = two \nC=\"x \\\"q\\\" \\n y\"\nD='raw # not comment'\nE=val # trailing\n bad line\n9X=nope\nF=\n";
        let got = parse_env(text);
        let want: Vec<(String, String)> =
            [("A", "1"), ("B", "two"), ("C", "x \"q\" \n y"), ("D", "raw # not comment"), ("E", "val"), ("9X", "nope"), ("F", "")]
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
        assert_eq!(got, want);
    }

    #[test]
    fn env_updates_keep_comments_order_and_other_keys() {
        let text = "# bots\nSVANBOT_API_KEY=old\nexport BOT_2_NAME=Two # inline\nOTHER=\"x y\"\nSVANBOT_API_KEY=dup\n";
        let out = update_env_text(text, &[("SVANBOT_API_KEY", Some("new_key-1")), ("BOT_2_NAME", None), ("BOT_3_NAME", Some("Three"))]);
        assert_eq!(out, "# bots\nSVANBOT_API_KEY=new_key-1\nOTHER=\"x y\"\nBOT_3_NAME=Three\n");
        let parsed = parse_env(&out);
        assert!(parsed.contains(&("OTHER".into(), "x y".into())) && parsed.contains(&("SVANBOT_API_KEY".into(), "new_key-1".into())));
        assert!(
            env_value_is_safe("op_live-AbC.123:4=")
                && !env_value_is_safe("a b")
                && !env_value_is_safe("x\nEVIL=1")
                && !env_value_is_safe("$(rm)")
        );
    }

    #[test]
    fn private_writes_are_mode_600() {
        let dir = std::env::temp_dir().join(format!("sv10-rt-env-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(".env");
        write_private_atomic(&p, "A=1\n").unwrap();
        write_private_atomic(&p, "A=2\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "A=2\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn uuid_v4_shape() {
        let a = uuid_v4();
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "4");
        assert!(matches!(&a[19..20], "8" | "9" | "a" | "b"));
        assert_ne!(a, uuid_v4());
    }

    #[test]
    fn free_bytes_matches_df() {
        let ours = free_bytes(Path::new("/")).unwrap();
        let out = std::process::Command::new("df").arg("-Pk").arg("/").output().unwrap();
        let df =
            String::from_utf8_lossy(&out.stdout).lines().nth(1).unwrap().split_whitespace().nth(3).unwrap().parse::<u64>().unwrap() * 1024;
        assert!((ours as i64 - df as i64).abs() < 64 * 1024 * 1024, "statvfs {ours} df {df}");
    }
}

#[cfg(test)]
mod env_file_tests {
    /// The project's own `.env` (if present) parses into well-formed keys with non-empty API keys.
    #[test]
    fn project_env_file_parses() {
        // The workspace root is the nearest ancestor holding Cargo.lock (robust to crate moves).
        let root =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().find(|a| a.join("Cargo.lock").is_file()).expect("workspace root");
        let Ok(text) = std::fs::read_to_string(root.join(".env")) else { return };
        let parsed = super::parse_env(&text);
        let expected = text
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#') && t.contains('=')
            })
            .count();
        assert_eq!(parsed.len(), expected);
        // A key left empty is the state setup.sh leaves `.env` in until the operator pastes one (#15);
        // a quote surviving the parse is the mistake.
        assert!(parsed.iter().filter(|(k, _)| k.contains("API_KEY")).all(|(_, v)| !v.contains('"')));
    }
}

//! Hot-swap releases: a running binary notices when `scripts/release.sh` installs a new build at
//! its path, checks that the new file is complete and runs, and exits at a safe point so its
//! supervisor starts the new build. The fleet reconnects inside the server's 120 s grace window and
//! resyncs its seats, so play continues.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Exit code that tells the supervisor "new release, restart now" (EX_TEMPFAIL).
pub const SWAP_EXIT_CODE: i32 = 75;

#[derive(Clone, Debug, PartialEq)]
struct Fingerprint {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: i64,
}

fn fingerprint(path: &Path) -> Option<Fingerprint> {
    let m = std::fs::metadata(path).ok()?;
    Some(Fingerprint { dev: m.dev(), ino: m.ino(), len: m.len(), mtime: m.mtime() })
}

pub struct ExeWatch {
    path: PathBuf,
    started: Fingerprint,
}

impl ExeWatch {
    /// Watch the file this process was started from (resolved now, before any replacement).
    pub fn current() -> Option<Self> {
        let path = std::env::current_exe().ok()?;
        Self::at(path)
    }

    pub fn at(path: PathBuf) -> Option<Self> {
        let started = fingerprint(&path)?;
        Some(Self { path, started })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A different file is installed, has been left untouched for `settle`, and answers
    /// `--version` with the expected program name within 15 s.
    pub fn replacement_ready(&self, settle: Duration, program: &str) -> bool {
        let Some(now) = fingerprint(&self.path) else { return false };
        if now == self.started {
            return false;
        }
        let modified = std::fs::metadata(&self.path).and_then(|m| m.modified()).unwrap_or(SystemTime::now());
        if SystemTime::now().duration_since(modified).unwrap_or_default() < settle {
            return false;
        }
        responds_to_version(&self.path, program)
    }
}

fn responds_to_version(path: &Path, program: &str) -> bool {
    let Ok(mut child) =
        std::process::Command::new(path).arg("--version").stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn()
    else {
        return false;
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = String::new();
                if let Some(mut s) = child.stdout.take() {
                    use std::io::Read;
                    let _ = s.read_to_string(&mut out);
                }
                return status.success() && out.starts_with(program);
            }
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// `--version` output shared by every binary: `<program> <version> <build commit>`.
pub fn version_line(program: &str) -> String {
    format!("{program} {} {}", crate::VERSION, crate::BUILD_COMMIT)
}

/// Handle `--version` before any other startup work.
pub fn handle_version_flag(program: &str) {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("{}", version_line(program));
        std::process::exit(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn version_output_identifies_the_baked_commit() {
        let line = version_line("sv10-bot");
        assert_eq!(line, format!("sv10-bot {} {}", crate::VERSION, crate::BUILD_COMMIT));
    }

    fn script(path: &Path, body: &str) {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::rename(&tmp, path).unwrap();
    }

    #[test]
    fn only_a_settled_working_replacement_triggers_a_swap() {
        let dir = std::env::temp_dir().join(format!("sv10-release-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("sv10-bot");
        script(&exe, "echo sv10-bot 10.0.0");
        let watch = ExeWatch::at(exe.clone()).unwrap();
        assert!(!watch.replacement_ready(Duration::ZERO, "sv10-bot"), "unchanged file");

        script(&exe, "exit 3");
        assert!(!watch.replacement_ready(Duration::ZERO, "sv10-bot"), "replacement that fails --version");

        script(&exe, "echo learner 10.0.0");
        assert!(!watch.replacement_ready(Duration::ZERO, "sv10-bot"), "wrong program installed");

        script(&exe, "echo sv10-bot 10.0.1");
        assert!(!watch.replacement_ready(Duration::from_secs(3600), "sv10-bot"), "not settled yet");
        assert!(watch.replacement_ready(Duration::ZERO, "sv10-bot"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

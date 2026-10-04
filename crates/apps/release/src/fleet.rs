//! Which fleet processes are alive (`running_installed_bot_is_verified` and
//! `fleet_supervisors_running` in `rollback.sh`), read from the pid files in `artifacts/`.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

fn pid_in(file: &Path, pattern: fn(&str) -> bool) -> Option<u32> {
    let text: String = std::fs::read_to_string(file).ok()?.chars().filter(|c| !c.is_whitespace()).collect();
    pattern(&text).then(|| text.parse().ok()).flatten()
}

/// `artifacts/<name>` for each fixed name, then every `worker-*<suffix>` the directory holds.
fn pid_files(root: &Path, fixed: &[&str], suffix: &str) -> Vec<PathBuf> {
    let dir = root.join("artifacts");
    let mut files: Vec<PathBuf> = fixed.iter().map(|n| dir.join(n)).collect();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let mut workers: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.len() >= "worker-".len() + suffix.len() && n.starts_with("worker-") && n.ends_with(suffix))
            })
            .collect();
        workers.sort();
        files.extend(workers);
    }
    files
}

/// Whether a running bot, head or worker process executes the installed `target/release/sv10-bot`
/// (the same file, by device and inode).
pub fn installed_bot_running(root: &Path) -> bool {
    let Ok(installed) = std::fs::metadata(root.join("target/release/sv10-bot")) else { return false };
    pid_files(root, &["bot.pid", "head.pid"], ".pid").iter().any(|file| {
        let all_digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
        let Some(pid) = pid_in(file, all_digits) else { return false };
        std::fs::metadata(format!("/proc/{pid}/exe")).is_ok_and(|m| (m.dev(), m.ino()) == (installed.dev(), installed.ino()))
    })
}

/// Whether a live, non-zombie supervisor runs from this checkout. A pid left behind may have been
/// recycled by an unrelated process or be a zombie; only a live process whose working directory is
/// the root counts, so a fleet taken down on purpose is not read as a crash loop (#726).
pub fn supervisors_running(root: &Path) -> bool {
    let Ok(root_real) = std::fs::canonicalize(root) else { return false };
    let fixed = [
        "supervisor.pid",
        "head-supervisor.pid",
        "learner-supervisor.pid",
        "analyst-supervisor.pid",
        "monitor-supervisor.pid",
        "logrotate.pid",
    ];
    pid_files(root, &fixed, "-supervisor.pid").iter().any(|file| {
        let positive = |t: &str| t.bytes().next().is_some_and(|b| (b'1'..=b'9').contains(&b)) && t.bytes().all(|b| b.is_ascii_digit());
        let Some(pid) = pid_in(file, positive) else { return false };
        !is_zombie(pid) && std::fs::canonicalize(format!("/proc/{pid}/cwd")).is_ok_and(|cwd| cwd == root_real)
    })
}

/// `Z` in the state field of `/proc/<pid>/stat`, which follows the last `)` of the command name.
fn is_zombie(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|s| s.rsplit_once(')').map(|(_, rest)| rest.trim_start().starts_with('Z')))
        .unwrap_or(false)
}

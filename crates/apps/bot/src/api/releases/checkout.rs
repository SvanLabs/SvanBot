//! What the checkout and the release artifacts say: the commit range an update would bring and how
//! it groups, the last update check, and the saved builds a rollback can restore.
//!
//! Split out of `releases.rs` (the 500-line rule: that file was at its 657-line baseline ceiling).
//! Everything here reads — git, `releases.log`, `release-snapshots/`, `update-check.json` — and
//! nothing here answers an HTTP request.

use serde_json::{Value, json};

use super::super::now_secs;

/// Group a commit subject by its leading prefix (`Deepen sv10-store: …` → Refactors), including
/// conventional-commit prefixes (`feat:`, `perf:`, `test:`, `chore:`, `refactor(x):`).
pub fn changelog_group(subject: &str) -> &'static str {
    let token: String = subject.split(|c: char| !c.is_ascii_alphanumeric()).next().unwrap_or("").to_ascii_lowercase();
    match token.as_str() {
        "deepen" | "collapse" | "split" | "unify" | "move" | "refactor" => "Refactors",
        "fix" => "Fixes",
        "add" | "feat" => "Features",
        "perf" => "Performance",
        "test" | "tests" => "Tests",
        "remove" | "chore" => "Cleanup",
        "merge" => "Merges",
        "map" => "Planning",
        "update" | "bump" | "release" => "Updates",
        "doc" | "docs" => "Docs",
        _ => "Other",
    }
}

/// Parse one `artifacts/releases.log` line (`<date> <commit> <subject>`) into (commit, subject).
pub fn parse_release_line(line: &str) -> Option<(String, String)> {
    let mut parts = line.split_whitespace();
    parts.next()?;
    let commit = parts.next()?.to_string();
    let subject: String = parts.collect::<Vec<_>>().join(" ");
    if commit.is_empty() || subject.is_empty() { None } else { Some((commit, subject)) }
}

/// Installed commit and when, from the newest `releases.log` entry.
pub(super) fn installed(artifacts: &std::path::Path) -> Option<(String, String, String)> {
    let text = std::fs::read_to_string(artifacts.join("releases.log")).ok()?;
    let line = text.lines().rev().find(|l| !l.trim().is_empty())?;
    let (commit, subject) = parse_release_line(line)?;
    let at = line.split_whitespace().next().unwrap_or("").to_string();
    Some((commit, at, subject))
}

/// Run git in the repo root; `None` when git fails or is missing.
pub(super) fn git(root: &std::path::Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").arg("-C").arg(root).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

/// (short hash, subject) of local `main` HEAD.
pub(super) fn head_commit(root: &std::path::Path) -> Option<(String, String)> {
    let line = git(root, &["log", "-1", "--format=%h %s"])?;
    let line = line.trim_end();
    let mut parts = line.splitn(2, ' ');
    Some((parts.next()?.to_string(), parts.next().unwrap_or("").to_string()))
}

/// (full hash, subject) of every commit in `base..target`, newest first.
pub(super) fn commit_range_to(root: &std::path::Path, base: &str, target: &str) -> Vec<(String, String)> {
    let range = format!("{base}..{target}");
    let Some(out) = git(root, &["log", "--format=%H %s", &range]) else { return Vec::new() };
    out.lines()
        .filter_map(|l| {
            let mut parts = l.splitn(2, ' ');
            Some((parts.next()?.to_string(), parts.next().unwrap_or("").to_string()))
        })
        .collect()
}

/// The branch and remote updates come from (`SVANBOT_UPDATE_BRANCH`, `SVANBOT_UPDATE_REMOTE`).
pub(super) fn update_source() -> (String, String) {
    let var = |k: &str, d: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| d.to_string());
    (var("SVANBOT_UPDATE_REMOTE", "origin"), var("SVANBOT_UPDATE_BRANCH", "main"))
}

/// The last `scripts/update.sh --check`: what the update branch holds that this checkout lacks.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct UpdateCheck {
    /// `remote/branch` checked.
    pub source: String,
    /// Short commit at the tip of the update branch, when the fetch worked.
    pub commit: Option<String>,
    /// Commits on the update branch not yet in this checkout.
    pub behind: u64,
    /// When the check ran (seconds since the epoch).
    pub checked_at: f64,
    /// Why the check failed (network, credentials), if it did.
    pub error: Option<String>,
}

/// Parse `update.sh --check` output: `<behind> <commit>`.
pub fn parse_check(stdout: &str) -> Option<(u64, String)> {
    let mut parts = stdout.split_whitespace();
    let behind = parts.next()?.parse().ok()?;
    let commit = parts.next()?.to_string();
    Some((behind, commit))
}

pub(super) fn check_path(artifacts: &std::path::Path) -> std::path::PathBuf {
    artifacts.join("update-check.json")
}

/// The stored result of the last check, if any.
pub fn last_check(artifacts: &std::path::Path) -> Option<UpdateCheck> {
    std::fs::read_to_string(check_path(artifacts)).ok().and_then(|t| serde_json::from_str(&t).ok())
}

/// Fetch the update branch (`scripts/update.sh --check`, blocking; its fetch times out) and store
/// what it holds. Never retried in a loop: the caller runs it on a timer or on a click (LESSONS 30).
pub fn run_update_check(root: &std::path::Path, artifacts: &std::path::Path) -> UpdateCheck {
    let (remote, branch) = update_source();
    let mut check = UpdateCheck { source: format!("{remote}/{branch}"), checked_at: now_secs(), ..UpdateCheck::default() };
    let out = std::process::Command::new("bash")
        .arg("scripts/update.sh")
        .arg("--check")
        .current_dir(root)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .output();
    match out {
        Ok(o) if o.status.success() => match parse_check(&String::from_utf8_lossy(&o.stdout)) {
            Some((behind, commit)) => {
                check.behind = behind;
                check.commit = Some(commit);
            }
            None => check.error = Some("unexpected check output".into()),
        },
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            check.error =
                Some(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("fetch failed").trim().chars().take(200).collect());
        }
        Err(e) => check.error = Some(format!("could not run scripts/update.sh: {e}")),
    }
    let tmp = check_path(artifacts).with_extension("tmp");
    if let Ok(j) = serde_json::to_string(&check)
        && std::fs::write(&tmp, j).is_ok()
    {
        // The fetch and the parse are what the caller asked for and both succeeded; a rename that
        // does not land leaves the dashboard reading the previous check (or none) and the temp file
        // behind, which is reported rather than dropped (issue #326).
        if let Err(e) = std::fs::rename(&tmp, check_path(artifacts)) {
            tracing::warn!("the update check was not stored, so the panel keeps the previous result ({e})");
        }
    }
    check
}

/// Uncommitted or untracked build inputs, by the same definition `scripts/release.sh` refuses on.
pub(super) fn source_dirty(root: &std::path::Path) -> bool {
    git(
        root,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            "crates",
            "Cargo.toml",
            "Cargo.lock",
            "build.rs",
            ".cargo",
            "rust-toolchain",
            "rust-toolchain.toml",
            "web",
        ],
    )
    .is_some_and(|o| !o.trim().is_empty())
}

/// Saved release snapshots a rollback can restore (`artifacts/release-snapshots/<commit>`), newest
/// first, with the subject `releases.log` recorded for each and whether it is the installed one.
pub(super) fn snapshots(artifacts: &std::path::Path) -> Vec<Value> {
    let installed = installed(artifacts).map(|(c, _, _)| c);
    let store_format = sv10_store::packed::data_format(artifacts);
    let root = artifacts.parent().unwrap_or(artifacts);
    let log = std::fs::read_to_string(artifacts.join("releases.log")).unwrap_or_default();
    let mut out: Vec<(std::time::SystemTime, Value)> = std::fs::read_dir(artifacts.join("release-snapshots"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let commit = e.file_name().to_str()?.to_string();
            if !valid_commit(&commit) {
                return None;
            }
            let at = e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            let line = log.lines().rev().find(|l| parse_release_line(l).is_some_and(|(c, _)| c == commit));
            let subject = line.and_then(parse_release_line).map(|(_, s)| s);
            let installed_at = line.and_then(|l| l.split_whitespace().next()).map(String::from);
            // A build that cannot read the stored data is listed but refused (0229; rollback.sh checks too).
            let readable = build_data_format(root, &commit) >= store_format;
            Some((
                at,
                json!({"commit": commit, "subject": subject, "installed_at": installed_at, "current": installed.as_deref() == Some(commit.as_str()), "readable": readable}),
            ))
        })
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.0));
    out.into_iter().map(|(_, v)| v).collect()
}

/// The data format a build reads: the `DATA_FORMAT` line of its store codec, 1 before the codec
/// existed (the same rule as `scripts/rollback.sh`).
pub(super) fn build_data_format(root: &std::path::Path, commit: &str) -> u32 {
    let out =
        std::process::Command::new("git").arg("-C").arg(root).args(["show", &format!("{commit}:crates/libs/store/src/packed.rs")]).output();
    let source = out.ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    source.lines().find_map(|l| l.strip_prefix("pub const DATA_FORMAT: u32 = ")?.strip_suffix(';')?.parse().ok()).unwrap_or(1)
}

/// A short or full lowercase hex commit id (what snapshot directories are named by).
pub(super) fn valid_commit(c: &str) -> bool {
    (7..=40).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

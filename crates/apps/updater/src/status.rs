//! What is installed and what is running: the one panel that replaces `verify-install` and
//! `rollback.sh`.
//!
//! v10 verified an install by curling the bot's health endpoint, diffing `git rev-parse` and
//! tailing a release log, and it rolled back by re-pointing a `current` symlink by hand. Both were
//! asking the same question — which build is the fleet running, and is it the one we meant to
//! install? — so v11 answers it once, from the same files the updater itself acts on: the slot
//! tree, and the canary's start log.
//!
//! It reads; it changes nothing. A person who wants to change what is running runs `update` or
//! `rollback`, which take the lock and do the swap between hands.

use std::path::{Path, PathBuf};

use svanbot_runtime::health::{Health, WINDOW_MS};
use svanbot_runtime::release::version_cmp;
use svanbot_runtime::slot::Slots;

/// The build a slot points at, or `-` when the slot has never been pointed anywhere.
fn shown(slot: Option<PathBuf>) -> String {
    match slot {
        Some(dir) => super::version_of(&dir),
        None => "-".into(),
    }
}

/// Writes the panel for the install tree `tree`.
pub(crate) fn print(tree: &Path) -> Result<(), String> {
    let slots = Slots::in_dir(tree);
    println!("tree     {}", tree.display());
    println!("current  {}", shown(slots.current()));
    println!("previous {}", shown(slots.previous()));
    // The builds sitting in the tree are the ones an update could be pointed at; the slots above
    // are the two that are. Listing them is how a person sees a release that was installed but
    // never swapped into a slot.
    let installed = installed(tree);
    if !installed.is_empty() {
        println!("installed {}", installed.join(", "));
    } else {
        // The tree is empty, or holds only a staged release: either way, no window has anything
        // to point at, and the panel says so rather than leaving the line out.
        println!("installed -");
    }
    if let Ok(bot) = super::canary() {
        println!("canary   {bot}");
        match super::health_for(&bot) {
            Ok(health) => report(&health),
            Err(why) => println!("health   unknown: {why}"),
        }
    } else {
        // No canary named: still answer what is installed, but say what cannot be read.
        println!("canary   - (set SVANBOT_CANARY to read the start log)");
    }
    Ok(())
}

/// The bot's own state: whether it came up, whether it is crash-looping, and whether a swap is
/// mid-flight because a previous run stopped where it should not have.
fn report(health: &Health) {
    let now = crate::now_ms();
    match health.starts(now, WINDOW_MS) {
        Ok(starts) => match starts.last() {
            Some(start) => println!("running  {} since {}", start.version, start.at_ms),
            None => println!("running  nothing started in the last five minutes"),
        },
        Err(why) => println!("running  unknown: {why}"),
    }
    match health.in_crash_loop(now, WINDOW_MS, super::CRASH_STARTS) {
        Ok(true) => println!(
            "crash    {}+ starts in five minutes: the build is not surviving",
            super::CRASH_STARTS
        ),
        Ok(false) => {}
        Err(why) => println!("crash    unknown: {why}"),
    }
    if health.update_requested() {
        println!("pending  an update request is still standing: a swap did not finish");
    }
    if health.stopped() {
        println!("stopped  the bot says it left the table and has not been started again");
    }
}

/// The versions installed in `tree`, in version order. The same rule `update` uses to choose a
/// build: a slot and a staged release are not builds, so neither is listed, and the order is the
/// version order [`version_cmp`] gives, not the text order a directory listing happens to have.
fn installed(tree: &Path) -> Vec<String> {
    let mut versions: Vec<String> = super::installs(tree)
        .iter()
        .map(|path| super::version_of(path))
        .collect();
    versions.sort_by(|a, b| version_cmp(a, b));
    versions
}

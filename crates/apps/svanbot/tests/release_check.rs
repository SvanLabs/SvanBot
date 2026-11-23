//! `svanbot release-check`, run as the operator runs it (V11-22): it names every clause of the
//! ship bar a release fails and exits 1, and exits 2 when it cannot read what it is to check. The
//! bouts it plays again are not played here: `svanbot_runtime::ship` holds the clauses to a replay
//! it is handed, and a run that plays again bit for bit takes minutes.
// A test clone, fleet and store are files in a fresh temporary directory.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::json;
use support::scratch;
use svanbot_runtime::ledger::Played;
use svanbot_runtime::record::Hand;
use svanbot_runtime::store::{Source, Store};
use svanbot_strategy::params::Params;

/// 2026-11-23T10:00:00Z.
const TEN_O_CLOCK: i64 = 1_795_428_000_000;

/// A clone of one commit tagged `v11.0.0`, with the fleet's files and the stores beside it.
fn clone(name: &str) -> PathBuf {
    let dir = scratch(&format!("release-check-{name}"));
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args(["-c", "user.name=test", "-c", "user.email=test@localhost"])
            .args(args)
            .current_dir(&dir)
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_WORK_TREE")
            .output()
            .unwrap()
            .status;
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "--quiet"]);
    git(&["commit", "--quiet", "--allow-empty", "--message", "the release"]);
    git(&["tag", "v11.0.0"]);
    dir
}

/// `svanbot release-check <args>` in `dir`, with its fleet and its stores there too: the exit
/// code, and everything it printed.
fn check(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> (Option<i32>, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .arg("release-check")
        .args(args)
        .current_dir(dir)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("SVANBOT_FLEET", dir.join("fleet"))
        .env("SVANBOT_DATA", dir.join("data"))
        .envs(env.iter().copied())
        .output()
        .unwrap();
    let printed = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    (output.status.code(), printed)
}

fn store(dir: &Path, bot: &str) -> Store {
    std::fs::create_dir_all(dir.join("data")).unwrap();
    Store::open(&dir.join("data").join(format!("{bot}.db")), bot).unwrap()
}

fn played(id: &str) -> Played {
    Played {
        hand_id: id.into(),
        table_id: "t-1".into(),
        digest: Params::DEFAULT.digest(),
        net: 20,
        luck: 0.0,
        big_blind: 20,
        orbit: None,
    }
}

fn exported(id: &str, started_at: &str) -> Hand {
    let item = json!({"hand_id": id, "profit": 40, "big_blind": 20, "started_at": started_at});
    Hand {
        hand_id: id.into(),
        table_id: "t-9".into(),
        frames: vec![item.to_string()],
        traced: Vec::new(),
    }
}

#[test]
fn without_a_tag_it_says_how_it_is_run() {
    let dir = clone("usage");
    let (code, printed) = check(&dir, &[], &[]);
    assert_eq!(code, Some(2), "{printed}");
    assert!(
        printed.contains("usage: svanbot release-check <tag> [fleet]"),
        "{printed}"
    );
    let (code, _) = check(&dir, &["v11.0.0", "canary"], &[]);
    assert_eq!(code, Some(2));
}

#[test]
fn a_tag_git_does_not_know_is_refused_before_anything_is_read() {
    let dir = clone("unknown");
    let (code, printed) = check(&dir, &["v11.0.9"], &[]);
    assert_eq!(code, Some(2), "{printed}");
    assert!(
        printed.contains("git has no commit named v11.0.9"),
        "{printed}"
    );
}

#[test]
fn with_no_arena_run_from_the_tags_commit_both_leagues_are_refused() {
    let dir = clone("no-run");
    let (code, printed) = check(&dir, &["v11.0.0"], &[]);
    assert_eq!(code, Some(1), "{printed}");
    assert!(printed.contains("refused: no heads-up run from "), "{printed}");
    assert!(printed.contains("refused: no 6-max run from "), "{printed}");
    assert!(!printed.contains("clears the ship bar"), "{printed}");
}

#[test]
fn the_canary_stage_needs_a_release_a_canary_and_the_canary_on_that_release() {
    let dir = clone("canary-taken");
    let (_, printed) = check(&dir, &["HEAD", "fleet"], &[]);
    assert!(printed.contains("refused: HEAD names no release"), "{printed}");
    let (_, printed) = check(&dir, &["v11.0.0", "fleet"], &[]);
    assert!(printed.contains("needs SVANBOT_CANARY"), "{printed}");
    // A store that never ran the release: the window has not opened.
    store(&dir, "SurSvan").append_played(&played("h-1"), TEN_O_CLOCK).unwrap();
    let (code, printed) = check(&dir, &["v11.0.0", "fleet"], &[("SVANBOT_CANARY", "SurSvan")]);
    assert_eq!(code, Some(1), "{printed}");
    assert!(
        printed.contains("refused: SurSvan has not taken 11.0.0"),
        "{printed}"
    );
}

#[test]
fn the_canary_stage_reads_the_window_from_the_stores_and_refuses_it_short_of_its_hands() {
    let dir = clone("canary-window");
    let canary = store(&dir, "SurSvan");
    canary.append_played(&played("h-0"), TEN_O_CLOCK - 1).unwrap();
    canary.mark_release("11.0.0", TEN_O_CLOCK).unwrap();
    canary.append_played(&played("h-1"), TEN_O_CLOCK + 1_000).unwrap();
    canary.append_played(&played("h-2"), TEN_O_CLOCK + 3_600_000).unwrap();
    let mate = store(&dir, "Svanar");
    for (id, started_at) in [
        ("x-1", "2026-11-23T10:30:00Z"),
        // Before the canary took the release, and after its latest hand.
        ("x-2", "2026-11-23T09:59:59Z"),
        ("x-3", "2026-11-23T11:00:01Z"),
    ] {
        mate.append_hand(&exported(id, started_at), Source::History, 0)
            .unwrap();
    }
    let (code, printed) = check(&dir, &["v11.0.0", "fleet"], &[("SVANBOT_CANARY", "SurSvan")]);
    assert_eq!(code, Some(1), "{printed}");
    assert!(
        printed.contains("Svanar: 1 export hands in the window, 0 unread"),
        "{printed}"
    );
    assert!(
        printed.contains("refused: the canary is not yet measured: 2 of its 20000 hands"),
        "{printed}"
    );
}

//! `svanbot play pin`: the continuation-pin experiment's rows. Its design names the shared
//! archetypes and the reference policy, not the stablemate duels; a stablemate row costs a full
//! policy-vs-policy ring for every bout and does not belong to this experiment (V11-90).
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::Path;
use std::process::{Command, Output};

use svanbot_runtime::fleet::Fleet;

fn pin(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .env_clear()
        .env("SVANBOT_FLEET", dir.join("fleet"))
        .arg("play")
        .arg("pin")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn a_pin_run_plays_the_shared_rows_and_no_stablemate() {
    let dir = std::env::temp_dir().join(format!("svanbot-pin-rows-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("fleet")).unwrap();
    let out = pin(&dir, &["fixed", "1", "1", "1"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let runs = Fleet::in_dir(dir.join("fleet"))
        .policy_arena_runs()
        .unwrap();
    assert_eq!(runs.len(), 1);
    let names: Vec<_> = runs[0]
        .rows
        .iter()
        .map(|row| row.opponent.as_str())
        .collect();
    assert!(
        names.iter().all(|name| !name.starts_with("stablemate")),
        "{names:?}"
    );
    for expected in [
        "pricer",
        "v10.0.2",
        "calling station",
        "nit",
        "maniac",
        "switching",
        "learning",
        "sizing",
        "bluffer",
        "mixing",
    ] {
        assert!(
            names.contains(&expected),
            "{expected} missing from {names:?}"
        );
    }
}

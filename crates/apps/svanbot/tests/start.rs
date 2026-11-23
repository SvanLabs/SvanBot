//! The binary refuses to start without what fair play and the venue need.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]

use std::path::PathBuf;
use std::process::Command;

/// A health directory outside the repo, so a bot that starts without a store — which it does on
/// purpose, so the updater sees it come up — records that start somewhere other than the clone.
fn health_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-start-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(env: &[(&str, &str)]) -> (Option<i32>, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .env_clear()
        .env("SVANBOT_HEALTH", health_dir())
        .envs(env.iter().copied())
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn without_a_key_it_does_not_start() {
    let (code, stderr) = run(&[
        ("SVANBOT_NAME", "SurSvan"),
        ("SVANBOT_ROSTER", "SurSvan,Svanar"),
    ]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("SVANBOT_API_KEY"), "{stderr}");
}

#[test]
fn without_the_roster_it_does_not_start() {
    let (code, stderr) = run(&[
        ("SVANBOT_NAME", "SurSvan"),
        ("SVANBOT_API_KEY", "k"),
        ("SVANBOT_ROSTER", " , "),
    ]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("SVANBOT_ROSTER"), "{stderr}");
}

#[test]
fn without_a_store_it_does_not_start_because_an_incident_must_be_kept() {
    let (code, stderr) = run(&[
        ("SVANBOT_NAME", "SurSvan"),
        ("SVANBOT_API_KEY", "k"),
        ("SVANBOT_ROSTER", "SurSvan,Svanar"),
        (
            "SVANBOT_STORE",
            "/proc/svanbot-has-no-store-here/SurSvan.db",
        ),
        // Never reached: the store is opened before the venue is.
        ("SVANBOT_URL", "ws://127.0.0.1:9/ws"),
    ]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("store"), "{stderr}");
}

#[test]
fn the_download_needs_the_bot_s_own_key() {
    let output = Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .arg("download")
        .env_clear()
        .env("SVANBOT_HEALTH", health_dir())
        .env("SVANBOT_NAME", "SurSvan")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("SVANBOT_API_KEY"));
}

#[test]
fn a_proposal_of_a_number_the_policy_does_not_have_is_refused_and_writes_nothing() {
    let fleet = health_dir().join("fleet-refused");
    let output = Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .args(["propose", "bluff_more=1"])
        .env_clear()
        .env("SVANBOT_HEALTH", health_dir())
        .env("SVANBOT_FLEET", &fleet)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("exploit"), "{stderr}");
    assert!(!fleet.join("proposal.json").exists());
}

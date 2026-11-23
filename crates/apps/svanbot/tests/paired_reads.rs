//! The paired reads command measures each shared deal once across the fixed archetypes.
#![allow(clippy::disallowed_types)]

use std::process::Command;

#[test]
fn the_fixed_archetypes_share_the_same_deals_in_the_paired_verdict() {
    let health = std::env::temp_dir().join(format!("svanbot-paired-reads-{}", std::process::id()));
    let output = Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .args(["paired", "reads", "2", "1", "0"])
        .env_clear()
        .env("SVANBOT_HEALTH", health)
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.lines().any(|line| {
            line.starts_with("reads on against off, heads up:") && line.contains("over 2 deals")
        }),
        "{stdout}"
    );
}

#[test]
fn six_max_counts_each_rotated_deal_once_and_empty_reads_match() {
    let output = Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .args(["paired", "reads", "6-max", "2", "1", "0"])
        .env_clear()
        .env(
            "SVANBOT_HEALTH",
            std::env::temp_dir().join(format!("svanbot-ring-reads-{}", std::process::id())),
        )
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("reads on against off, 6-max: +0 mbb/100, 95% CI ±0, over 2 deals"),
        "{stdout}"
    );
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn passive_opponents_keep_the_population_model_in_the_aggressive_reads_trial() {
    let output = Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .args(["paired", "reads", "6-max", "8", "1", "40"])
        .env_clear()
        .env(
            "SVANBOT_HEALTH",
            std::env::temp_dir().join(format!("svanbot-aggression-{}", std::process::id())),
        )
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    for name in ["calling station", "nit"] {
        assert!(
            stdout.contains(&format!("off, {name}: +0 mbb/100, 95% CI ±0")),
            "{stdout}"
        );
    }
}

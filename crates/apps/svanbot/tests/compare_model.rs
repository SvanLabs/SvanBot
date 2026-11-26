//! The paired action-model comparison: both arms on the same deals, and the confirmation gate.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn play(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .env_clear()
        .arg("play")
        .args(args)
        .output()
        .unwrap()
}

fn fixture(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn compare_model_refuses_a_missing_candidate_or_incumbent_before_play() {
    let dir = fixture("compare-model");
    let candidate = dir.join("candidate.model");
    let incumbent = dir.join("incumbent.model");
    let run = |incumbent: &str| {
        play(&[
            "compare-model",
            candidate.to_str().unwrap(),
            incumbent,
            "1",
            "17",
            "1",
        ])
    };
    let missing_candidate = run("baseline");
    assert_eq!(missing_candidate.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing_candidate.stderr).contains("candidate model"));
    std::fs::write(
        &candidate,
        svanbot_strategy::action::ActionModel::baseline().to_text(),
    )
    .unwrap();
    let missing_incumbent = run(incumbent.to_str().unwrap());
    assert_eq!(missing_incumbent.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing_incumbent.stderr).contains("incumbent model"));
    assert!(missing_incumbent.stdout.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn compare_model_plays_each_file_in_its_own_arm() {
    let dir = fixture("compare-model-arms");
    let baseline = dir.join("baseline.model");
    let uniform = dir.join("uniform.model");
    std::fs::write(
        &baseline,
        svanbot_strategy::action::ActionModel::baseline().to_text(),
    )
    .unwrap();
    std::fs::write(
        &uniform,
        svanbot_strategy::action::ActionModel::uniform().to_text(),
    )
    .unwrap();
    let run = |candidate: &Path, incumbent: &Path| {
        let result = play(&[
            "compare-model",
            candidate.to_str().unwrap(),
            incumbent.to_str().unwrap(),
            "1",
            "23",
            "2",
        ]);
        assert_eq!(result.status.code(), Some(1));
        let stdout = String::from_utf8_lossy(&result.stdout);
        let overall = stdout
            .lines()
            .find(|line| line.trim_start().starts_with("overall"))
            .unwrap_or_else(|| panic!("no overall line in {stdout}"));
        overall
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse::<f64>()
            .unwrap()
    };
    let forward = run(&uniform, &baseline);
    let reverse = run(&baseline, &uniform);
    assert!(forward.abs() > 0.0, "both arms played alike: {forward}");
    assert!(
        (forward + reverse).abs() < 0.05,
        "swapping the files did not swap the arms: {forward} and {reverse}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn compare_model_rejects_confirmation_on_the_same_deals() {
    let result = play(&[
        "compare-model",
        "candidate",
        "baseline",
        "30",
        "17",
        "--confirm",
        "17",
    ]);
    assert_eq!(result.status.code(), Some(2));
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("fresh seed"));
}

//! `svanbot paired compare`: one build against another, row by row, over two Arena runs played
//! on the same seed (V11-39). It plays nothing itself: the runs are the record.
// The runs are files in a fresh temporary directory.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]

mod support;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use support::scratch;
use svanbot_runtime::fleet::{ArenaRow, ArenaRun, League};

fn run(commit: &str, seed: u64, v10: &[f64]) -> ArenaRun {
    ArenaRun {
        commit: commit.into(),
        champion: Some("0123456789abcdef".into()),
        seed,
        league: League::SixMax,
        iterations: Some(20),
        rows: vec![
            ArenaRow {
                opponent: "v10.0.2".into(),
                deals: 50 * v10.len(),
                mbb_per_100: 0.0,
                ci95: 0.0,
                bouts: v10.to_vec(),
            },
            ArenaRow {
                opponent: "nit".into(),
                deals: 50 * v10.len(),
                mbb_per_100: 0.0,
                ci95: 0.0,
                bouts: vec![4.0; v10.len()],
            },
        ],
    }
}

fn write(dir: &Path, name: &str, run: &ArenaRun) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, run.as_text()).unwrap();
    path
}

fn compare(base: &Path, next: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .args(["paired", "compare"])
        .arg(base)
        .arg(next)
        .env_clear()
        .output()
        .unwrap()
}

#[test]
fn each_row_is_the_second_build_less_the_first_over_the_same_bouts() {
    let dir = scratch("compare-rows");
    let base = write(&dir, "base.json", &run("aaaa", 7, &[10.0, 20.0]));
    let next = write(&dir, "next.json", &run("bbbb", 7, &[15.0, 30.0]));
    let output = compare(&base, &next);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("6-max v10.0.2: +15000 mbb/100, 95% CI ±9800, over 100 deals"),
        "{stdout}"
    );
    assert!(
        stdout.contains("6-max nit: +0 mbb/100, 95% CI ±0, over 100 deals"),
        "{stdout}"
    );
    assert_eq!(output.status.code(), Some(0), "{stdout}");
}

#[test]
fn a_row_wholly_below_zero_fails_the_comparison() {
    let dir = scratch("compare-worse");
    let base = write(&dir, "base.json", &run("aaaa", 7, &[15.0, 30.0]));
    let next = write(&dir, "next.json", &run("bbbb", 7, &[10.0, 20.0]));
    let output = compare(&base, &next);
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn runs_from_different_seeds_are_refused_rather_than_compared() {
    let dir = scratch("compare-seeds");
    let base = write(&dir, "base.json", &run("aaaa", 7, &[10.0, 20.0]));
    let next = write(&dir, "next.json", &run("bbbb", 8, &[15.0, 30.0]));
    let output = compare(&base, &next);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("seed"), "{stderr}");
    assert!(output.stdout.is_empty());
}

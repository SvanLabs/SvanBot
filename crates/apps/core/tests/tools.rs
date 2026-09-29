//! Process-level checks for the operator tools.

use std::process::Command;

#[test]
fn zero_benchmark_repeats_are_refused_without_a_panic() {
    let output = Command::new(env!("CARGO_BIN_EXE_bench")).args(["draw", "--repeat", "0"]).output().unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--repeat must be at least 1"));
}

#[test]
fn positive_benchmark_repeats_report_the_absence_of_a_one_round_interval() {
    let output = Command::new(env!("CARGO_BIN_EXE_bench")).args(["draw", "--repeat", "1"]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["rounds"], 1);
    assert!(report["k1"]["reject_again_over_reject"]["ci95"].is_null());
    assert_eq!(report["k1"]["reject_again_over_reject"]["verdict"], "one round: no interval");
}

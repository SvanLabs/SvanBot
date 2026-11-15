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

fn empty_models_file(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("sv10-tools-{}-{name}.json", std::process::id()));
    std::fs::write(&path, serde_json::to_string(&sv10_core::model::ModelStore::default()).unwrap()).unwrap();
    path
}

#[test]
fn an_empty_live_pool_stops_sim_with_a_message_instead_of_a_panic() {
    let models = empty_models_file("pool");
    for sub in ["paired", "variance"] {
        let output = Command::new(env!("CARGO_BIN_EXE_sim")).args([sub, "1", "1"]).env("SIM_MODELS", &models).output().unwrap();
        let err = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(2), "{sub}: {err}");
        assert!(err.contains("no live opponents"), "{sub}: {err}");
        assert!(!err.contains("panicked"), "{sub}: {err}");
    }
    std::fs::remove_file(models).ok();
}

fn sim_refuses(args: &[&str], env: &[(&str, &str)], needle: &str) {
    let output = Command::new(env!("CARGO_BIN_EXE_sim")).args(args).envs(env.iter().copied()).output().unwrap();
    let err = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{args:?} {env:?}: {err}");
    assert!(err.contains(needle), "{args:?} {env:?}: {err}");
    assert!(!err.contains("panicked"), "{args:?} {env:?}: {err}");
}

#[test]
fn a_zero_sized_run_is_refused_not_reported_as_a_precise_zero() {
    sim_refuses(&["paired", "0", "1"], &[], "tables must be at least 1");
    sim_refuses(&["paired", "1", "0"], &[], "hands must be at least 1");
    sim_refuses(&["variance", "0", "1"], &[], "tables must be at least 1");
    sim_refuses(&["variance", "1", "0"], &[], "hands must be at least 1");
    sim_refuses(&["variance", "1", "1", "0"], &[], "salts must be at least 1");
    sim_refuses(&["0", "1"], &[], "hands must be at least 1");
    sim_refuses(&["1", "0"], &[], "tables must be at least 1");
}

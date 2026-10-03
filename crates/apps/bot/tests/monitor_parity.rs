//! Parity tests for the monitor port: the frozen fixture and the lines the Python monitor it
//! replaced printed for it. The commands and their output are in the pull request for #717.

use serde_json::Value;
use sv10_bot::monitor::{Monitor, Options, unix_now};
use sv10_store::store::{HandRow, Store};

/// Frozen input for both runs: `crates/apps/bot/tests/fixtures/monitor-parity.json` describes a
/// seed store and the hands that arrive after the monitor opened. Every expected line below was
/// printed by `scripts/monitor.py` (the copy the port replaced) against a store built from this
/// same file, with the same flags — the two commands and their output are in the pull request.
const FIXTURE: &str = include_str!("fixtures/monitor-parity.json");

fn hand(v: &Value) -> HandRow {
    HandRow {
        bot: v["bot"].as_str().unwrap().to_string(),
        hand_id: v["hand_id"].as_str().unwrap().to_string(),
        table_id: "t".into(),
        ended_at: v["ended_at"].as_str().unwrap().to_string(),
        hole: v["hole"].as_str().unwrap().to_string(),
        board: v["board"].as_str().unwrap().to_string(),
        pot: v["pot"].as_i64().unwrap(),
        net: v["net"].as_i64(),
        winners: v["winners"].as_str().unwrap().to_string(),
        summary: serde_json::to_string(&v["summary"]).unwrap(),
        ..Default::default()
    }
}

/// A store under a fresh temp dir, seeded from the fixture's `seed` part (hands + models.v1).
fn fixture_store(tag: &str) -> (std::path::PathBuf, Store, Value) {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let root = std::env::temp_dir().join(format!("sv10-monitor-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("artifacts")).unwrap();
    let store = Store::open(&root.join("artifacts").join("svanbot10.db")).unwrap();
    for h in fixture["seed"]["hands"].as_array().unwrap() {
        store.insert_hand(&hand(h)).unwrap();
    }
    store.put_kv(sv10_bot::MODELS_KEY, &serde_json::to_string(&fixture["models"]).unwrap()).unwrap();
    for e in fixture["seed"]["events"].as_array().unwrap() {
        store.log_event(e["bot"].as_str().unwrap(), "error", e["message"].as_str().unwrap());
    }
    (root, store, fixture)
}

fn options(once: bool) -> Options {
    Options { min_hands: 2.0, big_loss_bb: 100.0, big_win_bb: 100.0, active_hours: 100_000.0, once, ..Options::default() }
}

/// The monitor's `HH:MM line` without the clock and in a stable order: a pass prints its lines
/// in a fixed order except where two bots stall or two opponents tie, which the dashboard reads
/// by kind.
fn normalize(lines: &[String]) -> Vec<String> {
    let mut out: Vec<String> = lines
        .iter()
        .map(|l| {
            let l = l.split_once(' ').expect("every line starts with HH:MM").1;
            assert!(l.chars().next().is_some_and(|c| c.is_ascii_uppercase()), "{l}");
            l.to_string()
        })
        .collect();
    out.sort();
    out
}

/// The frozen lines, sorted the same way as [`normalize`]'s output.
fn frozen(lines: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    out.sort();
    out
}

/// `--once` on the seed store: the starting ledger and one empty summary window.
#[test]
fn one_shot_pass_matches_the_python_monitors_output() {
    let (root, store, _) = fixture_store("once");
    let opts = options(true);
    let mut monitor = Monitor::open(&root, opts).unwrap();
    let mut lines = vec![monitor.start_line()];
    lines.extend(monitor.pass(unix_now() + 1.0, true));
    let expected = [
        "START monitor: 4 hands, 3 opponents, 0 beat us at 95%: none",
        "SUMMARY 30m: 0 hands +0 chips |  | took the most from us: none | gave us the most: none | toughest all-time (bb/100 moved between us and them in champion hands, ranked without each one's biggest pot): Fish +10000bb/100 (2)",
        "OPPONENTS 0 faced, 0 new | most played: none",
    ];
    assert_eq!(normalize(&lines), frozen(&expected), "store had {} hands", store.max_hand_rowid().unwrap());
}

/// Two passes over hands that arrived after the monitor opened — the second, with no new rows,
/// is where the bots that just played go quiet: big results, a new nemesis, stalls, a fleet
/// error and both summary windows, all as the Python monitor printed them over its two loop
/// iterations.
#[test]
fn a_pass_over_new_hands_matches_the_python_monitors_output() {
    let (root, store, fixture) = fixture_store("live");
    let opts = Options { summary_min: 0, stall_min: 0, ..options(false) };
    let mut monitor = Monitor::open(&root, opts).unwrap();
    for h in fixture["new"]["hands"].as_array().unwrap() {
        store.insert_hand(&hand(h)).unwrap();
    }
    for e in fixture["new"]["events"].as_array().unwrap() {
        store.log_event(e["bot"].as_str().unwrap(), "error", e["message"].as_str().unwrap());
    }
    let start = unix_now();
    let mut lines = monitor.pass(start + 1.0, false);
    lines.extend(monitor.pass(start + 2.0, false));
    let expected = [
        "BIGLOSS Alpha -2500 chips (-125 bb) hand h-new-loss; won by Villain",
        "BIGWIN Bravo +2000 chips (+100 bb) hand h-new-win; against Fish",
        "NEMESIS Villain beats us: -12500.0 bb/100 moved to them over 2 champion hands (experiment-arm hands excluded, 0361; family-wise 95% upper -12500.0, z 2.24)",
        "STALL Alpha: no completed hand for 0+ minutes",
        "STALL Bravo: no completed hand for 0+ minutes",
        "STALL Solo: no completed hand for 0+ minutes",
        "ERROR Alpha: hand upload failed: connection reset (table 42, attempt 3)",
        "SUMMARY 0m: 3 hands -490 chips | Alpha -2490/2 Bravo +2000/1 | took the most from us: Villain -125bb | gave us the most: Fish +100bb, Newbie +0bb | toughest all-time (bb/100 moved between us and them in champion hands, ranked without each one's biggest pot): Villain -12500bb/100 (2), Fish +10000bb/100 (3)",
        "OPPONENTS 3 faced, 1 new: Newbie | most played: Villain (Loose-passive VPIP 30/PFR 10, 200h; we -125 bb against them over 1 hands), Fish (no model; we +100 bb against them over 1 hands), Newbie (no model; we +0 bb against them over 1 hands)",
        "SUMMARY 0m: 0 hands +0 chips |  | took the most from us: none | gave us the most: none | toughest all-time (bb/100 moved between us and them in champion hands, ranked without each one's biggest pot): Villain -12500bb/100 (2), Fish +10000bb/100 (3)",
        "OPPONENTS 0 faced, 0 new | most played: none",
    ];
    assert_eq!(normalize(&lines), frozen(&expected));
}

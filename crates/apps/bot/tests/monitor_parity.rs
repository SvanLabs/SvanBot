//! Parity tests for the monitor port: the frozen fixture and the lines the Python monitor it
//! replaced printed for it. The commands and their output are in the pull request for #717.

use chrono::{DateTime, Utc};
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

/// A store with the schema and nothing in it, for cases whose dates are relative to now and so
/// cannot be frozen in the fixture.
fn plain_store(tag: &str) -> (std::path::PathBuf, Store) {
    let root = std::env::temp_dir().join(format!("sv10-monitor-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("artifacts")).unwrap();
    let store = Store::open(&root.join("artifacts").join("svanbot10.db")).unwrap();
    (root, store)
}

fn play_hand(store: &Store, bot: &str, hand_id: &str, ended_at: DateTime<Utc>) {
    store
        .insert_hand(&HandRow { bot: bot.into(), hand_id: hand_id.into(), ended_at: ended_at.to_rfc3339(), ..Default::default() })
        .unwrap();
}

fn stalls(lines: &[String]) -> Vec<String> {
    normalize(lines).into_iter().filter(|l| l.starts_with("STALL ")).collect()
}

/// The supervisor runs only `--big-loss-bb`, so `--big-win-bb` has to inherit it, as the Python
/// monitor's argparse default did; an explicit flag still wins.
#[test]
fn big_win_bb_defaults_to_big_loss_bb() {
    let inherited = Options::parse(&["--big-loss-bb".to_string(), "250".to_string()]).unwrap();
    assert_eq!(inherited.big_win_bb, 250.0);
    let explicit = Options::parse(&["--big-loss-bb".to_string(), "250".to_string(), "--big-win-bb".to_string(), "10".to_string()]).unwrap();
    assert_eq!(explicit.big_win_bb, 10.0);
}

/// `--active-hours` selects the stall-watch population: a bot whose last hand is older than the
/// window is not watched, so going quiet is not a stall — while a bot that played inside it is.
#[test]
fn only_bots_that_played_inside_the_active_window_are_stall_watched() {
    let (root, store) = plain_store("active-hours");
    let now = unix_now();
    let at = |secs_ago: i64| DateTime::<Utc>::from_timestamp(now as i64 - secs_ago, 0).unwrap();
    play_hand(&store, "Recent", "h-recent", at(60));
    play_hand(&store, "Dormant", "h-dormant", at(30 * 24 * 3600));

    let opts = Options { active_hours: 24.0, stall_min: 0, ..options(true) };
    let mut monitor = Monitor::open(&root, opts).unwrap();
    assert_eq!(stalls(&monitor.pass(now + 1.0, true)), frozen(&["STALL Recent: no completed hand for 0+ minutes"]));

    let opts = Options { active_hours: 24.0 * 31.0, stall_min: 0, ..options(true) };
    let mut monitor = Monitor::open(&root, opts).unwrap();
    assert_eq!(
        stalls(&monitor.pass(now + 1.0, true)),
        frozen(&["STALL Recent: no completed hand for 0+ minutes", "STALL Dormant: no completed hand for 0+ minutes"])
    );
}

/// A store that loses the `events` table mid-run: the pass reports it as a MONITOR line and the
/// loop keeps running, as the Python monitor did on a `sqlite3.Error`.
#[test]
fn a_failed_pass_is_reported_as_a_monitor_line() {
    let (root, store, _) = fixture_store("error");
    drop(store);
    let mut monitor = Monitor::open(&root, options(true)).unwrap();
    let conn = rusqlite::Connection::open(root.join("artifacts").join("svanbot10.db")).unwrap();
    conn.execute("DROP TABLE events", []).unwrap();
    drop(conn);
    let lines = monitor.pass(unix_now() + 1.0, true);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(normalize(&lines)[0].starts_with("MONITOR db error:"), "{lines:?}");
}

/// A burst of more than twenty error events is reported in full over the next passes (#874). The
/// mark used to jump to the newest event in the store after printing the first twenty, so the rest
/// of the burst was never printed.
#[test]
fn every_error_of_a_burst_is_reported_across_passes() {
    let (root, store, _) = fixture_store("error-burst");
    let mut monitor = Monitor::open(&root, options(false)).unwrap();
    for i in 0..25 {
        store.log_event("A", "error", &format!("burst {i}"));
    }
    let now = unix_now();
    let errors = |lines: Vec<String>| lines.into_iter().filter(|l| l.contains(" ERROR A: burst ")).count();
    assert_eq!(errors(monitor.pass(now + 1.0, false)), 20);
    assert_eq!(errors(monitor.pass(now + 2.0, false)), 5, "the rest of the burst");
    assert_eq!(errors(monitor.pass(now + 3.0, false)), 0, "and nothing twice");
    let _ = std::fs::remove_dir_all(&root);
}

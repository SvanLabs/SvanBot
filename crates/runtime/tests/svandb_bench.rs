//! SvanDB's gate measurements (V11-128): per-hand append, reads/fits rebuild, dashboard reads,
//! footprint and recovery, beside SQLite on the same workload. Ignored by default; the numbers
//! on the ticket come from `cargo test -p svanbot-runtime --test svandb_bench -- --ignored
//! --nocapture`.
// A benchmark store is a file in a fresh temporary directory, and its numbers go to the terminal.
#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    clippy::disallowed_types
)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::json;
use svanbot_learn::probe::Arm;
use svanbot_runtime::ledger::Played;
use svanbot_runtime::record::Hand;
use svanbot_runtime::store::{Source, Store, Svandb};
use svanbot_strategy::params::Params;

const HANDS: usize = 2_000;

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A hand of the size the venue sends: start, a couple of actions, a result.
fn hand(at: usize) -> Hand {
    let id = format!("h-{at:05}");
    let mut frames = vec![
        json!({
            "type": "hand_start",
            "hand_id": id,
            "table_id": "t-1",
            "stack": 2_000,
            "small_blind": 10,
            "big_blind": 20,
            "hole_cards": ["Ah", "Kd"],
            "started_at": "2026-11-23T10:00:00Z",
        })
        .to_string(),
    ];
    for action in ["call", "check", "bet", "call", "check", "check"] {
        frames.push(
            json!({"type": "player_action", "hand_id": id, "seat": 2, "action": action, "amount": 40})
                .to_string(),
        );
    }
    frames.push(
        json!({"type": "hand_result", "hand_id": id, "winners": [2], "profit": 80}).to_string(),
    );
    Hand {
        hand_id: id,
        table_id: "t-1".into(),
        frames,
        traced: Vec::new(),
    }
}

fn played(at: usize) -> Played {
    Played {
        hand_id: format!("h-{at:05}"),
        table_id: "t-1".into(),
        digest: Params::DEFAULT.digest(),
        net: i64::try_from(at % 200).unwrap() - 100,
        luck: 4.0,
        big_blind: 20,
        orbit: Some((
            7,
            u64::try_from(at / 70).unwrap(),
            if at.is_multiple_of(2) {
                Arm::Champion
            } else {
                Arm::Challenger
            },
        )),
        reference: None,
    }
}

fn footprint(dir: &Path, names: &[&str]) -> u64 {
    names
        .iter()
        .filter_map(|name| std::fs::metadata(dir.join(name)).ok())
        .map(|meta| meta.len())
        .sum()
}

fn files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

fn report(metric: &str, sqlite: Duration, svandb: Duration) {
    println!(
        "{metric:28} sqlite {:>10.1} us   svandb {:>10.1} us",
        sqlite.as_secs_f64() * 1e6,
        svandb.as_secs_f64() * 1e6
    );
}

#[test]
#[ignore = "gate measurement; the numbers go on V11-128"]
fn the_gate_measurements() {
    let sql_dir = scratch("bench-sql");
    let svan_dir = scratch("bench-svan");
    let sql_path = sql_dir.join("A.db");
    let svan_path = svan_dir.join("A.svandb");

    // Per-hand append: one hand and its played row, as the recorder makes them.
    let sql = Store::open(&sql_path, "A").unwrap();
    let sql_append = Instant::now();
    for at in 0..HANDS {
        sql.append_hand(&hand(at), Source::Live, i64::try_from(at).unwrap() * 1_000)
            .unwrap();
        sql.append_played(&played(at), i64::try_from(at).unwrap() * 1_000)
            .unwrap();
    }
    let sql_append = sql_append.elapsed();
    drop(sql);

    let svan = Svandb::open(&svan_path, "A").unwrap();
    let svan_append = Instant::now();
    for at in 0..HANDS {
        svan.append_hand(&hand(at), Source::Live, i64::try_from(at).unwrap() * 1_000)
            .unwrap();
        svan.append_played(&played(at), i64::try_from(at).unwrap() * 1_000)
            .unwrap();
    }
    let svan_append = svan_append.elapsed();
    drop(svan);
    report("append (per hand + played)", sql_append, svan_append);

    // Recovery: open the store and rebuild the read side (the fits' input).
    let sql_open = Instant::now();
    let sql = Store::open(&sql_path, "A").unwrap();
    let sql_fit = sql.hands_for_fit().unwrap();
    let sql_open = sql_open.elapsed();
    let svan_open = Instant::now();
    let svan = Svandb::open(&svan_path, "A").unwrap();
    let svan_fit = svan.hands_for_fit().unwrap();
    let svan_open = svan_open.elapsed();
    assert_eq!(sql_fit.len(), HANDS);
    assert_eq!(svan_fit.len(), HANDS);
    report("recovery + fit rebuild", sql_open, svan_open);

    // Dashboard reads: totals, the replayer's list, and a hand's decisions.
    let sql_dash = Instant::now();
    for _ in 0..100 {
        sql.totals().unwrap();
        sql.traced_hands().unwrap();
        sql.traces("h-00000").unwrap();
        sql.incidents().unwrap();
    }
    let sql_dash = sql_dash.elapsed();
    let svan_dash = Instant::now();
    for _ in 0..100 {
        svan.totals().unwrap();
        svan.traced_hands().unwrap();
        svan.traces("h-00000").unwrap();
        svan.incidents().unwrap();
    }
    let svan_dash = svan_dash.elapsed();
    report("dashboard reads (100x)", sql_dash, svan_dash);

    // Footprint: every file the store keeps while open.
    let sql_files = files(&sql_dir);
    let sql_footprint: u64 = sql_files
        .iter()
        .map(|name| {
            std::fs::metadata(sql_dir.join(name))
                .map(|meta| meta.len())
                .unwrap_or(0)
        })
        .sum();
    let svan_footprint = footprint(&svan_dir, &["A.svandb", "A.svandb.manifest"]);
    println!(
        "{:28} sqlite {:>10} bytes   svandb {:>10} bytes",
        "footprint", sql_footprint, svan_footprint
    );

    // Migration: copy the SQLite store into a fresh log, and time the copy.
    let import_path = scratch("bench-import").join("A.svandb");
    let imported = Instant::now();
    let copy = Svandb::import(&sql, &import_path).unwrap();
    let imported = imported.elapsed();
    println!(
        "{:28} svandb {:>10.1} us",
        "migration (2000 hands)",
        imported.as_secs_f64() * 1e6
    );
    assert_eq!(copy.totals().unwrap().hands, sql.totals().unwrap().hands);
    // Every read surface the two share is the same after the copy.
    assert_eq!(copy.hands().unwrap(), sql.hands().unwrap());
    assert_eq!(copy.played().unwrap(), sql.played().unwrap());
}

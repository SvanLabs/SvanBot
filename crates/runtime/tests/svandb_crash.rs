//! SvanDB's crash gate (V11-128): kill -9 an appending process at arbitrary points, then check
//! that every hand whose append returned is still there and no torn record was read.
// A test fleet is a file in a fresh temporary directory, and the child's acknowledged appends go
// to its pipe.
#![allow(
    clippy::disallowed_macros,
    clippy::disallowed_methods,
    clippy::disallowed_types
)]
#![cfg(unix)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::json;
use svanbot_runtime::record::Hand;
use svanbot_runtime::store::{Source, Svandb};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hand(id: &str) -> Hand {
    Hand {
        hand_id: id.into(),
        table_id: "t-1".into(),
        frames: vec![
            json!({"type": "hand_start", "hand_id": id}).to_string(),
            json!({"type": "hand_result", "hand_id": id}).to_string(),
        ],
        traced: Vec::new(),
    }
}

/// The child half: appends hands for ever, printing each index only after its append returned.
/// The parent kills this process with SIGKILL; the printed indices are the acknowledged writes.
#[test]
fn child_appends_until_killed() {
    let Ok(path) = std::env::var("SVANDB_CRASH_PATH") else {
        return;
    };
    let db = Svandb::open(PathBuf::from(path).as_path(), "A").unwrap();
    let mut at = 0u64;
    loop {
        let id = format!("h-{at}");
        db.append_hand(&hand(&id), Source::Live, i64::try_from(at).unwrap_or(0))
            .unwrap();
        println!("{at}");
        std::io::stdout().flush().unwrap();
        at += 1;
    }
}

#[test]
fn a_kill_9_leaves_every_acknowledged_hand_and_reads_back_cleanly() {
    let mut acknowledged_total = 0u64;
    for round in 0..12u64 {
        let dir = scratch(&format!("kill-round-{round}"));
        let path = dir.join("A.svandb");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "child_appends_until_killed", "--nocapture"])
            .env("SVANDB_CRASH_PATH", &path)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        // Kill at a different point each round: inside a log write, a manifest rename, or
        // between two appends. SIGKILL cannot be caught, so no shutdown code runs.
        std::thread::sleep(Duration::from_millis(15 + round * 10));
        child.kill().unwrap();
        let output = child.wait_with_output().unwrap();
        let acknowledged = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.trim().parse::<u64>().ok())
            .max()
            .map_or(0, |last| last + 1);
        acknowledged_total += acknowledged;

        let db = Svandb::open(&path, "A").unwrap();
        assert!(
            db.totals().unwrap().hands >= acknowledged,
            "round {round}: {acknowledged} hands were acknowledged, the log has {}",
            db.totals().unwrap().hands
        );
        for at in 0..acknowledged {
            assert!(
                db.holds(&format!("h-{at}")).unwrap(),
                "round {round}: acknowledged hand h-{at} was lost"
            );
        }
        // The log is still writable, and the next clean append survives a reopen.
        db.append_hand(&hand("after"), Source::Live, 99).unwrap();
        drop(db);
        let reopened = Svandb::open(&path, "A").unwrap();
        assert!(
            reopened.holds("after").unwrap(),
            "round {round}: the append after recovery was lost"
        );
    }
    assert!(
        acknowledged_total > 0,
        "no round got far enough to acknowledge a hand; the test proved nothing"
    );
}

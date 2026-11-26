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

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::json;
use svanbot_runtime::record::Hand;
use svanbot_runtime::store::{Source, Svandb};

/// The crash child, by the compile-time path cargo gives this package's tests.
const CHILD: &str = env!("CARGO_BIN_EXE_svandb-crash-child");

/// The round's directory, under a compile-time root. The child runs with it as its working
/// directory and opens its constant `A.svandb` there.
fn scratch(round: u64) -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/dev/svandb-crash")
        .join(format!("round-{round}"));
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

#[test]
fn a_kill_9_leaves_every_acknowledged_hand_and_reads_back_cleanly() {
    let mut acknowledged_total = 0u64;
    for round in 0..12u64 {
        let dir = scratch(round);
        let path = dir.join("A.svandb");
        let manifest = dir.join("A.svandb.manifest");
        let mut child = Command::new(CHILD)
            .current_dir(&dir)
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

        // A kill before the child opened (or first committed) its log leaves no store to read:
        // nothing was acknowledged, so nothing may be lost. Later rounds get further.
        if !path.exists() || !manifest.exists() {
            assert_eq!(acknowledged, 0, "a hand was acknowledged without a store");
            continue;
        }
        let db = Svandb::open(&path, "A").unwrap();
        assert!(
            db.totals().unwrap().hands >= acknowledged,
            "the log has fewer hands than the child acknowledged"
        );
        for at in 0..acknowledged {
            assert!(
                db.holds(&format!("h-{at}")).unwrap(),
                "an acknowledged hand was lost"
            );
        }
        // The log is still writable, and the next clean append survives a reopen.
        db.append_hand(&hand("after"), Source::Live, 99).unwrap();
        drop(db);
        let reopened = Svandb::open(&path, "A").unwrap();
        assert!(
            reopened.holds("after").unwrap(),
            "the append after recovery was lost"
        );
    }
    assert!(
        acknowledged_total > 0,
        "no round got far enough to acknowledge a hand; the test proved nothing"
    );
}

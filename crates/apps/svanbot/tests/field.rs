//! `svanbot field`: a bot's own field, fitted from its own store in the command's process, played
//! as one Arena row recorded apart from the release rows.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::json;
use svanbot_runtime::record::Hand;
use svanbot_runtime::store::{Source, Store};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// One opponent under the hero's store: it raises preflop and the flop.
fn hand(id: &str) -> Hand {
    Hand {
        hand_id: id.into(),
        table_id: "t-1".into(),
        traced: Vec::new(),
        frames: vec![
            json!({
                "hand_id": id, "seat": 0, "dealer_seat": 0,
                "small_blind": 10, "big_blind": 20, "num_players": 2,
                "hole_cards": ["3c", "3d"], "board": ["Ks", "9d", "4c"],
                "shown_cards": {}, "pot_total": 120,
                "all_actions": [
                    {"seat": 1, "sequence_in_hand": 1, "street": "preflop", "action": "raise", "amount": 60},
                    {"seat": 0, "sequence_in_hand": 2, "street": "preflop", "action": "call", "amount": 0},
                    {"seat": 0, "sequence_in_hand": 3, "street": "flop", "action": "check", "amount": 0},
                    {"seat": 1, "sequence_in_hand": 4, "street": "flop", "action": "raise", "amount": 60}
                ]
            })
            .to_string(),
        ],
    }
}

fn store_with(dir: &Path, bot: &str, count: usize) -> PathBuf {
    let path = dir.join(format!("{bot}.db"));
    let store = Store::open(&path, bot).unwrap();
    for at in 0..count {
        store
            .append_hand(
                &hand(&format!("{bot}-{at}")),
                Source::History,
                at as i64 * 1_000,
            )
            .unwrap();
    }
    path
}

/// Runs the command from a scratch store and fleet, and returns what it printed.
fn field(dir: &Path, bot: &str, args: &[&str]) -> (Option<i32>, String, String, PathBuf) {
    let db = store_with(dir, bot, 12);
    let fleet = dir.join("fleet");
    let output = Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .arg("field")
        .args(args)
        .env_clear()
        .env("SVANBOT_NAME", bot)
        .env("SVANBOT_STORE", &db)
        .env("SVANBOT_FLEET", &fleet)
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        db,
    )
}

#[test]
fn the_command_fits_the_bots_own_field_and_records_it_apart() {
    let dir = scratch("entry");
    // Heads up, one bout, an explicit seed, one re-solve iteration: the command's smallest run.
    let (code, stdout, stderr, db) = field(&dir, "Alpha", &["heads-up", "1", "7", "1"]);
    assert_eq!(code, Some(0), "{stdout}\n{stderr}");
    assert!(stdout.contains("field Alpha"), "{stdout}");
    assert!(
        db.with_extension("field").exists(),
        "the field is kept beside the bot's store"
    );

    // The row is apart: it is in the policy record, and the ship bar's arena has no such file.
    let fleet = dir.join("fleet");
    let policy = std::fs::read_to_string(fleet.join("policy-runs.json")).unwrap();
    assert!(policy.contains("\"field Alpha\""), "{policy}");
    assert!(
        !fleet.join("arena.json").exists(),
        "a field row never reaches the release Arena"
    );
}

#[test]
fn the_field_plays_a_full_table_as_one_row_too() {
    let dir = scratch("six-max");
    let (code, stdout, stderr, _) = field(&dir, "Alpha", &["6-max", "1", "7", "1"]);
    assert_eq!(code, Some(0), "{stdout}\n{stderr}");
    assert!(stdout.contains("6-max league"), "{stdout}");
    assert!(stdout.contains("field Alpha"), "{stdout}");
    let policy = std::fs::read_to_string(dir.join("fleet/policy-runs.json")).unwrap();
    assert_eq!(policy.matches("\"field Alpha\"").count(), 1, "{policy}");
}

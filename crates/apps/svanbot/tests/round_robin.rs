//! `svanbot round-robin` end to end: assigned bots, a bounded pass, the fleet's own record, and
//! the release Arena untouched.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::{Path, PathBuf};
use std::process::Command;

use svanbot_learn::round_robin::Table;
use svanbot_runtime::fleet::{Assignment, Fleet};
use svanbot_runtime::round_robin::Record;
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::style;

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn assign(fleet: &Fleet, bot: &str) {
    fleet
        .assign(
            bot,
            Assignment {
                kind: PolicyKind::Solving,
                style: &style::STANDARD,
            },
            0,
        )
        .unwrap();
}

fn run(dir: &Path, rows: usize, bouts: u64) -> (Option<i32>, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .arg("round-robin")
        .args([bouts.to_string(), "7".into(), "1".into()])
        .env_clear()
        .env("SVANBOT_FLEET", dir)
        .env("SVANBOT_ROUND_ROBIN_ROWS", rows.to_string())
        .output()
        .unwrap();
    (
        output.status.code(),
        format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

#[test]
fn a_bounded_pass_records_the_matrix_and_never_touches_the_release_arena() {
    let dir = scratch("bounded");
    let fleet = Fleet::in_dir(&dir);
    assign(&fleet, "Alpha");
    assign(&fleet, "Beta");
    let arena = b"{\"runs\":[]}";
    std::fs::write(dir.join("arena.json"), arena).unwrap();

    let (code, output) = run(&dir, 5, 1);
    assert_eq!(code, Some(0), "{output}");
    assert!(output.contains("5 row(s) played, 35 left"), "{output}");

    let record = Record::load(&dir).unwrap().expect("the pass was recorded");
    assert_eq!(record.played, 5);
    assert_eq!(record.skipped, 35);
    assert_eq!(record.budget, 5);
    assert_eq!(
        record
            .bots
            .iter()
            .map(|bot| bot.bot.as_str())
            .collect::<Vec<_>>(),
        ["Alpha", "Beta"]
    );
    assert_eq!(record.bots[0].rows.len(), 20);
    assert!(
        record.bots[0].rows[..5]
            .iter()
            .all(|row| row.table == Table::HeadsUp && !row.bouts.is_empty())
    );
    assert!(
        record.bots[0].rows[5..]
            .iter()
            .all(|row| row.bouts.is_empty())
    );
    assert_eq!(record.bots[1].rows.len(), 20);
    assert!(record.bots[1].rows.iter().all(|row| row.bouts.is_empty()));

    assert_eq!(std::fs::read(dir.join("arena.json")).unwrap(), arena);
    assert!(!dir.join("policy-runs.json").exists());
}

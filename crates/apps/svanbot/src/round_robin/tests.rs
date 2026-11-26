//! The app's round-robin pass: plan order, the row budget, the archive growth and the record it
//! writes, with the sim stood in for.

use std::path::PathBuf;

use svanbot_learn::archive::Regression;
use svanbot_learn::round_robin::Table;
use svanbot_runtime::fleet::Assignment;
use svanbot_runtime::round_robin::{BotRecord, Record};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::Params;
use svanbot_strategy::style;

use super::*;

fn scratch(name: &str) -> PathBuf {
    // A unit test has no `CARGO_TARGET_TMPDIR`; the build tree's own scratch directory is the
    // same trusted place, under this worktree's `target/`.
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/round-robin-tests")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fleet(name: &str, bots: &[&str]) -> Fleet {
    let fleet = Fleet::in_dir(scratch(name));
    for bot in bots {
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
    fleet
}

fn settings() -> Settings {
    Settings {
        bouts: 1,
        seed: 7,
        iterations: Some(1),
    }
}

#[test]
fn a_pass_plays_every_bot_heads_up_then_the_ring_and_records_the_matrix() {
    let fleet = fleet("matrix", &["Beta", "Alpha"]);
    let mut seen = Vec::new();
    let mut play = |gauntlet: &Gauntlet, row: &Row| {
        seen.push((
            gauntlet.bot.clone(),
            row.name.clone(),
            row.table,
            row.archive,
        ));
        Ok(vec![1.0])
    };
    let record = pass(&fleet, settings(), 250, None, &mut play).unwrap();

    // Bots are read by name, so the matrix is the same pass to pass.
    assert_eq!(
        record
            .bots
            .iter()
            .map(|bot| bot.bot.as_str())
            .collect::<Vec<_>>(),
        ["Alpha", "Beta"]
    );
    assert_eq!(record.played, 40);
    assert_eq!(record.skipped, 0);
    assert_eq!(record.budget, 250);
    assert_eq!(record.bots[0].rows.len(), 20);
    assert!(
        record.bots[0].rows[..10]
            .iter()
            .all(|row| row.table == Table::HeadsUp)
    );
    assert!(
        record.bots[0].rows[10..]
            .iter()
            .all(|row| row.table == Table::SixMax)
    );
    assert_eq!(record.bots[0].regression, Regression::Admitted);
    // The fake played the rows in the plan's order, bot by bot.
    assert_eq!(seen[0].0, "Alpha");
    assert_eq!(seen[0].1, "pricer");
    assert_eq!(seen[0].2, Table::HeadsUp);
    assert_eq!(seen[10].2, Table::SixMax);
    assert_eq!(seen[20].0, "Beta");
    assert_eq!(seen[39].1, "mixing");
}

#[test]
fn the_budget_stops_the_pass_and_the_rest_of_the_plan_is_recorded_as_skipped() {
    let fleet = fleet("budget", &["Alpha", "Beta"]);
    let mut play = |_: &Gauntlet, _: &Row| Ok(vec![1.0]);
    let record = pass(&fleet, settings(), 5, None, &mut play).unwrap();
    assert_eq!(record.played, 5);
    assert_eq!(record.skipped, 35);
    // The matrix keeps every planned row; the five the budget reached hold this pass's bout.
    assert_eq!(record.bots[0].rows.len(), 20);
    assert!(
        record.bots[0].rows[..5]
            .iter()
            .all(|row| row.bouts == [1.0])
    );
    assert!(
        record.bots[0].rows[5..]
            .iter()
            .all(|row| row.bouts.is_empty())
    );
    assert_eq!(record.bots[1].rows.len(), 20);
    assert!(record.bots[1].rows.iter().all(|row| row.bouts.is_empty()));
}

#[test]
fn a_second_pass_appends_to_the_standing_matrix_until_a_champion_changes() {
    let fleet = fleet("standing", &["Alpha"]);
    let first = pass(&fleet, settings(), 20, None, &mut |_, _| Ok(vec![1.0])).unwrap();
    assert_eq!(first.bots[0].rows[0].bouts, [1.0]);
    let second = pass(&fleet, settings(), 20, Some(&first), &mut |_, _| {
        Ok(vec![2.0])
    })
    .unwrap();
    let row = &second.bots[0].rows[0];
    assert_eq!(row.bouts, [1.0, 2.0]);
    assert_eq!(row.mean, 1.5);
    assert_eq!(row.reading().bouts, 2);

    // A new champion starts every row afresh: the old policy's evidence is not about this one.
    fleet
        .set_champion(
            "Alpha",
            &Params {
                exploit: 0.9,
                spread: 0.0,
            },
        )
        .unwrap();
    let third = pass(&fleet, settings(), 20, Some(&second), &mut |_, _| {
        Ok(vec![3.0])
    })
    .unwrap();
    assert_eq!(third.bots[0].rows[0].bouts, [3.0]);
    assert_eq!(
        third.bots[0].archive.len(),
        1,
        "the old champion is archived"
    );
}

#[test]
fn a_champion_change_archives_the_old_one_and_the_pass_measures_the_new_one_against_it() {
    let fleet = fleet("archive", &["Alpha"]);
    let old = Params {
        exploit: 0.4,
        spread: 0.0,
    };
    let new = Params {
        exploit: 0.7,
        spread: 0.0,
    };
    fleet.set_champion("Alpha", &new).unwrap();
    let previous = Record {
        recorded_ms: 1,
        seed: 7,
        bouts: 1,
        iterations: Some(1),
        budget: 250,
        played: 0,
        skipped: 0,
        bots: vec![BotRecord {
            bot: "Alpha".into(),
            policy: "solving/standard".into(),
            champion: old,
            archive: Vec::new(),
            rows: Vec::new(),
            regression: Regression::Admitted,
        }],
    };
    let mut play = |_: &Gauntlet, row: &Row| {
        if row.archive.is_some() {
            Ok(vec![100.0; 30])
        } else {
            Ok(vec![1.0])
        }
    };
    let record = pass(&fleet, settings(), 250, Some(&previous), &mut play).unwrap();
    let alpha = &record.bots[0];
    assert_eq!(alpha.champion, new);
    assert_eq!(alpha.archive.len(), 1);
    assert_eq!(alpha.archive[0].params, old);
    let archive_row = alpha.rows.last().unwrap();
    assert_eq!(archive_row.table, Table::SixMax);
    assert_eq!(archive_row.archive, Some(old.digest()));
    assert_eq!(alpha.rows.len(), 21);
    // Thirty winning bouts are enough for the regression rule to admit the new champion.
    assert_eq!(alpha.regression, Regression::Admitted);
    // The new champion's shared rows are fresh evidence, not the old champion's.
    assert_eq!(alpha.rows[0].bouts, [1.0]);
}

#[test]
fn the_pass_writes_no_file_at_all_so_it_cannot_touch_the_release_arena() {
    let dir = scratch("no-write");
    let fleet = Fleet::in_dir(&dir);
    fleet
        .assign(
            "Alpha",
            Assignment {
                kind: PolicyKind::Solving,
                style: &style::STANDARD,
            },
            0,
        )
        .unwrap();
    let sentinel = b"{\"runs\":[]}";
    std::fs::write(dir.join("arena.json"), sentinel).unwrap();
    let mut play = |_: &Gauntlet, _: &Row| Ok(vec![0.0]);
    pass(&fleet, settings(), 250, None, &mut play).unwrap();
    assert_eq!(std::fs::read(dir.join("arena.json")).unwrap(), sentinel);
    assert!(!dir.join("policy-runs.json").exists());
}

#[test]
fn an_archived_champion_plays_a_real_six_max_bout() {
    let game = deciders::game(7);
    let policy = Policy::reference(Params::DEFAULT);
    let results = play_archived(
        &game,
        League::SixMax,
        &policy,
        Params {
            exploit: 0.4,
            spread: 0.0,
        },
        0,
        Some(1),
    );
    assert_eq!(results.len(), BOUT as usize);
    assert!(results.iter().all(|result| result.is_finite()));
}

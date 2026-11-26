//! The round-robin board (V11-97): the matrix of every bot's gauntlet, and each bot's brain, check
//! and guard, as `round-robin.json` records them.
//!
//! The matrix is rows by bots: the first bot to record a row fixes its place, and every other bot's
//! cell for it is that bot's read against the same row, or empty when its plan had none. The brain
//! and its check come from the live fleet files, not from the record, so a reverted assignment
//! reads as the reference policy and a check against another champion reads as stale. The guard has
//! no record of its own: an assignment is the window the live guard watches, and its `since` is
//! when that window began.
//!
//! A file that is absent is an empty board, not an error. A file that cannot be read is an error
//! beside an empty board, as the search board reports its own.

use std::path::Path;

use serde_json::{Value, json};
use svanbot_learn::archive::Exploitability;
use svanbot_learn::round_robin::Table;
use svanbot_runtime::fleet::{Assignment, Check, Fleet};
use svanbot_runtime::round_robin::{BotRecord, Record, regression_name};
use svanbot_strategy::params::Digest;

/// The round-robin board in `fleet_dir`.
pub(super) fn board(fleet_dir: &Path) -> Value {
    let fleet = Fleet::in_dir(fleet_dir);
    let mut errors = Vec::new();
    let record = match Record::load(fleet_dir) {
        Ok(record) => record,
        Err(why) => {
            errors.push(why);
            None
        }
    };
    let empty = |errors: &[String]| {
        json!({
            "empty": true,
            "error": (!errors.is_empty()).then(|| errors.join("; ")),
            "columns": [],
            "matrix": [],
        })
    };
    let Some(record) = record else {
        return empty(&errors);
    };
    let assignments = fleet.assignments().unwrap_or_else(|why| {
        errors.push(why);
        Vec::new()
    });
    let checks = fleet.checks().unwrap_or_else(|why| {
        errors.push(why);
        Vec::new()
    });
    let champion = fleet.champion().map_err(|why| errors.push(why)).ok();
    let columns: Vec<Value> = record
        .bots
        .iter()
        .map(|bot| {
            column(
                bot,
                &assignments,
                &checks,
                champion.as_ref().map(|c| c.digest()),
            )
        })
        .collect();
    json!({
        "empty": false,
        "error": (!errors.is_empty()).then(|| errors.join("; ")),
        "recorded_ms": record.recorded_ms,
        "seed": record.seed,
        "bouts": record.bouts,
        "iterations": record.iterations,
        "budget": record.budget,
        "played": record.played,
        "skipped": record.skipped,
        "columns": columns,
        "matrix": matrix(&record),
    })
}

/// One bot's column: the brain it plays now, that brain's latest check, the live guard's window,
/// and the record's regression verdict and measured policy.
fn column(
    record: &BotRecord,
    assignments: &[(String, Assignment, i64)],
    checks: &[Check],
    champion: Option<Digest>,
) -> Value {
    let assigned = assignments.iter().find(|(bot, _, _)| bot == &record.bot);
    let brain = assigned.map(|(_, assignment, _)| {
        json!({
            "kind": assignment.kind.name(),
            "style": assignment.style.name,
        })
    });
    let check = assigned.and_then(|(_, assignment, _)| {
        checks
            .iter()
            .find(|check| check.kind == assignment.kind && check.style == assignment.style.name)
    });
    let measured = assigned.is_some_and(|(_, assignment, _)| {
        format!("{}/{}", assignment.kind.name(), assignment.style.name) == record.policy
    });
    json!({
        "bot": record.bot,
        "policy": record.policy,
        "champion": record.champion.digest().to_string(),
        "regression": regression_name(record.regression),
        "measured": measured,
        "brain": brain,
        // The best exploiter's win rate against this champion, when one has been confirmed: the
        // brain's exploitability, beside the matrix's win rates. A change of champion restarts
        // the search, so a measurement about another champion reads as stale, not as this one's.
        "exploitability": record.exploiter.as_ref().map(|found| json!({
            "mbb_per_100": found.win_rate,
            "radius": found.radius,
            "deals": found.deals,
            "champion": found.champion.to_string(),
            "fresh": found.champion == record.champion.digest(),
            "within_bound": Exploitability::Measured(found.win_rate).within_bound(),
        })),
        "check": check.map(|check| json!({
            "admitted": check.admitted,
            "stale": champion.map(|champion| check.reference != champion),
            "checked_ms": check.checked_ms,
            "bouts": check.bouts,
            "heads_up": check.heads_up,
            "six_max": check.six_max,
        })),
        "guard": assigned.map(|(_, _, since)| json!({
            "state": "watching",
            "since_ms": since,
        })),
    })
}

/// The matrix: each row the first time a bot records it, with one cell per bot in record order.
fn matrix(record: &Record) -> Vec<Value> {
    let mut keys: Vec<(&str, Table, Option<Digest>, Option<Digest>)> = Vec::new();
    for bot in &record.bots {
        for row in &bot.rows {
            let key = (row.row.as_str(), row.table, row.archive, row.exploiter);
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys.into_iter()
        .map(|(name, table, archive, exploiter)| {
            json!({
                "row": name,
                "table": table.name(),
                "archive": archive.map(|champion| champion.to_string()),
                "exploiter": exploiter.map(|exploiter| exploiter.to_string()),
                "cells": record.bots.iter().map(|bot| {
                    bot.rows
                        .iter()
                        .find(|row| row.row == name && row.table == table
                            && row.archive == archive && row.exploiter == exploiter)
                        .map(|row| json!({
                            "mean": row.mean,
                            "radius": row.radius,
                            "bouts": row.bouts.len(),
                            "verdict": verdict(row.mean, row.radius),
                        }))
                }).collect::<Vec<_>>(),
            })
        })
        .collect()
}

/// A row's read from the bot's seat: ahead or behind when the whole sequence bound is off zero,
/// even when it straddles, and unresolved before two bouts gave it a bound.
fn verdict(mean: f64, radius: Option<f64>) -> &'static str {
    match radius {
        None => "unresolved",
        Some(radius) if mean - radius > 0.0 => "ahead",
        Some(radius) if mean + radius < 0.0 => "behind",
        Some(_) => "even",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::Value;
    use svanbot_learn::archive::Regression;
    use svanbot_runtime::fleet::{Assignment, Check, Fleet};
    use svanbot_runtime::round_robin::{
        ArchivedChampion, BotRecord, ExploiterRecord, Record, RowRecord,
    };
    use svanbot_strategy::kind::PolicyKind;
    use svanbot_strategy::params::{Digest, Params};
    use svanbot_strategy::style::{LOOSE, TIGHT};

    use super::{Table, board};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "svanbot-round-robin-board-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn digest(value: u64) -> Digest {
        format!("{value:016x}").parse().unwrap()
    }

    fn params(exploit: f64) -> Params {
        Params {
            exploit,
            spread: 0.0,
            ..Params::DEFAULT
        }
    }

    fn row(
        name: &str,
        table: Table,
        archive: Option<u64>,
        mean: f64,
        radius: Option<f64>,
    ) -> RowRecord {
        RowRecord {
            row: name.into(),
            table,
            archive: archive.map(digest),
            exploiter: None,
            bouts: vec![mean; 2],
            mean,
            radius,
        }
    }

    fn record() -> Record {
        Record {
            recorded_ms: 1_700_000_000_000,
            seed: 7,
            bouts: 1,
            iterations: Some(2),
            budget: 60,
            played: 4,
            skipped: 56,
            bots: vec![
                BotRecord {
                    bot: "Alpha".into(),
                    policy: "solving/tight".into(),
                    champion: params(0.2),
                    archive: vec![ArchivedChampion {
                        params: params(1.0 / 7.0),
                    }],
                    exploiter_log: Vec::new(),
                    // The best exploiter measured against this very champion: 4 bb/100, inside
                    // the +5 bb/100 bound, and fresh.
                    exploiter: Some(ExploiterRecord {
                        params: params(0.3),
                        champion: params(0.2).digest(),
                        win_rate: 4_000.0,
                        radius: 300.0,
                        deals: 500,
                    }),
                    rows: vec![
                        row("pricer", Table::HeadsUp, None, -500.0, Some(1_500.0)),
                        row("nit", Table::HeadsUp, None, 3_000.0, Some(500.0)),
                        row(
                            "archive 0000000000000001",
                            Table::SixMax,
                            Some(1),
                            -1_000.0,
                            Some(500.0),
                        ),
                    ],
                    regression: Regression::Refused {
                        champion: digest(1),
                    },
                },
                BotRecord {
                    bot: "Beta".into(),
                    policy: "solving/loose".into(),
                    champion: Params::DEFAULT,
                    archive: Vec::new(),
                    exploiter_log: Vec::new(),
                    exploiter: None,
                    rows: vec![row("mixing", Table::SixMax, None, 0.0, None)],
                    regression: Regression::Admitted,
                },
            ],
        }
    }

    #[test]
    fn the_board_reads_the_matrix_and_each_bots_brain_check_and_guard() {
        let dir = scratch("mine");
        record().save(&dir).unwrap();
        let fleet = Fleet::in_dir(&dir);
        fleet
            .assign(
                "Alpha",
                Assignment {
                    kind: PolicyKind::Solving,
                    style: &TIGHT,
                },
                1_500,
            )
            .unwrap();
        fleet
            .assign(
                "Beta",
                Assignment {
                    kind: PolicyKind::Solving,
                    style: &LOOSE,
                },
                1_700,
            )
            .unwrap();
        fleet
            .record_check(&Check {
                kind: PolicyKind::Solving,
                style: "tight".into(),
                reference: Params::DEFAULT.digest(),
                seed: 7,
                iterations: None,
                bouts: 60,
                admitted: true,
                heads_up: "heads up evidence".into(),
                six_max: "6-max evidence".into(),
                checked_ms: 1_600,
            })
            .unwrap();

        let found = board(&dir);
        assert_eq!(found["empty"], false);
        assert_eq!(found["seed"], 7);
        assert_eq!(found["played"], 4);
        let alpha = &found["columns"][0];
        assert_eq!(alpha["bot"], "Alpha");
        assert_eq!(alpha["brain"]["kind"], "solving");
        assert_eq!(alpha["brain"]["style"], "tight");
        assert_eq!(alpha["check"]["admitted"], true);
        assert_eq!(alpha["check"]["stale"], false);
        assert_eq!(alpha["check"]["checked_ms"], 1_600);
        assert_eq!(alpha["guard"]["state"], "watching");
        assert_eq!(alpha["guard"]["since_ms"], 1_500);
        assert_eq!(alpha["regression"], format!("refused {}", digest(1)));
        assert_eq!(alpha["measured"], true);
        assert_eq!(alpha["exploitability"]["mbb_per_100"], 4_000.0);
        assert_eq!(alpha["exploitability"]["radius"], 300.0);
        assert_eq!(alpha["exploitability"]["deals"], 500);
        assert_eq!(alpha["exploitability"]["fresh"], true);
        assert_eq!(alpha["exploitability"]["within_bound"], true);
        let beta = &found["columns"][1];
        assert!(beta["check"].is_null(), "Beta has no check");
        assert!(beta["exploitability"].is_null(), "Beta has no exploiter");
        assert_eq!(beta["guard"]["since_ms"], 1_700);

        let rows = found["matrix"].as_array().unwrap();
        let row = |name: &str| rows.iter().find(|row| row["row"] == name).unwrap();
        assert_eq!(row("pricer")["cells"][1], Value::Null);
        assert_eq!(row("pricer")["cells"][0]["verdict"], "even");
        assert_eq!(row("nit")["cells"][0]["verdict"], "ahead");
        assert_eq!(
            row("archive 0000000000000001")["cells"][0]["verdict"],
            "behind"
        );
        assert_eq!(row("mixing")["cells"][0], Value::Null);
        assert_eq!(row("mixing")["cells"][1]["verdict"], "unresolved");

        fleet.promote(&params(0.4)).unwrap();
        let promoted = board(&dir);
        assert_eq!(promoted["columns"][0]["check"]["stale"], true);
    }

    #[test]
    fn a_record_that_is_not_there_is_an_empty_board_not_an_error() {
        let dir = scratch("empty");
        let found = board(&dir);
        assert_eq!(found["empty"], true);
        assert_eq!(found["error"], Value::Null);
        assert_eq!(found["matrix"], Value::Array(Vec::new()));
        assert_eq!(found["columns"], Value::Array(Vec::new()));
    }

    #[test]
    fn a_record_that_cannot_be_read_is_reported_beside_the_empty_board() {
        let dir = scratch("broken");
        std::fs::write(dir.join("round-robin.json"), "{").unwrap();
        let found = board(&dir);
        assert_eq!(found["empty"], true);
        assert!(
            found["error"]
                .as_str()
                .unwrap()
                .contains("round-robin.json"),
            "{}",
            found["error"]
        );
    }
}

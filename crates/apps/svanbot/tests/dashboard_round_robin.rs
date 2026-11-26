//! The round-robin board in the fleet document (V11-97): the matrix, and each bot's brain, check
//! and guard, read from `round-robin.json` and the fleet's policy files.
// A test server is a socket and files in a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

mod support;

use serde_json::Value;
use support::{get, serve};
use svanbot_learn::archive::Regression;
use svanbot_learn::round_robin::Table;
use svanbot_runtime::fleet::{Assignment, Check, Fleet};
use svanbot_runtime::round_robin::{BotRecord, Record, RowRecord};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::Params;
use svanbot_strategy::style::TIGHT;

#[test]
fn the_fleet_carries_the_round_robin_board_with_each_bot_brain_check_and_guard() {
    let (addr, dir) = serve("round-robin-board");
    let (_, body) = get(addr, "/api/fleet");
    let fleet: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(fleet["round_robin"]["empty"], true);
    assert_eq!(fleet["round_robin"]["matrix"], serde_json::json!([]));

    let fleet_dir = dir.join("data").join("fleet");
    let record = Record {
        recorded_ms: 1_700_000_000_000,
        seed: 7,
        bouts: 1,
        iterations: None,
        budget: 60,
        played: 1,
        skipped: 59,
        bots: vec![BotRecord {
            bot: "Alpha".into(),
            policy: "solving/tight".into(),
            champion: Params::DEFAULT,
            archive: Vec::new(),
            rows: vec![RowRecord {
                row: "nit".into(),
                table: Table::HeadsUp,
                archive: None,
                bouts: vec![1_000.0; 2],
                mean: 1_000.0,
                radius: Some(500.0),
            }],
            regression: Regression::Admitted,
        }],
    };
    record.save(&fleet_dir).unwrap();
    let fleet_file = Fleet::in_dir(&fleet_dir);
    fleet_file
        .assign(
            "Alpha",
            Assignment {
                kind: PolicyKind::Solving,
                style: &TIGHT,
            },
            1_500,
        )
        .unwrap();
    fleet_file
        .record_check(&Check {
            kind: PolicyKind::Solving,
            style: "tight".into(),
            reference: Params::DEFAULT.digest(),
            seed: 7,
            iterations: None,
            bouts: 60,
            admitted: true,
            heads_up: "heads up".into(),
            six_max: "6-max".into(),
            checked_ms: 1_600,
        })
        .unwrap();

    let (_, body) = get(addr, "/api/fleet");
    let board = &serde_json::from_str::<Value>(&body).unwrap()["round_robin"];
    assert_eq!(board["empty"], false);
    assert_eq!(board["columns"][0]["bot"], "Alpha");
    assert_eq!(board["columns"][0]["brain"]["kind"], "solving");
    assert_eq!(board["columns"][0]["brain"]["style"], "tight");
    assert_eq!(board["columns"][0]["check"]["admitted"], true);
    assert_eq!(board["columns"][0]["guard"]["since_ms"], 1_500);
    assert_eq!(board["matrix"][0]["row"], "nit");
    assert_eq!(board["matrix"][0]["cells"][0]["verdict"], "ahead");
}

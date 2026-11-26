//! The round-robin's own fleet file: one bounded pass, its matrix, its archive, and the
//! regression verdict, read back to the bit, apart from `arena.json`.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;

use svanbot_learn::archive::Regression;
use svanbot_learn::paired::Verdict;
use svanbot_learn::round_robin::Table;
use svanbot_learn::search::{Entry, Stage, Step};
use svanbot_runtime::round_robin::{
    ArchivedChampion, BotRecord, ExploiterRecord, FILE, KEEP, Record, RowRecord,
};
use svanbot_strategy::params::{Digest, Params};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
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

fn row(name: &str, table: Table, archive: Option<u64>, bouts: Vec<f64>) -> RowRecord {
    let mean = bouts.iter().sum::<f64>() / bouts.len().max(1) as f64;
    RowRecord {
        row: name.into(),
        table,
        archive: archive.map(digest),
        exploiter: None,
        bouts,
        mean,
        radius: Some(1.0 + 0.5),
    }
}

fn record() -> Record {
    Record {
        recorded_ms: 1_700_000_000_000,
        seed: 7,
        bouts: 1,
        iterations: Some(2),
        budget: 60,
        played: 3,
        skipped: 57,
        bots: vec![
            BotRecord {
                bot: "Alpha".into(),
                policy: "solving/standard".into(),
                // A number with no short decimal form must come back to the bit.
                champion: params(0.1 + 0.2),
                archive: vec![ArchivedChampion {
                    params: params(1.0 / 7.0),
                }],
                // The exploiter search's log, and the best exploiter it has confirmed: the
                // numbers must come back to the bit as the archive's do.
                exploiter_log: vec![Entry::Simmed {
                    step: Step {
                        champion: params(0.1 + 0.2).digest(),
                        challenger: params(1.0 / 3.0),
                        stage: Stage::First,
                        seed: u64::MAX - 1,
                    },
                    verdict: Verdict {
                        deals: 100,
                        mbb_per_100: 812.5,
                        ci95: 95.25,
                    },
                }],
                exploiter: Some(ExploiterRecord {
                    params: params(1.0 / 3.0),
                    champion: params(0.1 + 0.2).digest(),
                    win_rate: 3_450.25,
                    radius: 120.5,
                    deals: 200,
                }),
                rows: vec![
                    row("pricer", Table::HeadsUp, None, vec![1.0, -2.0]),
                    row("nit", Table::HeadsUp, None, vec![3.0]),
                    row(
                        &format!("archive {}", digest(1)),
                        Table::SixMax,
                        Some(1),
                        vec![-100.0; KEEP],
                    ),
                ],
                regression: Regression::Refused {
                    champion: digest(1),
                },
            },
            BotRecord {
                bot: "Beta".into(),
                policy: "solving/tight".into(),
                champion: Params::DEFAULT,
                archive: Vec::new(),
                exploiter_log: Vec::new(),
                exploiter: None,
                rows: vec![row("mixing", Table::SixMax, None, vec![0.0; 2])],
                regression: Regression::Admitted,
            },
        ],
    }
}

#[test]
fn a_pass_reads_back_to_the_bit_with_its_matrix_archive_and_verdicts() {
    let dir = scratch("round-trip");
    let record = record();
    record.save(&dir).unwrap();
    assert!(dir.join(FILE).exists());
    assert!(!dir.join("arena.json").exists(), "never the release Arena");
    assert_eq!(Record::load(&dir).unwrap(), Some(record.clone()));
    assert_eq!(
        record.bot("Alpha").map(|bot| bot.rows.len()),
        Some(3),
        "the matrix keeps every row in order"
    );
    assert_eq!(record.bot("Missing"), None);
}

#[test]
fn no_record_yet_is_no_pass_yet() {
    assert_eq!(Record::load(&scratch("missing")).unwrap(), None);
}

#[test]
fn a_record_written_before_exploiters_still_loads() {
    let dir = scratch("before-exploiters");
    // The same record with the two fields V11-100 added absent, as an older pass wrote it.
    let mut value: serde_json::Value = serde_json::from_str(&record().as_text()).unwrap();
    for bot in value["bots"].as_array_mut().unwrap() {
        let bot = bot.as_object_mut().unwrap();
        bot.remove("exploiter");
        bot.remove("exploiter_log");
    }
    std::fs::write(dir.join(FILE), value.to_string()).unwrap();
    let loaded = Record::load(&dir).unwrap().unwrap();
    assert_eq!(loaded.bots.len(), 2);
    assert!(
        loaded
            .bots
            .iter()
            .all(|bot| bot.exploiter.is_none() && bot.exploiter_log.is_empty())
    );
    // Everything the older record did carry still reads.
    assert_eq!(loaded.bot("Alpha").unwrap().rows.len(), 3);
}

#[test]
fn a_record_that_does_not_name_its_table_or_verdict_is_refused() {
    let dir = scratch("broken");
    let text = record().as_text();
    std::fs::write(dir.join(FILE), text.replace("6-max", "final table")).unwrap();
    assert!(Record::load(&dir).unwrap_err().contains("table"));

    let text = record().as_text();
    std::fs::write(dir.join(FILE), text.replace("admitted", "fine")).unwrap();
    assert!(Record::load(&dir).unwrap_err().contains("verdict"));
}

//! The round-robin plan, its bounded runner, the Sequence read, and the regression rule's
//! evidence: one bot's gauntlet is ten shared pairs heads up, the same ten at the 6-max ring, then
//! one heads-up row against each archived champion.

use std::cell::RefCell;

use svanbot_learn::archive::{Archive, Archived, Regression};
use svanbot_learn::round_robin::{Budget, MAX_ROWS, Table, plan, read, regression, run};
use svanbot_strategy::params::{Digest, Params};

fn digest(value: u64) -> Digest {
    format!("{value:016x}").parse().unwrap()
}

fn params(value: u64) -> Params {
    Params {
        exploit: value as f64 / 10.0,
        spread: 0.0,
    }
}

fn archived(value: u64) -> Archived {
    Archived {
        params: params(value),
        style: "standard",
        promotion: Vec::new(),
    }
}

fn archived_digest(value: u64) -> Digest {
    params(value).digest()
}

/// The ten shared rows the Arena has today: the pricer, v10.0.2 and the eight archetypes.
fn shared() -> Vec<String> {
    [
        "pricer",
        "v10.0.2",
        "calling station",
        "nit",
        "maniac",
        "switching",
        "learning",
        "sizing",
        "bluffer",
        "mixing",
    ]
    .map(String::from)
    .to_vec()
}

#[test]
fn a_plan_is_ten_shared_pairs_heads_up_then_the_same_ring_then_the_archive() {
    let mut archive = Archive::new();
    archive.record(archived(1));
    archive.record(archived(2));
    let plan = plan("Alpha", digest(9), &shared(), &archive);

    assert_eq!(plan.bot, "Alpha");
    assert_eq!(plan.champion, digest(9));
    assert_eq!(plan.rows.len(), 22);
    assert!(
        plan.rows[..10]
            .iter()
            .all(|row| row.table == Table::HeadsUp)
    );
    assert_eq!(plan.rows[0].name, "pricer");
    assert_eq!(plan.rows[9].name, "mixing");
    assert!(
        plan.rows[10..20]
            .iter()
            .all(|row| row.table == Table::SixMax)
    );
    assert_eq!(plan.rows[10].name, "pricer");
    // The archive is newest first, and its rows are played at the 6-max ring.
    assert_eq!(
        plan.rows[20].name,
        format!("archive {}", archived_digest(2))
    );
    assert_eq!(plan.rows[20].archive, Some(archived_digest(2)));
    assert_eq!(plan.rows[21].archive, Some(archived_digest(1)));
    assert_eq!(plan.rows[20].table, Table::SixMax);
}

#[test]
fn the_runner_plays_in_plan_order_and_stops_at_its_budget() {
    let plan = plan("Alpha", digest(9), &shared(), &Archive::new());
    let seen = RefCell::new(Vec::new());
    let run = run(&plan, Budget { rows: 3 }, |row| {
        seen.borrow_mut().push(row.name.clone());
        vec![1.0, 2.0]
    });

    let played: Vec<_> = plan.rows[..3].iter().map(|row| row.name.clone()).collect();
    assert_eq!(*seen.borrow(), played);
    assert_eq!(run.rows.len(), 3);
    assert_eq!(run.left, plan.rows.len() - 3);
    assert_eq!(run.rows[0].reading.bouts, 2);
    assert!(MAX_ROWS >= plan.rows.len(), "a pass bounds the whole plan");
}

#[test]
fn a_reading_is_the_sequence_read_over_the_paired_bouts() {
    let reading = read(&[1.0, 2.0, 3.0]);
    assert_eq!(reading.bouts, 3);
    assert!((reading.mean - 2.0).abs() < 1e-12);
    assert!(reading.radius.is_finite(), "three bouts give an interval");
    assert!(reading.radius >= 0.0);

    let none = read(&[]);
    assert_eq!(none.bouts, 0);
    assert_eq!(none.mean, 0.0);
    assert!(none.radius.is_infinite(), "no evidence is no interval");
}

#[test]
fn the_archive_rows_are_the_evidence_the_regression_rule_reads() {
    let mut archive = Archive::new();
    archive.record(archived(1));
    let plan = plan("Alpha", digest(9), &[], &archive);
    assert_eq!(plan.rows.len(), 1);

    // Thirty paired bouts, each +100 mbb/100 against the archived champion: a clear win.
    let won = run(&plan, Budget { rows: 1 }, |_| vec![100.0; 30]);
    assert_eq!(regression(&archive, &won.rows), Regression::Admitted);

    // The same rows losing: the promotion is refused against that champion.
    let lost = run(&plan, Budget { rows: 1 }, |_| vec![-100.0; 30]);
    assert_eq!(
        regression(&archive, &lost.rows),
        Regression::Refused {
            champion: archived_digest(1)
        }
    );

    // No archive, nothing to lose to: the first champion is admitted.
    assert_eq!(regression(&Archive::new(), &[]), Regression::Admitted);
}

#[test]
fn a_shared_row_carries_no_archive_and_is_no_regression_evidence() {
    let plan = plan("Alpha", digest(9), &["pricer".into()], &Archive::new());
    assert_eq!(plan.rows.len(), 2);
    assert!(plan.rows.iter().all(|row| row.archive.is_none()));
    let run = run(&plan, Budget { rows: 2 }, |_| vec![-100.0; 30]);
    assert_eq!(regression(&Archive::new(), &run.rows), Regression::Admitted);
}

//! `svanbot learn` (V11-49): one bounded step of the unattended search a run, against a fleet in
//! a scratch directory and a stand-in for the paired sim.

use std::cell::RefCell;
use std::path::PathBuf;

use svanbot::learn::{GUARD_HANDS, step};
use svanbot_learn::paired::Verdict;
use svanbot_learn::probe::Kind;
use svanbot_learn::search::{Entry, Stage, Step};
use svanbot_runtime::fleet::{Fleet, Proposal};
use svanbot_strategy::params::Params;

fn scratch(name: &str) -> Fleet {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("svanbot-learn-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Fleet::in_dir(dir)
}

/// A stand-in sim that finds `mbb` mbb/100, ±100, for every step, and keeps what it was asked.
struct Stub {
    mbb: f64,
    asked: RefCell<Vec<(Step, u64)>>,
}

impl Stub {
    fn finding(mbb: f64) -> Self {
        Self {
            mbb,
            asked: RefCell::new(Vec::new()),
        }
    }

    fn sim(&self) -> impl Fn(&Step, u64) -> Verdict + '_ {
        move |step, deals| {
            self.asked.borrow_mut().push((*step, deals));
            Verdict {
                deals: deals as usize,
                mbb_per_100: self.mbb,
                ci95: 100.0,
            }
        }
    }
}

fn never(_: &Step, _: u64) -> Verdict {
    panic!("no sim may run while evidence is pending")
}

#[test]
fn each_run_sims_one_step_logs_it_and_the_next_run_sims_the_next() {
    let fleet = scratch("one-step");
    let level = Stub::finding(20.0);
    let note = step(&fleet, 2_000, 100, &level.sim()).unwrap();
    let asked = level.asked.borrow().clone();
    assert_eq!(asked.len(), 1, "one sim a run");
    let (first, deals) = asked[0];
    assert_eq!(deals, 2_000);
    assert_eq!(
        first.challenger,
        Params {
            exploit: 0.4,
            spread: 0.0,
            ..Params::DEFAULT
        }
    );
    assert!(note.contains("simmed"), "{note}");
    let log = fleet.search_log().unwrap();
    assert_eq!(log.entries.len(), 1);
    assert_eq!(log.last, Some((100, note)));

    step(&fleet, 2_000, 200, &level.sim()).unwrap();
    let asked = level.asked.borrow().clone();
    assert_eq!(asked.len(), 2);
    assert_ne!(
        asked[1].0.challenger, first.challenger,
        "a step is never simmed twice"
    );
    assert!(
        fleet.waiting().unwrap().is_none(),
        "nothing above: nothing proposed"
    );
}

#[test]
fn a_challenger_two_sims_put_above_is_proposed_for_the_harm_guard_against_its_champion() {
    let fleet = scratch("proposes");
    let above = Stub::finding(300.0);
    step(&fleet, 2_000, 100, &above.sim()).unwrap();
    let note = step(&fleet, 2_000, 200, &above.sim()).unwrap();
    let asked = above.asked.borrow().clone();
    assert_eq!(asked[1].0.stage, Stage::Confirmation);
    assert!(note.contains("proposed"), "{note}");

    let proposal = fleet
        .waiting()
        .unwrap()
        .expect("a proposal waits for the canary");
    assert_eq!(
        proposal.challenger,
        Params {
            exploit: 0.4,
            spread: 0.0,
            ..Params::DEFAULT
        }
    );
    assert_eq!(proposal.kind, Kind::Guard { hands: GUARD_HANDS });
    assert_eq!(proposal.against, Some(Params::DEFAULT.digest()));
    assert_eq!(proposal.sim_estimate, Some(0.3));
    assert!(matches!(
        fleet.search_log().unwrap().entries.last(),
        Some(Entry::Proposed { .. })
    ));
}

#[test]
fn while_a_proposal_waits_or_a_probe_runs_a_run_sims_nothing_and_says_it_waits() {
    let fleet = scratch("waits");
    let waiting = Proposal {
        challenger: Params {
            exploit: 0.7,
            spread: 0.0,
            ..Params::DEFAULT
        },
        kind: Kind::Probe,
        harm_margin: 5.0,
        sim_estimate: None,
        max_orbits: 100,
        against: None,
    };
    fleet.propose(&waiting).unwrap();
    let note = step(&fleet, 2_000, 100, &never).unwrap();
    assert!(note.contains("waits"), "{note}");
    // Taken up by the canary, it is a running probe: still nothing to do.
    fleet.take_proposal().unwrap();
    let note = step(&fleet, 2_000, 200, &never).unwrap();
    assert!(note.contains("probe"), "{note}");
    assert_eq!(fleet.search_log().unwrap().entries, Vec::new());
    assert_eq!(fleet.search_log().unwrap().last, Some((200, note)));
}

#[test]
fn a_frozen_reference_stops_the_search_and_a_restart_does_not_resume_it() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("frozen-search");
    let _ = std::fs::remove_dir_all(&dir);
    let fleet = Fleet::in_dir(&dir);
    fleet.freeze(100).unwrap();

    let first = step(&fleet, 2_000, 200, &never).unwrap();
    assert!(first.contains("frozen"), "{first}");
    // A restart is a fresh Fleet over the same files, as the timer's next process is.
    let restarted = Fleet::in_dir(&dir);
    let second = step(&restarted, 2_000, 300, &never).unwrap();
    assert!(second.contains("frozen"), "{second}");
    assert_eq!(restarted.search_log().unwrap().entries, Vec::new());
    assert_eq!(restarted.search_log().unwrap().last, Some((300, second)));
    assert!(restarted.waiting().unwrap().is_none());
}

#[test]
fn a_spent_search_sims_nothing_until_the_champion_changes() {
    let fleet = scratch("spent");
    let level = Stub::finding(0.0);
    for at in 0..6 {
        step(&fleet, 2_000, at, &level.sim()).unwrap();
    }
    // The sixth sim exhausts the search and writes the bot's own experiment; the canary takes
    // the offer up, then the spent search offers the waiting experiment again without a sim.
    let proposal = fleet
        .take_proposal()
        .unwrap()
        .expect("the experiment's proposal");
    fleet.end_probe().unwrap();
    assert_eq!(
        fleet
            .waiting_experiment()
            .unwrap()
            .expect("a waiting experiment")
            .experiment
            .claim
            .challenger(),
        proposal.challenger
    );
    let note = step(&fleet, 2_000, 10, &never).unwrap();
    assert!(
        note.contains("spent") && note.contains("experiment"),
        "{note}"
    );
    assert_eq!(level.asked.borrow().len(), 6, "a spent search sims nothing");
    let _ = fleet
        .take_proposal()
        .unwrap()
        .expect("the experiment is offered again");
    fleet.end_probe().unwrap();
    // A new champion is searched afresh.
    fleet
        .promote(&Params {
            exploit: 0.4,
            spread: 0.0,
            ..Params::DEFAULT
        })
        .unwrap();
    step(&fleet, 2_000, 20, &level.sim()).unwrap();
    assert_eq!(level.asked.borrow().len(), 7);
}

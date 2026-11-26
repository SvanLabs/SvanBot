//! `svanbot learn` over the bots with a champion slot (V11-76): one bounded step a run, rotating
//! through the bots, each simulated with its own kind and style.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::process::Command;

use svanbot::learn::bot_step;
use svanbot_learn::paired::Verdict;
use svanbot_learn::search::{Stage, Step};
use svanbot_runtime::fleet::{Assignment, Candidate, Fleet};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::Params;
use svanbot_strategy::policy::Policy;
use svanbot_strategy::style::{LOOSE, TIGHT};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-bot-learn-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn tight() -> Assignment {
    Assignment {
        kind: PolicyKind::Solving,
        style: &TIGHT,
    }
}

fn loose() -> Assignment {
    Assignment {
        kind: PolicyKind::Solving,
        style: &LOOSE,
    }
}

fn below(deals: u64) -> Verdict {
    Verdict {
        deals: deals as usize,
        mbb_per_100: -50.0,
        ci95: 10.0,
    }
}

#[test]
fn two_slots_rotate_and_each_search_plays_its_own_kind_and_style() {
    let dir = scratch("rotate");
    let fleet = Fleet::in_dir(&dir);
    let a = Params { exploit: 0.4 };
    let b = Params { exploit: 0.6 };
    fleet.set_champion("A", &a).unwrap();
    fleet.set_champion("B", &b).unwrap();
    fleet.assign("A", tight(), 1).unwrap();
    fleet.assign("B", loose(), 1).unwrap();

    let calls = RefCell::new(Vec::new());
    let sim = |challenger: &Policy, champion: &Policy, step: &Step, deals: u64| {
        calls.borrow_mut().push((
            challenger.name(),
            champion.name(),
            step.challenger,
            champion.params,
            step.stage,
            deals,
        ));
        below(deals)
    };

    let note = bot_step(&fleet, 200, 1_600, 100, &sim).unwrap();
    assert!(note.starts_with("A "), "{note}");
    let note = bot_step(&fleet, 200, 1_600, 200, &sim).unwrap();
    assert!(note.starts_with("B "), "{note}");
    bot_step(&fleet, 200, 1_600, 300, &sim).unwrap();
    {
        let calls = calls.borrow();
        assert_eq!(calls.len(), 3);
        assert_eq!(
            (calls[0].0.as_str(), calls[0].1.as_str()),
            ("solving/tight", "solving/tight")
        );
        assert_eq!(calls[0].2, Params { exploit: 0.4 * 0.8 });
        assert_eq!(calls[0].3, a);
        assert_eq!(calls[0].4, Stage::First);
        assert_eq!(
            (calls[1].0.as_str(), calls[1].1.as_str()),
            ("solving/loose", "solving/loose")
        );
        assert_eq!(calls[1].2, Params { exploit: 0.6 * 0.8 });
        assert_eq!(calls[1].3, b);
        assert_eq!(
            calls[2].3, a,
            "A's turn to search again, and its champion is A's slot"
        );
    }
    // A restart reads the last run from the fleet files and continues the rotation.
    let reloaded = Fleet::in_dir(&dir);
    let note = bot_step(&reloaded, 200, 1_600, 400, &sim).unwrap();
    assert!(
        note.starts_with("B "),
        "after A at 100 and 300, B is oldest: {note}"
    );
    // A champion change invalidates A's progress: A is searched from its new champion.
    let promoted = Params { exploit: 0.35 };
    fleet.set_champion("A", &promoted).unwrap();
    let note = bot_step(&fleet, 200, 1_600, 500, &sim).unwrap();
    assert!(note.starts_with("A "), "{note}");
    let calls = calls.borrow();
    let last = calls.last().unwrap();
    assert_eq!(
        last.2,
        Params {
            exploit: 0.35 * 0.8
        }
    );
    assert_eq!(last.3, promoted);
}

#[test]
fn promising_candidates_get_the_long_confirmation_and_become_a_candidate() {
    let fleet = Fleet::in_dir(scratch("candidate"));
    let champion = Params { exploit: 0.4 };
    let promising = Params { exploit: 0.4 * 0.8 };
    fleet.set_champion("A", &champion).unwrap();
    fleet.assign("A", tight(), 1).unwrap();
    let calls = RefCell::new(Vec::new());
    let sim = |_challenger: &Policy, _champion: &Policy, step: &Step, deals: u64| {
        calls
            .borrow_mut()
            .push((step.stage, step.challenger, deals));
        let mbb = match step.stage {
            Stage::First if step.challenger == promising => 200.0,
            Stage::First => -50.0,
            Stage::Confirmation => 300.0,
        };
        Verdict {
            deals: deals as usize,
            mbb_per_100: mbb,
            ci95: 100.0,
        }
    };
    // Six short screens of the six one-step challengers.
    for at in 0..6 {
        bot_step(&fleet, 200, 1_600, at, &sim).unwrap();
    }
    let note = bot_step(&fleet, 200, 1_600, 6, &sim).unwrap();
    assert!(note.contains("confirmed"), "{note}");
    let note = bot_step(&fleet, 200, 1_600, 7, &sim).unwrap();
    assert!(note.contains("candidate"), "{note}");
    {
        let calls = calls.borrow();
        assert_eq!(calls.len(), 7, "the candidate run sims nothing");
        assert!(calls[..6].iter().all(|call| call.0 == Stage::First));
        assert!(calls[..6].iter().all(|call| call.2 == 200));
        assert_eq!(calls[6].0, Stage::Confirmation);
        assert_eq!(calls[6].2, 1_600, "the confirmation gets more CPU");
    }
    let search = fleet.bot_search("A", &champion).unwrap();
    assert_eq!(
        search.candidate,
        Some(Candidate {
            challenger: promising,
            kind: PolicyKind::Solving,
            style: "tight",
            verdict: Verdict {
                deals: 1_600,
                mbb_per_100: 300.0,
                ci95: 100.0,
            },
        })
    );
    assert_eq!(fleet.champion_slot("A"), Ok(Some(champion)));
}

#[test]
fn a_confirmation_budget_that_expires_without_a_verdict_holds_the_incumbent() {
    let fleet = Fleet::in_dir(scratch("held"));
    let champion = Params { exploit: 0.4 };
    let promising = Params { exploit: 0.4 * 0.8 };
    fleet.set_champion("A", &champion).unwrap();
    let sim = |_challenger: &Policy, _champion: &Policy, step: &Step, deals: u64| {
        let mbb = match step.stage {
            Stage::First if step.challenger == promising => 200.0,
            Stage::First => -50.0,
            Stage::Confirmation => 50.0,
        };
        Verdict {
            deals: deals as usize,
            mbb_per_100: mbb,
            ci95: 100.0,
        }
    };
    for at in 0..6 {
        bot_step(&fleet, 200, 1_600, at, &sim).unwrap();
    }
    bot_step(&fleet, 200, 1_600, 6, &sim).unwrap();
    let note = bot_step(&fleet, 200, 1_600, 7, &sim).unwrap();
    assert!(note.contains("held"), "{note}");
    assert_eq!(fleet.champion_slot("A"), Ok(Some(champion)));
    assert_eq!(fleet.bot_search("A", &champion).unwrap().candidate, None);
}

#[test]
fn a_bot_with_a_candidate_waiting_is_not_searched_again() {
    let fleet = Fleet::in_dir(scratch("waiting"));
    let a = Params { exploit: 0.4 };
    let b = Params { exploit: 0.6 };
    fleet.set_champion("A", &a).unwrap();
    fleet.set_champion("B", &b).unwrap();
    let waiting = Candidate {
        challenger: Params { exploit: 0.32 },
        kind: PolicyKind::Solving,
        style: "standard",
        verdict: Verdict {
            deals: 1_600,
            mbb_per_100: 300.0,
            ci95: 100.0,
        },
    };
    fleet
        .record_bot_search("A", &a, &[], "candidate", 1, Some(&waiting))
        .unwrap();
    let calls = RefCell::new(Vec::new());
    let sim = |_challenger: &Policy, _champion: &Policy, _step: &Step, deals: u64| {
        calls.borrow_mut().push(deals);
        below(deals)
    };
    let note = bot_step(&fleet, 200, 1_600, 100, &sim).unwrap();
    assert!(note.starts_with("B "), "{note}");
    assert_eq!(calls.borrow().len(), 1);
}

#[test]
fn no_slot_means_no_search() {
    let fleet = Fleet::in_dir(scratch("no-slots"));
    let sim = |_: &Policy, _: &Policy, _: &Step, _: u64| -> Verdict {
        panic!("no sim may run without a champion slot")
    };
    let note = bot_step(&fleet, 200, 1_600, 100, &sim).unwrap();
    assert!(note.contains("no champion slot"), "{note}");
}

#[test]
fn the_cli_with_no_slots_still_searches_the_fleet_champion() {
    let dir = scratch("cli-fleet");
    let output = Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .env_clear()
        .env("SVANBOT_FLEET", &dir)
        .args(["learn", "--deals", "2"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("simmed"), "{stdout}");
    assert!(dir.join("search.json").exists());
    assert!(!dir.join("searches.json").exists());
}

#[test]
fn the_cli_searches_the_slotted_bot() {
    let dir = scratch("cli");
    let fleet = Fleet::in_dir(&dir);
    let champion = Params { exploit: 0.4 };
    fleet.set_champion("A", &champion).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_svanbot"))
        .env_clear()
        .env("SVANBOT_FLEET", &dir)
        .args(["learn", "--deals", "2"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("A screened"), "{stdout}");
    let search = fleet.bot_search("A", &champion).unwrap();
    assert_eq!(search.entries.len(), 1);
    assert_eq!(
        search
            .entries
            .iter()
            .map(|entry| match entry {
                svanbot_learn::search::Entry::Simmed { verdict, .. } => verdict.deals,
                _ => 0,
            })
            .sum::<usize>(),
        2
    );
}

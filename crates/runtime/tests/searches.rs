//! Each bot's search progress: what it screened and confirmed, and the candidate it produced,
//! keyed by the champion it was measured against.
// A test fleet is a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;

use svanbot_learn::paired::Verdict;
use svanbot_learn::search::{Entry, Stage, Step};
use svanbot_runtime::fleet::{BotSearch, Candidate, Fleet, SEARCH_KEPT};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::Params;

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn champion(exploit: f64) -> Params {
    Params { exploit }
}

fn verdict(mbb: f64) -> Verdict {
    Verdict {
        deals: 200,
        mbb_per_100: mbb,
        ci95: 100.0,
    }
}

fn entry(champion: &Params, challenger: Params) -> Entry {
    Entry::Simmed {
        step: Step {
            champion: champion.digest(),
            challenger,
            stage: Stage::First,
            seed: 7,
        },
        verdict: verdict(100.0),
    }
}

fn candidate(challenger: Params) -> Candidate {
    Candidate {
        challenger,
        kind: PolicyKind::Solving,
        style: "tight",
        verdict: verdict(300.0),
    }
}

#[test]
fn a_bot_s_search_is_keyed_by_the_champion_it_measured() {
    let fleet = Fleet::in_dir(scratch("champion"));
    let first = champion(0.4);
    let other = champion(0.6);
    let screened = entry(&first, champion(0.32));
    fleet
        .record_bot_search("A", &first, &[screened], "screened it", 10, None)
        .unwrap();
    assert_eq!(
        fleet.bot_search("A", &first).unwrap(),
        BotSearch {
            champion: first.digest(),
            entries: vec![screened],
            last: Some((10, "screened it".into())),
            candidate: None,
        }
    );
    // A new champion invalidates the old progress, and a new run starts its own log.
    assert_eq!(
        fleet.bot_search("A", &other).unwrap(),
        BotSearch::empty(other.digest())
    );
    let next = entry(&other, champion(0.48));
    fleet
        .record_bot_search("A", &other, &[next], "screened the new champion", 20, None)
        .unwrap();
    assert_eq!(fleet.bot_search("A", &other).unwrap().entries, vec![next]);
    // The old champion's log is gone: it was replaced, not merged.
    assert_eq!(
        fleet.bot_search("A", &first).unwrap(),
        BotSearch::empty(first.digest())
    );
}

#[test]
fn two_bots_keep_their_own_search_and_candidate() {
    let fleet = Fleet::in_dir(scratch("two-bots"));
    let a = champion(0.4);
    let b = champion(0.6);
    let a_entry = entry(&a, champion(0.32));
    let b_entry = entry(&b, champion(0.48));
    let a_candidate = candidate(champion(0.32));
    let b_candidate = candidate(champion(0.48));
    fleet
        .record_bot_search("A", &a, &[a_entry], "A screened", 10, Some(&a_candidate))
        .unwrap();
    fleet
        .record_bot_search("B", &b, &[b_entry], "B screened", 20, Some(&b_candidate))
        .unwrap();
    let read_a = fleet.bot_search("A", &a).unwrap();
    let read_b = fleet.bot_search("B", &b).unwrap();
    assert_eq!(read_a.entries, vec![a_entry]);
    assert_eq!(read_b.entries, vec![b_entry]);
    assert_eq!(read_a.candidate, Some(a_candidate));
    assert_eq!(read_b.candidate, Some(b_candidate));
    // Updating A moves nothing of B's.
    fleet
        .record_bot_search("A", &a, &[], "A waited", 30, None)
        .unwrap();
    assert_eq!(fleet.bot_search("B", &b).unwrap(), read_b);
}

#[test]
fn a_candidate_is_dropped_when_its_champion_changed_under_the_search() {
    let fleet = Fleet::in_dir(scratch("stale-candidate"));
    let measured = champion(0.4);
    let promoted = champion(0.35);
    let wanted = candidate(champion(0.32));
    fleet
        .record_bot_search(
            "A",
            &measured,
            &[entry(&measured, champion(0.32))],
            "candidate",
            10,
            Some(&wanted),
        )
        .unwrap();
    assert_eq!(
        fleet.bot_search("A", &measured).unwrap().candidate,
        Some(wanted.clone())
    );
    // A promotion lands: the old champion's log and candidate are not this one's, so nothing
    // carried over can be read as evidence for the new champion.
    let fresh = fleet.bot_search("A", &promoted).unwrap();
    assert_eq!(fresh.candidate, None);
    assert!(fresh.entries.is_empty());
    // The next run writes a fresh log under the new champion; the old candidate is replaced.
    fleet
        .record_bot_search(
            "A",
            &promoted,
            &[entry(&promoted, champion(0.28))],
            "screened the new champion",
            20,
            None,
        )
        .unwrap();
    assert_eq!(fleet.bot_search("A", &promoted).unwrap().candidate, None);
    assert_eq!(fleet.bot_search("A", &measured).unwrap().candidate, None);
}

#[test]
fn a_bot_s_log_keeps_only_its_newest_entries() {
    let fleet = Fleet::in_dir(scratch("kept"));
    let incumbent = champion(0.4);
    let screened = entry(&incumbent, champion(0.32));
    let entries = vec![screened; SEARCH_KEPT + 2];
    fleet
        .record_bot_search("A", &incumbent, &entries, "many", 10, None)
        .unwrap();
    assert_eq!(
        fleet.bot_search("A", &incumbent).unwrap().entries.len(),
        SEARCH_KEPT
    );
}

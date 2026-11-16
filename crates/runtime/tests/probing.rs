//! A bot's ledger kept in its store, and the canary's probe from proposal to promotion.
// A test store and fleet are files in a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::path::PathBuf;

use svanbot_learn::probe::Kind;
use svanbot_runtime::fleet::{Fleet, Proposal};
use svanbot_runtime::ledger::ORBIT;
use svanbot_runtime::probing::Probing;
use svanbot_runtime::record::Hand;
use svanbot_runtime::store::Store;
use svanbot_strategy::params::Params;

const CHALLENGER: Params = Params {
    raise_share: 0.9,
    ..Params::DEFAULT
};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-probing-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn hand(number: u32, stack: i64) -> Hand {
    Hand {
        hand_id: format!("h-{number}"),
        table_id: "t1".to_owned(),
        frames: vec![
            format!(
                r#"{{"type":"hand_start","hand_id":"h-{number}","seat":2,"dealer_seat":0,"blinds":{{"small_blind":10,"big_blind":20}}}}"#
            ),
            format!(
                r#"{{"type":"hand_result","winners":[],"final_stacks":{{"0":1960,"2":{stack}}}}}"#
            ),
        ],
    }
}

fn guard(hands: u64) -> Proposal {
    Proposal {
        challenger: CHALLENGER,
        kind: Kind::Guard { hands },
        harm_margin: 5.0,
        sim_estimate: Some(2.0),
        max_orbits: 10_000,
    }
}

/// Plays `hands` hands numbered from `from`, in which a hand played by the challenger wins
/// `challenger` chips and any other breaks even. Returns the next hand's number.
fn play(probing: &mut Probing<'_>, from: u32, hands: u32, challenger: i64) -> u32 {
    let mut stack = 1_000_000 + i64::from(from);
    for number in from..from + hands {
        if probing.params() == CHALLENGER {
            stack += challenger + i64::from(number % 3) * 20 - 20;
        }
        probing.on_hand(&hand(number, stack), 1_000);
    }
    from + hands
}

#[test]
fn every_bot_keeps_what_its_hands_won() {
    let dir = scratch("keeps");
    let store = Store::open(&dir.join("MegaSvan.db"), "MegaSvan").unwrap();
    let mut probing = Probing::open(&store, Fleet::in_dir(dir.join("fleet")), false, 500).0;
    probing.on_hand(&hand(1, 2000), 1_000);
    probing.on_hand(&hand(2, 2060), 1_000);
    let played = store.played().unwrap();
    assert_eq!(played.len(), 1);
    assert_eq!((played[0].net, played[0].orbit), (60, None));
}

#[test]
fn only_the_canary_takes_up_a_proposal() {
    let dir = scratch("only-canary");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    fleet.propose(&guard(120)).unwrap();
    let store = Store::open(&dir.join("MegaSvan.db"), "MegaSvan").unwrap();
    let mut probing = Probing::open(&store, fleet.clone(), false, 500).0;
    play(&mut probing, 0, 30, 0);
    assert_eq!(store.open_probe().unwrap(), None);
    assert!(fleet.take_proposal().unwrap().is_some());
}

#[test]
fn a_guard_that_clears_promotes_the_challenger_and_the_fleet_adopts_it() {
    let dir = scratch("clears");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    let mut canary = Probing::open(&store, fleet.clone(), true, 500).0;
    let other_store = Store::open(&dir.join("MegaSvan.db"), "MegaSvan").unwrap();
    let mut other = Probing::open(&other_store, fleet.clone(), false, 500).0;

    fleet.propose(&guard(120)).unwrap();
    // The first hand is not counted, and the canary takes the proposal up after it.
    let next = play(&mut canary, 0, 1, 0);
    assert!(store.open_probe().unwrap().is_some());
    // Mid-guard, the rest of the fleet still plays the champion.
    let next = play(&mut canary, next, 60, 0);
    play(&mut other, 0, 2, 0);
    assert_eq!(other.params(), Params::DEFAULT);
    assert_eq!(fleet.champion(), Ok(Params::DEFAULT));

    let notes = (next..next + 80)
        .flat_map(|number| canary.on_hand(&hand(number, 1_000_000), 2_000))
        .collect::<Vec<_>>();
    assert!(
        notes.iter().any(|note| note.contains("cleared")),
        "{notes:?}"
    );
    assert_eq!(store.open_probe().unwrap(), None);
    assert_eq!(fleet.champion(), Ok(CHALLENGER));
    assert_eq!(canary.params(), CHALLENGER);
    // The others adopt between hands.
    play(&mut other, 2, 1, 0);
    assert_eq!(other.params(), CHALLENGER);
}

#[test]
fn a_guard_that_finds_harm_rolls_back_and_the_fleet_never_sees_the_challenger() {
    let dir = scratch("harm");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    let mut canary = Probing::open(&store, fleet.clone(), true, 500).0;
    fleet.propose(&guard(30_000)).unwrap();
    // The challenger loses two big blinds a hand.
    play(&mut canary, 0, 1 + 400 * ORBIT, -40);
    assert_eq!(store.open_probe().unwrap(), None);
    assert_eq!(fleet.champion(), Ok(Params::DEFAULT));
    assert_eq!(canary.params(), Params::DEFAULT);
}

#[test]
fn a_restarted_canary_takes_its_probe_up_where_the_store_left_it() {
    let dir = scratch("restart");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    fleet.propose(&guard(120)).unwrap();
    let mut canary = Probing::open(&store, fleet.clone(), true, 500).0;
    // 15 full orbits of the guard's 20.
    let next = play(&mut canary, 0, 1 + 15 * ORBIT, 0);
    drop(canary);

    let (mut canary, notes) = Probing::open(&store, fleet.clone(), true, 500);
    assert!(
        notes.iter().any(|note| note.contains("15 orbits")),
        "{notes:?}"
    );
    // One uncounted hand at the table again, then the five orbits left.
    play(&mut canary, next, 1 + 5 * ORBIT, 0);
    assert_eq!(fleet.champion(), Ok(CHALLENGER));
}

#[test]
fn a_promotion_taken_back_by_hand_reaches_the_canary_too() {
    let dir = scratch("taken-back");
    let fleet = Fleet::in_dir(dir.join("fleet"));
    fleet.promote(&CHALLENGER).unwrap();
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    let mut canary = Probing::open(&store, fleet.clone(), true, 500).0;
    assert_eq!(canary.params(), CHALLENGER);
    std::fs::remove_file(dir.join("fleet").join("champion.json")).unwrap();
    play(&mut canary, 0, 1, 0);
    assert_eq!(canary.params(), Params::DEFAULT);
}

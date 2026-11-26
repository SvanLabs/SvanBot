//! The ledger: what each hand won, and the canary's probe on top of it.

use svanbot_learn::probe::{Arm, Decided, Hypothesis, Kind, Verdict};
use svanbot_runtime::ledger::{Ledger, ORBIT};
use svanbot_runtime::record::Hand;
use svanbot_strategy::params::Params;

const CHALLENGER: Params = Params {
    exploit: 0.9,
    spread: 0.0,
    ..Params::DEFAULT
};

/// A hand at `table` that leaves our seat, 2, with `stack` chips, in the venue's own frames.
fn hand(table: &str, number: u32, stack: u64) -> Hand {
    Hand {
        hand_id: format!("h-{table}-{number}"),
        table_id: table.to_owned(),
        frames: vec![
            format!(
                r#"{{"type":"hand_start","hand_id":"h-{table}-{number}","seat":2,"dealer_seat":0,"blinds":{{"small_blind":10,"big_blind":20}}}}"#
            ),
            format!(
                r#"{{"type":"hand_result","winners":[],"final_stacks":{{"0":1960,"2":{stack}}}}}"#
            ),
        ],
        traced: Vec::new(),
    }
}

fn hypothesis(kind: Kind) -> Hypothesis {
    Hypothesis {
        champion: Params::DEFAULT.digest(),
        challenger: CHALLENGER.digest(),
        kind,
        harm_margin: 5.0,
        sim_estimate: None,
        max_orbits: 10_000,
    }
}

#[test]
fn a_hand_wins_what_the_stack_gained_since_the_hand_before() {
    let mut ledger = Ledger::new(Params::DEFAULT);
    // The first hand at a table has no hand before it to measure from.
    assert_eq!(ledger.on_hand(&hand("t1", 1, 2000)), None);
    let played = ledger.on_hand(&hand("t1", 2, 2060)).unwrap();
    assert_eq!((played.net, played.big_blind), (60, 20));
    assert_eq!(played.digest, Params::DEFAULT.digest());
    assert_eq!(played.orbit, None);
    let played = ledger.on_hand(&hand("t1", 3, 1900)).unwrap();
    assert_eq!(played.net, -160);
    // A new table starts over: its first hand is measured from nothing.
    assert_eq!(ledger.on_hand(&hand("t2", 1, 2000)), None);
}

#[test]
fn the_luck_of_our_own_draws_is_set_beside_the_result() {
    let mut ledger = Ledger::new(Params::DEFAULT);
    ledger.on_hand(&hand("t1", 1, 2000));
    // A coin flip between an action worth 100 chips and one worth 0, and the 100 came up: 50
    // chips of the hand were luck.
    ledger.decided(Decided {
        strategy: vec![0.5, 0.5],
        values: vec![100.0, 0.0],
        taken: 0,
    });
    let played = ledger.on_hand(&hand("t1", 2, 2100)).unwrap();
    assert_eq!(played.net, 100);
    assert!((played.luck - 50.0).abs() < 1e-9);
    // The draws of one hand do not carry into the next.
    let played = ledger.on_hand(&hand("t1", 3, 2100)).unwrap();
    assert!(played.luck.abs() < 1e-9);
}

#[test]
fn a_probe_holds_one_arm_for_an_orbit_and_plays_by_its_parameter_set() {
    let mut ledger = Ledger::new(Params::DEFAULT);
    ledger.start(hypothesis(Kind::Probe), CHALLENGER, 0);
    ledger.on_hand(&hand("t1", 0, 2000));
    let mut arms = Vec::new();
    for orbit in 0..40 {
        let by = *ledger.params();
        let mut seen = Vec::new();
        for n in 0..ORBIT {
            assert_eq!(*ledger.params(), by, "the arm holds for the orbit");
            let played = ledger.on_hand(&hand("t1", 1 + orbit * ORBIT + n, 2000));
            seen.push(played.unwrap().orbit.unwrap());
        }
        let (_, number, arm) = seen[0];
        assert!(seen.iter().all(|&at| at == seen[0]));
        assert_eq!(number, u64::from(orbit));
        assert_eq!(
            by,
            match arm {
                Arm::Champion => Params::DEFAULT,
                Arm::Challenger => CHALLENGER,
            }
        );
        arms.push(arm);
    }
    let challenger = arms.iter().filter(|&&arm| arm == Arm::Challenger).count();
    assert!((10..=30).contains(&challenger), "a fair coin: {challenger}");
    // 240 hands that all broke even tell the arms apart in no way.
    assert_eq!(ledger.verdict(), Some(Verdict::Running));
}

/// Plays orbits in which the champion's hands win `champion` chips each and the challenger's win
/// `challenger`, until a verdict or `orbits` run out.
fn play(ledger: &mut Ledger, champion: i64, challenger: i64, orbits: u32) -> Option<Verdict> {
    let mut stack: i64 = 1_000_000;
    ledger.on_hand(&hand("t1", 0, stack as u64));
    for n in 0..orbits * ORBIT {
        // A little noise, so the spread is not zero.
        let noise = i64::from(n % 3) * 20 - 20;
        stack += noise
            + if *ledger.params() == CHALLENGER {
                challenger
            } else {
                champion
            };
        ledger.on_hand(&hand("t1", n + 1, stack as u64));
        if ledger.verdict() != Some(Verdict::Running) {
            break;
        }
    }
    ledger.verdict()
}

#[test]
fn a_challenger_that_loses_ends_the_probe_as_harmful() {
    let mut ledger = Ledger::new(Params::DEFAULT);
    ledger.start(hypothesis(Kind::Probe), CHALLENGER, 0);
    assert_eq!(play(&mut ledger, 20, -20, 400), Some(Verdict::Harmful));
    // The champion stays.
    ledger.finish(false);
    assert_eq!(*ledger.params(), Params::DEFAULT);
    assert_eq!(ledger.verdict(), None);
}

#[test]
fn a_guard_clears_after_its_hands_and_the_challenger_becomes_the_champion() {
    let mut ledger = Ledger::new(Params::DEFAULT);
    ledger.start(hypothesis(Kind::Guard { hands: 120 }), CHALLENGER, 0);
    assert_eq!(play(&mut ledger, 0, 0, 400), Some(Verdict::Cleared));
    ledger.finish(true);
    assert_eq!(*ledger.params(), CHALLENGER);
}

#[test]
fn a_restart_takes_up_the_probe_from_its_stored_orbits() {
    let mut ledger = Ledger::new(Params::DEFAULT);
    ledger.start(hypothesis(Kind::Guard { hands: 120 }), CHALLENGER, 19);
    for _ in 0..19 {
        ledger.replay(Arm::Champion, 0.0, ORBIT);
    }
    // 114 of the guard's 120 hands are in: one more orbit clears it, numbered after the rest.
    ledger.on_hand(&hand("t1", 0, 2000));
    for n in 0..ORBIT {
        assert_eq!(ledger.verdict(), Some(Verdict::Running));
        let played = ledger.on_hand(&hand("t1", n + 1, 2000)).unwrap();
        assert_eq!(played.orbit.unwrap().1, 19);
    }
    assert_eq!(ledger.verdict(), Some(Verdict::Cleared));
}

#[test]
fn a_hand_with_no_result_for_our_seat_is_not_counted_and_nor_is_the_next() {
    let mut ledger = Ledger::new(Params::DEFAULT);
    ledger.on_hand(&hand("t1", 1, 2000));
    let mut cut = hand("t1", 2, 2000);
    cut.frames.pop();
    assert_eq!(ledger.on_hand(&cut), None);
    assert_eq!(ledger.on_hand(&hand("t1", 3, 2500)), None);
    assert_eq!(ledger.on_hand(&hand("t1", 4, 2520)).unwrap().net, 20);
}

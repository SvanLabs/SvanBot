//! The rebuilt preflop of a snapshot-only resync when the street held a caller (#595).
//!
//! `situation()` rebuilds the preflop betting from the bets on the table when no aggressive record
//! survived the resync, and its candidate set is every in-hand seat with more than the big blind in
//! front of it — which a caller of a raise also has. Reading those as raisers lost the caller's chips
//! from the pot and its Call from the history, and left the raise attributed to whichever of two tied
//! seats the `HashMap` iterated first.
//!
//! The caller-free geometry is pinned next door in
//! `resync_rebuilt_raises_have_consistent_pot_geometry`; these are the cases that script could not
//! reach, in a file of their own so that neither outgrows the 500-line limit.

use super::super::*;
use serde_json::json;

fn tracker() -> TableTracker {
    let mut t = TableTracker::default();
    t.reset_table();
    t.bb = 20;
    t.hero_seat = Some(0);
    t.hole = Some([Card::parse("As").unwrap(), Card::parse("Kd").unwrap()]);
    t.dealer = 0;
    t
}

fn seat(seat: usize, bet: i64) -> SeatView {
    SeatView { seat, name: format!("p{seat}"), stack: 2_000, bet, in_hand: true, ..Default::default() }
}

fn facing() -> LegalActions {
    LegalActions { can_fold: true, can_check: false, call: Some(180), raise_min: Some(300), raise_max: Some(2_000), all_in: Some(2_000) }
}

fn record(r: &ActionRecord) -> (usize, ActionKind, i64, i64, i64) {
    (r.seat, r.kind, r.to, r.pot_before, r.to_call_before)
}

/// p3 opens to 60, p5 calls the 60, p4 3-bets to 180, hero on the button facing 180: the rebuilt
/// history has to be the one the event stream would have produced, field for field.
#[test]
fn a_caller_of_a_raise_is_rebuilt_as_a_call_and_keeps_its_chips_in_the_pot() {
    let mut t = tracker();
    for (s, b) in [(0, 0), (1, 10), (2, 20), (3, 60), (4, 180), (5, 60)] {
        t.seats.insert(s, seat(s, b));
    }
    let sit = t.situation(&json!({"type": "your_turn", "seat": 0, "pot": 330}), &facing()).unwrap();
    // The open's geometry, then the 3-bettor's pot_before with the caller's 60 in it: without it the
    // price the policy reads is 180/90, which crosses the overbet threshold a 180/150 raise does not.
    assert_eq!(record(&sit.history[0]), (3, ActionKind::Raise, 60, 30, 20));
    assert_eq!(record(&sit.history[1]), (5, ActionKind::Call, 60, 90, 60), "the caller's own action");
    assert_eq!(record(&sit.history[2]), (4, ActionKind::Raise, 180, 150, 60));
    // Every chip on the table is counted once, and the raise is not attributed to a seat that called.
    assert_eq!(sit.history[2].pot_before + 180, sit.pot);
    assert_eq!(sit.history.len(), 3, "no folds to reconstruct here: {:?}", sit.history);
}

/// A blind calling the top level: the seat that raised to it is the raiser even though the blind comes
/// earlier in postflop order, and the blind's call is recorded after it, owing what it had not posted.
#[test]
fn a_blind_calling_the_top_level_is_not_mistaken_for_the_raiser() {
    let mut t = tracker();
    // SB 10, BB 20, p3 opens 60, p4 calls 60, p5 3-bets 180, the small blind calls 180.
    for (s, b) in [(0, 0), (1, 180), (2, 20), (3, 60), (4, 60), (5, 180)] {
        t.seats.insert(s, seat(s, b));
    }
    let sit = t.situation(&json!({"type": "your_turn", "seat": 0, "pot": 500}), &facing()).unwrap();
    let actions: Vec<(usize, ActionKind)> = sit.history.iter().map(|r| (r.seat, r.kind)).collect();
    assert_eq!(
        actions,
        vec![(3, ActionKind::Raise), (4, ActionKind::Call), (5, ActionKind::Raise), (1, ActionKind::Call),],
        "the small blind raised nothing: {:?}",
        sit.history
    );
    // The blind owed 170 of its 180; the last action leaves the pot holding every chip posted.
    assert_eq!(record(&sit.history[3]), (1, ActionKind::Call, 180, 330, 170));
    assert_eq!(sit.history[3].pot_before + (180 - 10), sit.pot);
}

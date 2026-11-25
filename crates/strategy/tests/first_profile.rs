//! The first real profile (V11-43, slice 5), tested at the profile's seam: a spot and what the bot
//! has read of the opponent in, an action out, or none, so the re-solve or the pricer decides.

use svanbot_core::rules::{Action, Legal, RaiseRange};
use svanbot_core::spot::Spot;
use svanbot_strategy::first_profile::RULES;
use svanbot_strategy::profile::{OpponentStats, Profile, Rule, SeatedStats};

/// A heads-up spot against the opponent in seat 1, with a pot of `pot`, `call` to pay and a stack
/// of 100 big blinds (the big blind is 20).
fn spot(hole: &str, board: &str, pot: u64, call: Option<u64>) -> Spot {
    Spot {
        hole: hole.parse().unwrap(),
        board: board.parse().unwrap(),
        pot,
        legal: Legal {
            call,
            raise: Some(RaiseRange {
                min: 2 * call.unwrap_or(20),
                max: 2000,
            }),
        },
        opponents: 1,
        stack: 2000,
        callable_pot: None,
        committed: 0,
        big_blind: 20,
        in_position: false,
        players_behind: None,
        lines: vec![vec![]],
        seats: vec![1],
    }
}

/// What a bot has read of an opponent: `hands` hands each, with preflop entry at `vpip`, preflop
/// raising at `pfr` and folds to a postflop bet at `fold`, as shares of those hands.
fn read(hands: usize, vpip: f64, pfr: f64, fold: f64) -> OpponentStats {
    let mut stats = OpponentStats::default();
    let n = |share: f64| (share * hands as f64).round() as usize;
    for hand in 0..hands {
        stats.vpip.observe(usize::from(hand < n(vpip)), 1);
        stats.pfr.observe(usize::from(hand < n(pfr)), 1);
        stats.fold_to_bet.observe(usize::from(hand < n(fold)), 1);
    }
    stats
}

#[test]
fn a_rule_can_read_the_hands_equity_against_any_two_cards() {
    let rules = [Rule {
        when: |facts| facts.equity() > 0.8,
        then: |_| Action::RaiseTo(100),
    }];
    let profile = Profile::new(&rules);
    // Aces preflop and a royal flush on the river are both far ahead of a random hand.
    assert_eq!(
        profile.action(&spot("AhAs", "", 30, Some(20)), 1),
        Some(Action::RaiseTo(100))
    );
    assert_eq!(
        profile.action(&spot("AhKh", "QhJhTh2c3d", 400, Some(200)), 1),
        Some(Action::RaiseTo(400))
    );
    // Seven-deuce offsuit is not.
    assert_eq!(profile.action(&spot("7h2c", "", 30, Some(20)), 1), None);
}

#[test]
fn a_rule_can_read_the_one_opponent_left_in_the_hand() {
    let rules = [Rule {
        when: |facts| {
            facts
                .sole_opponent()
                .is_some_and(|read| read.vpip.value() == Some(1.0))
        },
        then: |_| Action::Call,
    }];
    let seen = [SeatedStats {
        seat: 1,
        stats: read(40, 1.0, 0.0, 0.0),
    }];
    let profile = Profile::new(&rules).with_opponents(&seen);
    let heads_up = spot("AhAs", "", 30, Some(20));
    assert_eq!(profile.action(&heads_up, 1), Some(Action::Call));

    // A sim's spot that names no seats still has the one read it was given.
    let mut unnamed = spot("AhAs", "", 30, Some(20));
    unnamed.seats = vec![];
    assert_eq!(profile.action(&unnamed, 1), Some(Action::Call));

    // The read is of another seat, or two opponents are still in: nothing here is theirs to use.
    let mut elsewhere = spot("AhAs", "", 30, Some(20));
    elsewhere.seats = vec![2];
    assert_eq!(profile.action(&elsewhere, 1), None);
    let mut crowded = spot("AhAs", "", 30, Some(20));
    crowded.opponents = 2;
    crowded.seats = vec![1, 2];
    assert_eq!(profile.action(&crowded, 1), None);
}

/// The first profile against the opponent in seat 1, whom the bot has read as `stats`.
fn first(stats: OpponentStats, at: &Spot) -> Option<Action> {
    let seen = [SeatedStats { seat: 1, stats }];
    Profile::new(&RULES).with_opponents(&seen).action(at, 1)
}

/// A player who enters every pot and never raises or folds to a bet, over 40 hands.
fn station() -> OpponentStats {
    read(40, 1.0, 0.0, 0.0)
}

#[test]
fn against_a_calling_station_a_strong_hand_bets_the_pot() {
    // A set of aces on a dry flop, checked to us.
    let flop = spot("AhAs", "Ad7c2s", 100, None);
    assert_eq!(first(station(), &flop), Some(Action::RaiseTo(100)));
    // Before the flop the same logic holds: the station calls anything, so aces raise the pot.
    let preflop = spot("AhAs", "", 30, Some(10));
    assert_eq!(first(station(), &preflop), Some(Action::RaiseTo(40)));
}

#[test]
fn against_a_calling_station_a_weak_hand_never_bluffs() {
    // Nothing on a board of broadway cards, checked to us: a bluff never folds it out.
    let flop = spot("7h2c", "AsKdQc", 100, None);
    assert_eq!(first(station(), &flop), Some(Action::Check));
}

#[test]
fn a_calling_stations_rare_bet_is_respected_unless_we_hold_a_monster() {
    // A pair of queens facing its bet on a dry flop: it bets only when it is strong.
    let weak = spot("QhJd", "Ks7d2c", 150, Some(50));
    assert_eq!(first(station(), &weak), Some(Action::Fold));
    // A set of aces is still raised.
    let monster = spot("AhAs", "Ad7c2s", 150, Some(50));
    assert_eq!(first(station(), &monster), Some(Action::RaiseTo(200)));
}

#[test]
fn a_read_that_is_thin_or_is_not_a_station_leaves_the_spot_to_the_re_solve() {
    let flop = spot("AhAs", "Ad7c2s", 100, None);
    // Ten hands are below the evidence floor.
    assert_eq!(first(read(10, 1.0, 0.0, 0.0), &flop), None);
    // A player who raises a third of the pots he enters is no station.
    assert_eq!(first(read(40, 1.0, 0.35, 0.0), &flop), None);
    // And one who folds to bets is not one to bet big for value against.
    assert_eq!(first(read(40, 1.0, 0.0, 0.6), &flop), None);
    // No read at all: the profile matches nothing.
    assert_eq!(Profile::new(&RULES).action(&flop, 1), None);
}

/// A player who enters every pot and raises two in three, over 40 hands.
fn maniac() -> OpponentStats {
    read(40, 1.0, 0.65, 0.05)
}

#[test]
fn against_a_maniac_a_strong_hand_raises_its_bet() {
    // A set of aces facing a pot-sized bet on a dry flop.
    let flop = spot("AhAs", "Ad7c2s", 200, Some(100));
    assert_eq!(first(maniac(), &flop), Some(Action::RaiseTo(300)));
}

#[test]
fn against_a_maniac_a_hand_with_the_price_calls_and_one_without_folds() {
    // It bets with any two cards, so a pot-sized bet needs a third of the pot in equity: a pair of
    // sevens under a king has it, and four-three offsuit under three broadway cards does not.
    let priced = spot("7h6h", "Ks7d2c", 200, Some(100));
    assert_eq!(first(maniac(), &priced), Some(Action::Call));
    let short = spot("4h3c", "KsQdJc", 200, Some(100));
    assert_eq!(first(maniac(), &short), Some(Action::Fold));
}

#[test]
fn against_a_maniac_with_nothing_owed_only_a_monster_bets() {
    // Checked to, a middling hand lets it bluff into us; a set of aces bets the pot.
    let middling = spot("Th9h", "Ks7d2c", 100, None);
    assert_eq!(first(maniac(), &middling), Some(Action::Check));
    let monster = spot("AhAs", "Ad7c2s", 100, None);
    assert_eq!(first(maniac(), &monster), Some(Action::RaiseTo(100)));
}

#[test]
fn a_player_who_is_neither_a_maniac_nor_a_station_is_left_to_the_re_solve() {
    // Tight and calm: he enters one pot in two and raises one in four.
    let flop = spot("AhAs", "Ad7c2s", 200, Some(100));
    assert_eq!(first(read(40, 0.5, 0.25, 0.3), &flop), None);
}

//! The first real profile (V11-43, slice 5): rules for the opponents a bot has read as extreme.
//!
//! A rule fires only on a read that has cleared the evidence floor ([`MIN_EVIDENCE_HANDS`]), and
//! only with one opponent left in the hand. Every other spot, and every opponent not yet read, is
//! decided by the re-solve or the pricer as before.
//!
//! The calling station is the first one. It enters nearly every pot, almost never raises, and
//! does not fold to a bet. Against it a bluff has nobody to fold out, and every hand ahead of a
//! random one earns from a bet, so the profile bets those for the pot, never bluffs, and folds to
//! the station's rare bet unless it holds a monster.
//!
//! The maniac is the second. It enters nearly every pot and raises most of them, with any two
//! cards, so its betting range is every hand and a hand's equity against any two cards is its
//! equity against the bet. The profile calls when that equity beats the pot odds, re-raises with
//! a hand far ahead, and otherwise lets it bluff into a check.
//!
//! [`MIN_EVIDENCE_HANDS`]: crate::profile::MIN_EVIDENCE_HANDS

use svanbot_core::rules::{Action, Chips, Street};

use crate::profile::{Facts, OpponentStats, Rule};

/// The equity, against any two cards, from which a hand is a value bet against a station: it calls
/// with every hand, so a hand ahead of a random one wins from every chip it bets.
const VALUE: f64 = 0.58;

/// A station's bet is rare and strong. A hand below this equity folds to it.
const RESPECT: f64 = 0.85;

/// A hand this far ahead of any two cards re-raises a maniac's bet, and bets into a check.
const RERAISE: f64 = 0.75;

/// What a call against a maniac needs above the pot odds, for the streets still to come.
const CUSHION: f64 = 0.04;

/// A player who enters almost every pot, almost never raises it, and does not fold to a bet.
fn is_station(read: OpponentStats) -> bool {
    read.vpip.value().is_some_and(|rate| rate >= 0.85)
        && read.pfr.value().is_some_and(|rate| rate <= 0.15)
        && read.fold_to_bet.value().is_none_or(|rate| rate <= 0.2)
}

/// Whether the one opponent left in the hand has been read as a station.
fn against_station(facts: &Facts<'_>) -> bool {
    facts.sole_opponent().is_some_and(is_station)
}

/// A player who enters almost every pot and raises most of the ones he enters.
fn is_maniac(read: OpponentStats) -> bool {
    read.vpip.value().is_some_and(|rate| rate >= 0.8)
        && read.pfr.value().is_some_and(|rate| rate >= 0.5)
}

/// Whether the one opponent left in the hand has been read as a maniac.
fn against_maniac(facts: &Facts<'_>) -> bool {
    facts.sole_opponent().is_some_and(is_maniac)
}

/// The share of the pot a call costs once it is in: the equity a call has to beat.
fn price(facts: &Facts<'_>) -> Option<f64> {
    let call = facts.legal().call?;
    Some(call as f64 / (facts.pot() + call) as f64)
}

/// A bet or raise adding `share` of the pot to what is owed.
fn pot_bet(facts: &Facts<'_>, share: f64) -> Action {
    let owed = facts.legal().call.unwrap_or(0);
    Action::RaiseTo(facts.committed() + owed + (facts.pot() as f64 * share) as Chips)
}

/// The first profile's rules, in order: the first to match decides.
pub const RULES: [Rule; 7] = [
    // Its bet is strong. Before the flop an owed call is only the blind, not a bet.
    Rule {
        when: |facts| {
            against_station(facts)
                && facts.street() != Street::Preflop
                && facts.legal().call.is_some()
                && facts.equity() < RESPECT
        },
        then: |_| Action::Fold,
    },
    // It calls with everything, so a hand ahead of a random one is worth a pot-sized bet.
    Rule {
        when: |facts| against_station(facts) && facts.equity() >= VALUE,
        then: |facts| pot_bet(facts, 1.0),
    },
    // It never folds, so a bluff has nobody to fold out: check what is not worth a bet.
    Rule {
        when: |facts| against_station(facts) && facts.legal().call.is_none(),
        then: |_| Action::Check,
    },
    // A maniac bets with any two cards, so a hand far ahead of one re-raises its bet or bets into
    // its check.
    Rule {
        when: |facts| against_maniac(facts) && facts.equity() >= RERAISE,
        then: |facts| pot_bet(facts, 1.0),
    },
    // Facing its bet, a call that beats the pot odds is worth it, and one that does not folds.
    Rule {
        when: |facts| {
            against_maniac(facts)
                && price(facts).is_some_and(|price| facts.equity() >= price + CUSHION)
        },
        then: |_| Action::Call,
    },
    Rule {
        when: |facts| against_maniac(facts) && facts.legal().call.is_some(),
        then: |_| Action::Fold,
    },
    // Checked to with a middling hand: let it bluff into us.
    Rule {
        when: |facts| against_maniac(facts),
        then: |_| Action::Check,
    },
];

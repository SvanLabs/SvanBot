//! The playbook (`GLOSSARY.md`): a set of compiled rules per opponent type, read from the stats
//! of the one opponent left in the hand and gated by that opponent's adaptivity class.
//!
//! A type is read only once its stats clear the evidence floor: [`Rate::value`] answers `None`
//! under [`crate::profile::MIN_EVIDENCE_HANDS`] hands, so an unread opponent falls through to the re-solve or
//! the pricer as if no playbook existed. The cuts are parameter-set numbers, so the learner moves
//! them inside the style's bounds; the rules themselves take no type condition, because the rule
//! set chosen is the type.

use svanbot_core::rules::{Action, Street};
use svanbot_core::spot::Spot;

use crate::first_profile::{MANIAC_RULES, STATION_RULES, pot_bet};
use crate::params::Params;
use crate::profile::{OpponentStats, Profile, Rate, Rule, SeatedStats};

/// A player who does not fold to a bet: the most they fold, as a share of the bets they face.
const STATION_FOLD: f64 = 0.2;

/// The share of the pot a bet asks for into a nit, which folds everything but strong hands.
const NIT_BLUFF: f64 = 0.6;

/// The equity from which a hand re-raises a nit's bet, or bets into its check.
const NIT_RESPECT: f64 = 0.85;

/// The three opponent types the playbook has a read for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Type {
    /// Enters most pots, rarely raises, and does not fold to a bet.
    Station,
    /// Enters most pots and raises most of those.
    Maniac,
    /// Enters few pots, folds all but strong hands to a bet, and raises those.
    Nit,
}

/// The cuts that read an opponent's type from their stats, as the parameter set carries them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cuts {
    pub station_vpip: f64,
    pub station_pfr: f64,
    pub maniac_vpip: f64,
    pub maniac_pfr: f64,
    pub nit_vpip: f64,
    pub nit_fold: f64,
}

impl Cuts {
    /// The cuts of `params`.
    pub fn of(params: &Params) -> Self {
        Self {
            station_vpip: params.station_vpip,
            station_pfr: params.station_pfr,
            maniac_vpip: params.maniac_vpip,
            maniac_pfr: params.maniac_pfr,
            nit_vpip: params.nit_vpip,
            nit_fold: params.nit_fold,
        }
    }
}

/// The type `stats` read as, or `None` for a regular or an opponent short of
/// [`crate::profile::MIN_EVIDENCE_HANDS`] hands at a stat. The maniac is read first, so a raiser whose stats sit
/// between loosened cuts is still a maniac; the nit needs evidence of folding to a bet, not only
/// of entering few pots.
pub fn type_of(stats: OpponentStats, cuts: Cuts) -> Option<Type> {
    let at_least = |rate: Rate, cut: f64| rate.value().is_some_and(|value| value >= cut);
    let at_most = |rate: Rate, cut: f64| rate.value().is_some_and(|value| value <= cut);
    if at_least(stats.vpip, cuts.maniac_vpip) && at_least(stats.pfr, cuts.maniac_pfr) {
        Some(Type::Maniac)
    } else if at_least(stats.vpip, cuts.station_vpip)
        && at_most(stats.pfr, cuts.station_pfr)
        && at_most(stats.fold_to_bet, STATION_FOLD)
    {
        Some(Type::Station)
    } else if at_most(stats.vpip, cuts.nit_vpip) && at_least(stats.fold_to_bet, cuts.nit_fold) {
        Some(Type::Nit)
    } else {
        None
    }
}

/// The rules of `kind`'s playbook, in order: the first to match decides.
pub fn rules(kind: Type) -> &'static [Rule] {
    match kind {
        Type::Station => &STATION_RULES,
        Type::Maniac => &MANIAC_RULES,
        Type::Nit => &NIT_RULES,
    }
}

/// The playbook's action at `spot` when one of `kind`'s rules matches, with each opponent's read
/// as the profile sees it. The caller has already read the opponent's type and class.
pub fn action(kind: Type, spot: &Spot, seed: u64, seated: &[SeatedStats]) -> Option<Action> {
    Profile::new(rules(kind))
        .with_opponents(seated)
        .action(spot, seed)
}

/// The nit's rules, in order: it folds all but strong hands to a bet, and raises those.
pub const NIT_RULES: [Rule; 3] = [
    // Its betting range is strong. Before the flop an owed call is only the blind, not a bet.
    Rule {
        when: |facts| {
            facts.legal().call.is_some()
                && facts.street() != Street::Preflop
                && facts.equity() < NIT_RESPECT
        },
        then: |_| Action::Fold,
    },
    // A hand far ahead of any two cards re-raises its bet, or bets into its check.
    Rule {
        when: |facts| facts.equity() >= NIT_RESPECT,
        then: |facts| pot_bet(facts, 1.0),
    },
    // It folds everything but strong hands to a bet: ask for 60% of the pot when checked to.
    Rule {
        when: |facts| facts.legal().call.is_none(),
        then: |facts| pot_bet(facts, NIT_BLUFF),
    },
];

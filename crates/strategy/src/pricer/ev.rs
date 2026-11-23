//! What each action is worth, in chips from here on, over the pricer's sampled deals (V11-39).
//!
//! Folding is worth nothing. A check takes our share of the pot, and a call our share of the pot
//! after the call less the call. A raise is answered by every opponent left in the hand at once:
//! each folds or calls on the cards they were dealt, folding as often as their action model
//! folds that bucket to a bet, and more often the bigger the bet ([`continues`]). Whoever folds
//! leaves the pot to the rest; whoever calls adds the raise and goes to showdown. Re-raises are
//! not answered here: a raise is priced as if the table only folds or calls.
//!
//! The answers are summed over every way the table can split into folders and callers, so the
//! value is exact for its deals rather than drawn again.

use std::collections::BTreeMap;

use svanbot_core::card::CardSet;
use svanbot_core::rules::{Chips, Street};
use svanbot_core::spot::Spot;
use svanbot_sim::equity::Showdown;

use crate::action::{ActionModel, Move, Situation};

/// The bet, as a share of the pot after the call, at which a bucket continues exactly as its
/// model says. The model counts every size of bet together, and two thirds of the pot is the
/// size it most often met.
const REFERENCE_SHARE: f64 = 0.66;

/// How fast a bigger bet makes a player fold more: their continue chance is raised to the power
/// `(share / REFERENCE_SHARE)^STEEPNESS`. A bucket that never folds keeps never folding.
const STEEPNESS: f64 = 0.5;

/// The chance a player continues against a raise that is `share` of the pot after the call, when
/// their model continues against a bet `reference` of the time.
pub(super) fn continues(reference: f64, share: f64) -> f64 {
    let reference = reference.clamp(0.0, 1.0);
    let share = share.max(1e-3);
    reference.powf((share / REFERENCE_SHARE).powf(STEEPNESS))
}

/// Each opponent's chance, in each deal, of continuing against a bet at the model's reference
/// size: `[deal][opponent]`.
pub(super) fn references(
    spot: &Spot,
    dealt: &[Showdown],
    models: &[&ActionModel],
    ranked: &BTreeMap<CardSet, usize>,
) -> Vec<Vec<f64>> {
    let facing = Situation {
        street: Street::of(spot.board),
        facing: true,
    };
    dealt
        .iter()
        .map(|deal| {
            deal.holes
                .iter()
                .zip(models)
                .map(|(hole, model)| {
                    // Every live pair is ranked on the board; a pair the table does not know
                    // reads as the middle of the range.
                    let bucket = ranked
                        .get(hole)
                        .copied()
                        .unwrap_or(crate::action::BUCKETS / 2);
                    1.0 - model.prob(facing, bucket, Move::Fold)
                })
                .collect()
        })
        .collect()
}

/// What raising to add `added` chips is worth, on average over `dealt`, when a call costs `call`
/// and the pot is `pot`. Each opponent that calls is taken to add the raise less our call: the
/// whole of it for the player who bet, a little less than the whole for anyone yet to act.
pub(super) fn raise(
    dealt: &[Showdown],
    references: &[Vec<f64>],
    pot: Chips,
    call: Chips,
    added: Chips,
) -> f64 {
    let (pot, call, added) = ((pot as f64), (call as f64), (added as f64));
    let share = (added - call) / (pot + call).max(1.0);
    let mut total = 0.0;
    for (deal, reference) in dealt.iter().zip(references) {
        let goes_on: Vec<f64> = reference.iter().map(|&r| continues(r, share)).collect();
        let opponents = goes_on.len();
        for callers in 0u64..(1 << opponents) {
            let mut chance = 1.0;
            for (at, &p) in goes_on.iter().enumerate() {
                chance *= if callers & (1 << at) != 0 { p } else { 1.0 - p };
            }
            if chance == 0.0 {
                continue;
            }
            let value = if callers == 0 {
                pot
            } else {
                let n = f64::from(callers.count_ones());
                deal.share_against(callers) * (pot + added + n * (added - call)) - added
            };
            total += chance * value;
        }
    }
    total / (dealt.len().max(1)) as f64
}

/// What calling `call` is worth, on average over `dealt`, into a pot of `pot` where the street's
/// bet is `bet`. An opponent in `answering` (a bit each) has not acted on this street yet: they
/// answer the bet as they would a raise of it, folding or calling, and a caller adds the bet. The
/// rest are in the pot already and stay to the showdown.
pub(super) fn call(
    dealt: &[Showdown],
    references: &[Vec<f64>],
    answering: u64,
    pot: Chips,
    call: Chips,
    bet: Chips,
) -> f64 {
    let (pot, call_chips, bet) = ((pot as f64), (call as f64), (bet as f64));
    let share = bet / pot.max(1.0);
    let mut total = 0.0;
    for (deal, reference) in dealt.iter().zip(references) {
        let opponents = reference.len() as u32;
        let all = (1u64 << opponents) - 1;
        let staying = all & !answering;
        // Each subset of the answering players that calls, with its chance.
        let mut callers = answering;
        loop {
            let mut chance = 1.0;
            for (at, &r) in reference.iter().enumerate() {
                if answering & (1 << at) != 0 {
                    let p = continues(r, share);
                    chance *= if callers & (1 << at) != 0 { p } else { 1.0 - p };
                }
            }
            if chance > 0.0 {
                let n = f64::from(callers.count_ones());
                let against = staying | callers;
                let share_won = if against == 0 {
                    1.0
                } else {
                    deal.share_against(against)
                };
                total += chance * (share_won * (pot + call_chips + n * bet) - call_chips);
            }
            if callers == 0 {
                break;
            }
            callers = (callers - 1) & answering;
        }
    }
    total / (dealt.len().max(1)) as f64
}

/// The raise-to totals worth pricing at `spot`, within its legal range and each once. Before the
/// flop: an open of two and a half blinds, a re-raise of three times the bet, a re-re-raise of 2.3
/// times it, and all-in when the stack is short of three of those. After it: a third, two thirds
/// and the whole of the pot after the call, and all-in when the stack is short of one and a half
/// pots.
pub(super) fn sizes(spot: &Spot) -> Vec<Chips> {
    let Some(range) = spot.legal.raise else {
        return Vec::new();
    };
    let call = spot.legal.call.unwrap_or(0);
    let all_in = spot.committed + spot.stack;
    let bb = spot.big_blind.max(1);
    let mut totals = Vec::new();
    if spot.board.is_empty() {
        let facing_to = spot.committed + call;
        let to = if facing_to <= bb {
            bb * 5 / 2
        } else if facing_to <= bb * 4 {
            facing_to * 3
        } else {
            facing_to * 23 / 10
        };
        totals.push(to);
        if all_in <= to * 3 {
            totals.push(all_in);
        }
    } else {
        let after_call = spot.pot + call;
        for share in [0.33, 0.66, 1.0] {
            totals.push(spot.committed + call + ((after_call as f64) * share) as Chips);
        }
        if (spot.stack as f64) <= 1.5 * (after_call as f64) {
            totals.push(all_in);
        }
    }
    let mut legal: Vec<Chips> = totals
        .into_iter()
        .map(|to| to.clamp(range.min, range.max))
        .collect();
    legal.sort_unstable();
    legal.dedup();
    legal
}

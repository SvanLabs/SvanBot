//! What each action is worth, in chips from here on, over the pricer's sampled deals (V11-39).
//!
//! Folding is worth nothing. A check takes our share of the pot, and a call our share of the pot
//! after the call less the call. A raise is answered by every opponent left in the hand at once:
//! each folds or calls on the cards they were dealt, folding as often as their action model
//! folds that bucket to a bet, and more often the bigger the bet ([`continues`]). Whoever folds
//! leaves the pot to the rest; whoever calls adds the raise and goes to showdown. Re-raises are
//! not answered after our own raise: it is priced as if the table only folds or calls.
//!
//! The answers are summed over every way the table can split into folders and callers, so the
//! value is exact for its deals rather than drawn again.

use svanbot_core::rules::{Chips, Street};
use svanbot_core::spot::Spot;
use svanbot_sim::equity::{MAX_OPPONENTS, Showdown};

use crate::action::{ActionModel, Move, Ranked, Situation};

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

/// How one opponent answers a bet at the model's reference size, on the cards they were dealt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Answer {
    /// The chance they go on, calling or raising.
    pub goes_on: f64,
}

/// Each opponent's answer, in each deal, to a bet at the model's reference size, flattened:
/// `deals × per_deal` entries, a deal's opponents consecutive and in their ranges' order.
pub(super) fn references(
    spot: &Spot,
    dealt: &[Showdown],
    models: &[&ActionModel],
    ranked: &Ranked,
) -> Vec<Answer> {
    let facing = Situation {
        street: Street::of(spot.board),
        facing: true,
    };
    let mut answers = Vec::with_capacity(dealt.len() * models.len());
    for deal in dealt {
        for (hole, model) in deal.holes().iter().zip(models) {
            // Every live pair is ranked on the board; a pair the table does not know
            // reads as the middle of the range.
            let bucket = ranked.get(hole).unwrap_or(crate::action::BUCKETS / 2);
            answers.push(Answer {
                goes_on: 1.0 - model.prob(facing, bucket, Move::Fold),
            });
        }
    }
    answers
}

/// What raising to add `added` chips is worth, on average over `dealt`, when a call costs `call`
/// and the pot is `pot`. Each opponent that calls is taken to add the raise less our call: the
/// whole of it for the player who bet, a little less than the whole for anyone yet to act.
/// `references` is [`references`]' flat output, `per_deal` answers a deal.
pub(super) fn raise(
    dealt: &[Showdown],
    references: &[Answer],
    per_deal: usize,
    pot: Chips,
    call: Chips,
    added: Chips,
) -> f64 {
    let (pot, call, added) = ((pot as f64), (call as f64), (added as f64));
    let share = (added - call) / (pot + call).max(1.0);
    let mut total = 0.0;
    for (deal, reference) in dealt.iter().zip(references.chunks_exact(per_deal.max(1))) {
        // Stack storage: the sample is thousands of deals, and a Vec here is an allocation each.
        let mut goes_on = [0.0f64; MAX_OPPONENTS];
        for (at, answer) in reference.iter().enumerate() {
            goes_on[at] = continues(answer.goes_on, share);
        }
        let goes_on = &goes_on[..reference.len()];
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

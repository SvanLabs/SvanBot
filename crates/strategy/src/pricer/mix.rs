//! The `Balanced` pricer: the same prices as the pricer, but an action is drawn among those
//! within a spread of the best, and an unopened pot's boundary is a chance rather than a cut.
//!
//! The spread is in chips of the pot. An action is eligible when its value is no less than the
//! best minus `spread × pot`; the eligible actions are weighted by `exp((value - best) / band)`,
//! so the near-best are likelier but an action on the edge can still be taken. A zero spread
//! leaves only the best (and its ties). The draw is seeded, so the same spot and seed repeat.

use svanbot_core::rng::Rng;
use svanbot_core::rules::{Action, Chips};
use svanbot_core::spot::Spot;
use svanbot_sim::equity::Showdown;

use crate::action::ActionModel;

use super::{Decision, ev, preflop, ranked};

/// The stream of a decision's seed that the mixing draw uses. Distinct from the sampling seed and
/// the opening stream, so a draw does not change the prices it chooses between.
const MIX_STREAM: u64 = u64::MAX - 1;

/// The stream of a decision's seed that the blended opening boundary uses.
const OPEN_STREAM: u64 = u64::MAX - 2;

/// A `Balanced` decision: the action, the prices behind it, and the draw it came from.
#[derive(Clone, Debug, PartialEq)]
pub struct Balanced {
    pub action: Action,
    pub equity: f64,
    pub price: Option<f64>,
    /// The chance each priced action had. Empty when the opening boundary decided, since that
    /// boundary is a chance over open-or-not, not a chip price.
    pub strategy: Vec<f64>,
    /// Each action's value in chips, in the same order as `strategy`.
    pub values: Vec<f64>,
    pub actions: Vec<Action>,
    pub taken: usize,
}

impl Balanced {
    /// The opening boundary's chance is over open-or-not, not over chip prices. Its `strategy`
    /// stays empty and the policy records no `Drawn`; only the blended action is kept.
    pub(super) fn opening(decision: Decision) -> Self {
        Self {
            action: decision.action,
            equity: decision.equity,
            price: decision.price,
            strategy: Vec::new(),
            values: Vec::new(),
            actions: Vec::new(),
            taken: 0,
        }
    }
}

/// The action worth the most chips from here on, with the same tie rule as the taking pricer: a
/// raise must beat the passive action outright, so a tie stays passive.
pub(super) fn choose(
    spot: &Spot,
    dealt: &[Showdown],
    models: &[&ActionModel],
    equity: f64,
) -> Action {
    let mut values = values(spot, dealt, models, equity).into_iter();
    let Some((mut best, mut worth)) = values.next() else {
        return Action::Check;
    };
    for (action, value) in values {
        if value > worth {
            (best, worth) = (action, value);
        }
    }
    best
}

/// Every action's value in chips, passive first and then the raises in ascending size.
pub(super) fn values(
    spot: &Spot,
    dealt: &[Showdown],
    models: &[&ActionModel],
    equity: f64,
) -> Vec<(Action, f64)> {
    let (passive, worth) = passive(spot, equity);
    let mut values = vec![(passive, worth)];
    let sizes = ev::sizes(spot);
    if !sizes.is_empty() {
        let references = ev::references(spot, dealt, models, &ranked(spot.board));
        let call = spot.legal.call.unwrap_or(0);
        for to in sizes {
            let raised = ev::raise(
                dealt,
                &references,
                models.len(),
                spot.pot,
                call,
                to - spot.committed,
            );
            values.push((Action::RaiseTo(to), raised));
        }
    }
    values
}

/// The passive action and what it is worth: our share of the pot for a check, or the call's
/// chips when it covers its price, else a fold at nothing.
fn passive(spot: &Spot, equity: f64) -> (Action, f64) {
    match spot.legal.call {
        None => (Action::Check, equity * (spot.pot as f64)),
        Some(call) => {
            let pot = spot.callable_pot.unwrap_or(spot.pot);
            let called = equity * ((pot as f64) + (call as f64)) - (call as f64);
            if called >= 0.0 {
                (Action::Call, called)
            } else {
                (Action::Fold, 0.0)
            }
        }
    }
}

/// The draw's result: the action, each action's chance, and the prices behind them.
pub(super) struct Drawn {
    pub action: Action,
    pub strategy: Vec<f64>,
    pub values: Vec<f64>,
    pub actions: Vec<Action>,
    pub taken: usize,
}

/// Draw an action from `values` within `spread × pot` of the best.
///
/// # Panics
/// Panics when `values` is empty: there is no action to draw and no legal default.
pub(super) fn draw(values: &[(Action, f64)], pot: Chips, spread: f64, seed: u64) -> Drawn {
    assert!(
        !values.is_empty(),
        "a draw needs at least one priced action"
    );
    let best = values
        .iter()
        .map(|(_, value)| *value)
        .fold(f64::NEG_INFINITY, f64::max);
    let spread = if spread.is_finite() {
        spread.max(0.0)
    } else {
        0.0
    };
    let band = spread * pot as f64;
    let floor = best - band;
    let eligible: Vec<usize> = values
        .iter()
        .enumerate()
        .filter(|(_, (_, value))| *value >= floor)
        .map(|(at, _)| at)
        .collect();
    let scale = band.max(f64::EPSILON);
    let weights: Vec<f64> = eligible
        .iter()
        .map(|&at| ((values[at].1 - best) / scale).exp())
        .collect();
    let total: f64 = weights.iter().sum();
    let mut strategy = vec![0.0; values.len()];
    if total > 0.0 {
        for (weight, &at) in weights.iter().zip(&eligible) {
            strategy[at] = weight / total;
        }
    } else {
        strategy[eligible[0]] = 1.0;
    }
    let roll = roll(seed, MIX_STREAM) * total.max(f64::MIN_POSITIVE);
    let mut seen = 0.0;
    let mut taken = *eligible.last().unwrap_or(&0);
    for (weight, &at) in weights.iter().zip(&eligible) {
        seen += weight;
        if roll < seen {
            taken = at;
            break;
        }
    }
    Drawn {
        action: values[taken].0,
        strategy,
        values: values.iter().map(|(_, value)| *value).collect(),
        actions: values.iter().map(|(action, _)| *action).collect(),
        taken,
    }
}

/// [`draw`]'s action alone, for a caller that only needs the choice.
///
/// # Panics
/// Panics when `values` is empty.
pub fn draw_within_spread(values: &[(Action, f64)], pot: Chips, spread: f64, seed: u64) -> Action {
    draw(values, pot, spread, seed).action
}

/// The unopened pot's decision with the blended boundary. The class opens with
/// [`preflop::open_chance`]'s chance; the draw is seeded, so the same spot and seed repeat.
pub(super) fn opening(spot: &Spot, open_scale: f64, seed: u64) -> Option<Decision> {
    let class = super::opening_class(spot)?;
    let position = preflop::Position {
        behind: spot.opponents,
        in_position: spot.in_position,
    };
    let chance = preflop::open_chance(class, position, open_scale);
    let open = if chance >= 1.0 {
        true
    } else if chance <= 0.0 {
        false
    } else {
        roll(seed, OPEN_STREAM) < chance
    };
    super::opening_with(spot, open)
}

/// A number in `[0, 1)` from the decision's seed and stream.
fn roll(seed: u64, stream: u64) -> f64 {
    let bits = Rng::stream(seed, stream).next_u64() >> 11;
    bits as f64 / (1u64 << 53) as f64
}

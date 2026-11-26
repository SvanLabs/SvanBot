//! The pricer: the fallback way to decide. It prices the legal actions by the bot's equity against
//! the opponents' ranges, and takes the best. Given an action model that has passed the gate
//! (`learn::fit::gate`), each opponent's range follows from what they did in the hand. Without
//! one, every range is uniform: all 1,326 pairs of hole cards, equally likely.
//!
//! - It raises for value when its equity is well above its fair share of the pot.
//! - It calls when its equity covers the price of the call.
//! - It checks when the check is free, and it never folds then.
//!
//! The `Balanced` kind uses the same prices, then draws among the actions within a spread of the
//! best instead of taking it ([`mix`]).

use svanbot_core::card::{Card, CardSet, Rank, Suit};
use svanbot_core::rules::{Action, Street};
use svanbot_core::spot::Spot;
use svanbot_sim::equity::{Showdown, equity_over, showdowns};

use crate::action::{ActionModel, Seen, range, ranked};
use crate::preflop;

mod ev;
mod mix;
pub use mix::{Balanced, draw_within_spread};

/// The work a decision may do, passed in so the decision never reads the clock: how many deals
/// to sample, and the seed they are drawn from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub samples: u64,
    pub seed: u64,
}

/// The action the pricer chose, the equity it chose it on, and the price it compared against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decision {
    pub action: Action,
    pub equity: f64,
    /// The share of the pot a call costs, `call / (pot + call)`, as a fraction of the pot after
    /// the call. `None` when there was nothing to call: a free check is priced at nothing, and a
    /// zero would say it costs nothing of a pot it is not in.
    pub price: Option<f64>,
}

/// The pricer's decision at `spot`, with each opponent's range from `model` and their line, or
/// uniform when there is no model or no line.
pub fn decide(spot: &Spot, model: Option<&ActionModel>, budget: Budget) -> Decision {
    match model {
        Some(model) => price_lines(spot, &vec![model; spot.lines.len()], budget),
        None => price_lines(spot, &[], budget),
    }
}

/// [`decide`] with each opponent's line read by its own model: `models[i]` reads `spot.lines[i]`.
/// A line without a model is read as uniform.
pub fn price_lines(spot: &Spot, models: &[&ActionModel], budget: Budget) -> Decision {
    price_lines_at(spot, models, budget, 1.0)
}

/// Like [`price_lines`], with a scaled opening range in unopened pots.
pub fn price_lines_at(
    spot: &Spot,
    models: &[&ActionModel],
    budget: Budget,
    open_scale: f64,
) -> Decision {
    if let Some(decision) = opening(spot, open_scale) {
        return decision;
    }
    let baseline = ActionModel::baseline();
    let sampled = sampled(spot, models, &baseline, budget);
    Decision {
        action: mix::choose(spot, &sampled.dealt, &sampled.folding, sampled.equity),
        equity: sampled.equity,
        price: price_of(spot),
    }
}

/// Like [`price_lines_at`] for the `Balanced` kind: the same prices, but a draw among the actions
/// within `spread` of the best, and a blended opening boundary. `spread` is in chips of the pot.
pub fn price_lines_balanced(
    spot: &Spot,
    models: &[&ActionModel],
    budget: Budget,
    open_scale: f64,
    spread: f64,
) -> Balanced {
    if let Some(decision) = mix::opening(spot, open_scale, budget.seed) {
        return Balanced::opening(decision);
    }
    let baseline = ActionModel::baseline();
    let sampled = sampled(spot, models, &baseline, budget);
    let values = mix::values(spot, &sampled.dealt, &sampled.folding, sampled.equity);
    let drawn = mix::draw(&values, spot.pot, spread, budget.seed);
    Balanced {
        action: drawn.action,
        equity: sampled.equity,
        price: price_of(spot),
        strategy: drawn.strategy,
        values: drawn.values,
        actions: drawn.actions,
        taken: drawn.taken,
    }
}

/// The share a call costs: `call / (callable pot + call)`. Without known contributions use the
/// ordinary pot price. `None` when there is nothing to call.
pub fn price_of(spot: &Spot) -> Option<f64> {
    spot.legal.call.map(|call| {
        (call as f64) / ((spot.callable_pot.unwrap_or(spot.pot) as f64) + (call as f64))
    })
}

/// The sampled deals, the models that answer a raise, and the equity over them. Shared by the
/// taking pricer and the mixing one, so both price the same spot from the same deals.
pub(super) struct Sampled<'a> {
    pub dealt: Vec<Showdown>,
    pub folding: Vec<&'a ActionModel>,
    pub equity: f64,
}

pub(super) fn sampled<'a>(
    spot: &Spot,
    models: &[&'a ActionModel],
    baseline: &'a ActionModel,
    budget: Budget,
) -> Sampled<'a> {
    let uniform = uniform_range(spot.hole | spot.board);
    let read: Vec<Vec<(CardSet, f64)>> = spot
        .lines
        .iter()
        .zip(models)
        .take(spot.opponents.max(1))
        .map(|(line, model)| {
            let seen: Vec<Seen> = line.iter().map(Seen::of).collect();
            range(model, &seen, spot.hole | spot.board)
        })
        .collect();
    let mut ranges: Vec<&[(CardSet, f64)]> = read.iter().map(Vec::as_slice).collect();
    ranges.resize(spot.opponents.max(1), &uniform);
    let dealt = showdowns(spot.hole, &ranges, spot.board, budget.samples, budget.seed);
    let equity = equity_over(&dealt);
    // A line with no model of its own answers a raise as the baseline's sound opponent would.
    let folding = (0..ranges.len())
        .map(|at| models.get(at).copied().unwrap_or(baseline))
        .collect();
    Sampled {
        dealt,
        folding,
        equity,
    }
}

/// The decision in an unopened pot, from the opening range ([`preflop::opens_at`]): before the
/// flop, with nobody yet to have acted, every opponent still in is behind us. A hand the position
/// opens is raised to the table's standard open, and any other folds (or checks, when it is free).
/// `None` when the pot has been opened, when a raise is not open to us, or when the caller did not
/// follow the hand: with no lines, an unopened pot cannot be told from a raised one.
fn opening(spot: &Spot, open_scale: f64) -> Option<Decision> {
    let class = opening_class(spot)?;
    let position = preflop::Position {
        behind: spot.opponents,
        in_position: spot.in_position,
    };
    opening_with(spot, preflop::opens_at(class, position, open_scale))
}

/// The class and position of an unopened pot, or `None` when this is not one.
pub(super) fn opening_class(spot: &Spot) -> Option<usize> {
    if !spot.board.is_empty() || spot.lines.is_empty() || spot.legal.raise.is_none() {
        return None;
    }
    if spot
        .lines
        .iter()
        .flatten()
        .any(|act| act.street == Street::Preflop)
    {
        return None;
    }
    Some(preflop::class_of(spot.hole))
}

/// The unopened pot's decision when `open` says the class opens.
pub(super) fn opening_with(spot: &Spot, open: bool) -> Option<Decision> {
    opening_class(spot)?;
    let class = preflop::class_of(spot.hole);
    let action = if open {
        Action::RaiseTo(*ev::sizes(spot).first()?)
    } else if spot.legal.call.is_none() {
        Action::Check
    } else {
        Action::Fold
    };
    Some(Decision {
        action,
        equity: preflop::equity(class, spot.opponents),
        price: price_of(spot),
    })
}

/// Every pair of hole cards that avoids the `dead` cards, each with weight 1.
pub(crate) fn uniform_range(dead: CardSet) -> Vec<(CardSet, f64)> {
    let deck: Vec<Card> = Rank::ALL
        .into_iter()
        .flat_map(|rank| Suit::ALL.map(|suit| Card::new(rank, suit)))
        .filter(|&card| !dead.contains(card))
        .collect();
    let mut range = Vec::with_capacity(deck.len() * (deck.len() - 1) / 2);
    for (i, &first) in deck.iter().enumerate() {
        for &second in &deck[i + 1..] {
            range.push(([first, second].into_iter().collect(), 1.0));
        }
    }
    range
}

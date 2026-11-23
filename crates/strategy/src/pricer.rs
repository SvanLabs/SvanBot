//! The pricer: the fallback way to decide. It prices the legal actions by the bot's equity against
//! the opponents' ranges, and takes the best. Given an action model that has passed the gate
//! (`learn::fit::gate`), each opponent's range follows from what they did in the hand. Without
//! one, every range is uniform: all 1,326 pairs of hole cards, equally likely.
//!
//! - It raises for value when its equity is well above its fair share of the pot.
//! - It calls when its equity covers the price of the call.
//! - It checks when the check is free, and it never folds then.

use svanbot_core::card::{Card, CardSet, Rank, Suit};
use svanbot_core::rules::Action;
use svanbot_core::spot::Spot;
use svanbot_sim::equity::sampled;

use crate::action::{ActionModel, Seen, range};
use crate::params::Params;

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
    price(spot, model, budget, &Params::DEFAULT)
}

/// [`decide`] by the margins of `params`.
pub fn price(
    spot: &Spot,
    model: Option<&ActionModel>,
    budget: Budget,
    params: &Params,
) -> Decision {
    match model {
        Some(model) => price_lines(spot, &vec![model; spot.lines.len()], budget, params),
        None => price_lines(spot, &[], budget, params),
    }
}

/// [`price`] with each opponent's line read by its own model: `models[i]` reads `spot.lines[i]`.
/// A line without a model is read as uniform.
pub fn price_lines(
    spot: &Spot,
    models: &[&ActionModel],
    budget: Budget,
    params: &Params,
) -> Decision {
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
    let equity = sampled(spot.hole, &ranges, spot.board, budget.samples, budget.seed);
    Decision {
        action: choose(spot, equity, params),
        equity,
        price: price_of(spot),
    }
}

/// The share of the pot a call costs: `call / (pot + call)`, the price the equity is measured
/// against. `None` when there is nothing to call.
pub fn price_of(spot: &Spot) -> Option<f64> {
    spot.legal
        .call
        .map(|call| f64::from(call) / f64::from(spot.pot + call))
}

fn choose(spot: &Spot, equity: f64, params: &Params) -> Action {
    let fair = 1.0 / (spot.opponents.max(1) + 1) as f64;
    if let Some(range) = spot.legal.raise
        && equity > fair + (1.0 - fair) * params.value_margin
    {
        let size = range
            .min
            .saturating_add((f64::from(spot.pot) * params.raise_share) as u32);
        return Action::RaiseTo(size.clamp(range.min, range.max));
    }
    match price_of(spot) {
        None => Action::Check,
        Some(price) if equity >= price => Action::Call,
        Some(_) => Action::Fold,
    }
}

/// Every pair of hole cards that avoids the `dead` cards, each with weight 1.
fn uniform_range(dead: CardSet) -> Vec<(CardSet, f64)> {
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

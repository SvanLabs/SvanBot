//! The pricer: the fallback way to decide. It prices the legal actions by the bot's equity against
//! the opponents' ranges, and takes the best. Given an action model that has passed the gate
//! (`learn::fit::gate`), each opponent's range follows from what they did in the hand. Without
//! one, every range is uniform: all 1,326 pairs of hole cards, equally likely.
//!
//! - It raises for value when its equity is well above its fair share of the pot.
//! - It calls when its equity covers the price of the call.
//! - It checks when the check is free, and it never folds then.

use svanbot_core::card::{Card, CardSet, Rank, Suit};
use svanbot_core::rules::{Action, Street};
use svanbot_core::spot::Spot;
use svanbot_sim::equity::{Showdown, equity_over, showdowns};

use crate::action::{ActionModel, Seen, range, ranked};

mod ev;
use crate::preflop;

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
    if let Some(decision) = opening(spot) {
        return decision;
    }
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
    let baseline = ActionModel::baseline();
    let folding: Vec<&ActionModel> = (0..ranges.len())
        .map(|at| models.get(at).copied().unwrap_or(&baseline))
        .collect();
    Decision {
        action: choose(spot, &dealt, &folding, equity),
        equity,
        price: price_of(spot),
    }
}

/// The share a call costs: `call / (callable pot + call)`. Without known contributions use the
/// ordinary pot price. `None` when there is nothing to call.
pub fn price_of(spot: &Spot) -> Option<f64> {
    spot.legal.call.map(|call| {
        (call as f64) / ((spot.callable_pot.unwrap_or(spot.pot) as f64) + (call as f64))
    })
}

/// The action worth the most chips from here on ([`ev`]). The passive action is worth our share
/// of the pot it is played into; a raise must beat it outright, so a tie stays passive, and a
/// free check is never folded.
fn choose(spot: &Spot, dealt: &[Showdown], models: &[&ActionModel], equity: f64) -> Action {
    let sizes = ev::sizes(spot);
    let answering = answering(spot, models.len());
    // Each opponent's fold chances, built only when a raise or an answered call will read them.
    let references = if !sizes.is_empty() || (spot.legal.call.is_some() && answering != 0) {
        ev::references(spot, dealt, models, &ranked(spot.board))
    } else {
        Vec::new()
    };
    let (mut best, mut worth) = match spot.legal.call {
        None => (Action::Check, equity * (spot.pot as f64)),
        Some(call) => {
            let pot = spot.callable_pot.unwrap_or(spot.pot);
            let called = if answering == 0 {
                equity * ((pot as f64) + (call as f64)) - (call as f64)
            } else {
                ev::call(
                    dealt,
                    &references,
                    answering,
                    pot,
                    call,
                    spot.committed + call,
                )
            };
            if called >= 0.0 {
                (Action::Call, called)
            } else {
                (Action::Fold, 0.0)
            }
        }
    };
    if !sizes.is_empty() {
        let call = spot.legal.call.unwrap_or(0);
        for to in sizes {
            let raised = ev::raise(dealt, &references, spot.pot, call, to - spot.committed);
            if raised > worth {
                (best, worth) = (Action::RaiseTo(to), raised);
            }
        }
    }
    best
}

/// The decision in an unopened pot, from the seat's chart ([`preflop::opens`]): before the flop,
/// with nobody yet to have acted, every opponent still in is behind us. A hand the seat opens is
/// raised to the table's standard open, and any other folds (or checks, when it is free). `None`
/// when the pot has been opened, when a raise is not open to us, or when the caller did not follow
/// the hand: with no lines, an unopened pot cannot be told from a raised one.
fn opening(spot: &Spot) -> Option<Decision> {
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
    let class = preflop::class_of(spot.hole);
    let seat = preflop::Seat {
        behind: spot.opponents,
        in_position: spot.in_position,
    };
    let action = if preflop::opens(class, seat) {
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

/// The opponents, a bit each in the order of the ranges, who have not acted on this street yet
/// and so still answer its bet. None when the caller did not follow the hand: with no lines,
/// nobody can be told apart, and every opponent is taken to be in.
fn answering(spot: &Spot, opponents: usize) -> u64 {
    if spot.lines.is_empty() {
        return 0;
    }
    let street = Street::of(spot.board);
    (0..opponents.min(31))
        .filter(|&at| {
            spot.lines
                .get(at)
                .is_none_or(|line| line.iter().all(|act| act.street != street))
        })
        .fold(0, |bits, at| bits | (1 << at))
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

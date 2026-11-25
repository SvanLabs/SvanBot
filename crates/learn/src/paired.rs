//! The paired sim: two policies play the same deals from mirrored seats, heads up, so the luck
//! of the cards cancels out. Each deal is played twice, once with the challenger in each seat,
//! and the deal's result is what the challenger won over the two hands, with the luck correction
//! for an all-in ([`play_corrected`](svanbot_sim::table::play_corrected)).
//!
//! A deal is drawn from its own stream of the seed ([`Rng::stream`]), so the deals can be played
//! in any order, on any number of threads, and add up the same.

use std::cell::RefCell;
use std::rc::Rc;

use svanbot_core::rng::Rng;
use svanbot_core::rules::{Action, Blinds, Chips};
use svanbot_core::spot::{Act, Spot};
use svanbot_sim::table::{Record, Seen, deal, play_observed};

use crate::probe::{Decided, corrected};

/// Draws from the current hand, shared by its decision maker and bout reader.
pub type Draws = Rc<RefCell<Vec<Decided>>>;

/// The challenger in a paired game. A bout can learn between deals while every rotation
/// within one deal plays from the same read.
pub trait Challenger {
    fn decide(&self, spot: &Spot, seed: u64) -> Action;
    fn finished_hand(&self, _us: usize, _lines: &[Vec<Act>], _actions: &[Record]) {}
    fn finished_deal(&self) {}

    /// Take and clear the draws used in the hand just played.
    fn take_decided(&self) -> Vec<Decided> {
        Vec::new()
    }
}

impl<F: Fn(&Spot, u64) -> Action> Challenger for F {
    fn decide(&self, spot: &Spot, seed: u64) -> Action {
        self(spot, seed)
    }
}

/// What every deal of a paired sim shares: the blinds, the stack each seat starts each hand
/// with, and the seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub blinds: Blinds,
    pub stack: Chips,
    pub seed: u64,
}

/// Sets the seeds handed to the decisions apart from the seeds of the deals.
const DECISIONS: u64 = 0x5eed_0fde_c151_0500;

/// Plays deal number `number` twice, the challenger on the button and then off it, and returns
/// what the challenger won in big blinds a hand. A policy is asked for its action at a spot, and
/// handed a seed to draw from.
pub fn duel(
    game: &Match,
    number: u64,
    challenger: &impl Fn(&Spot, u64) -> Action,
    champion: &impl Fn(&Spot, u64) -> Action,
) -> f64 {
    duel_with(game, number, challenger, champion, || {})
}

/// [`duel`], with `reset` run before each of the deal's two hands. A stateful opponent (the
/// learning archetype) is put back to where the deal began, so both mirrored hands face it the
/// same and the deal still cancels the luck of the cards. A stateless one passes `|| {}`.
pub fn duel_with(
    game: &Match,
    number: u64,
    challenger: &impl Fn(&Spot, u64) -> Action,
    champion: &impl Fn(&Spot, u64) -> Action,
    reset: impl Fn(),
) -> f64 {
    duel_seen(
        game,
        number,
        challenger,
        &|_, spot, _, seed| champion(spot, seed),
        reset,
    )
}

/// [`duel_with`], with the champion asked at its seat and shown the whole table ([`Seen`]): an
/// opponent that reads every seat, as v10.0.2 does. The challenger sees only its spot.
pub fn duel_seen(
    game: &Match,
    number: u64,
    challenger: &impl Challenger,
    champion: &impl Fn(usize, &Spot, &Seen, u64) -> Action,
    reset: impl Fn(),
) -> f64 {
    let cards = deal(2, &mut Rng::stream(game.seed, number));
    // Both hands of the deal pay an all-in over the same run-outs, so the mirror still cancels.
    let run_outs = Rng::stream(game.seed ^ DECISIONS, 2 * number).next_u64();
    let mut chips = 0.0;
    for seat in 0..2 {
        reset();
        let mut seeds = Rng::stream(game.seed ^ DECISIONS, 2 * number + seat as u64);
        let won = play_observed(
            &cards,
            &[game.stack; 2],
            0,
            game.blinds,
            run_outs,
            |at, spot, seen| {
                let seed = seeds.next_u64();
                if at == seat {
                    challenger.decide(spot, seed)
                } else {
                    champion(at, spot, seen, seed)
                }
            },
            |lines, actions| challenger.finished_hand(seat, lines, actions),
        );
        chips += corrected(won[seat], &challenger.take_decided());
    }
    challenger.finished_deal();
    chips / (2.0 * game.blinds.big as f64)
}

/// What a paired sim found: the challenger's win rate over the champion in mbb/100, and the
/// half-width of its 95% confidence interval.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Verdict {
    pub deals: usize,
    pub mbb_per_100: f64,
    pub ci95: f64,
}

impl Verdict {
    /// Whether the whole 95% interval is above 0: the challenger beats the champion.
    pub fn above_zero(&self) -> bool {
        self.mbb_per_100 - self.ci95 > 0.0
    }

    /// Whether the whole 95% interval is below 0: the challenger loses to the champion.
    pub fn below_zero(&self) -> bool {
        self.mbb_per_100 + self.ci95 < 0.0
    }
}

/// Big blinds a hand, as mbb/100.
const MBB_PER_100: f64 = 100_000.0;

/// The verdict on the deals' results ([`duel`]), in the order of the deals.
pub fn verdict(results: &[f64]) -> Verdict {
    let n = results.len() as f64;
    let mean = results.iter().sum::<f64>() / n.max(1.0);
    let spread: f64 = results.iter().map(|result| (result - mean).powi(2)).sum();
    let error = (spread / (n - 1.0).max(1.0) / n.max(1.0)).sqrt();
    Verdict {
        deals: results.len(),
        mbb_per_100: mean * MBB_PER_100,
        ci95: 1.96 * error * MBB_PER_100,
    }
}

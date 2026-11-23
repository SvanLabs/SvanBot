//! The two deciders a sim plays with, shared by `svanbot paired` and `svanbot arena`: the policy
//! under a parameter set, and the pricer. Both sims ask a decider for an action at a spot and
//! hand it a seed, so the same code plays both sides of the same game.

use svanbot_core::rules::{Action, Blinds};
use svanbot_core::spot::Spot;
use svanbot_learn::arena::opponent_name;
use svanbot_learn::paired::Match;
use svanbot_learn::reads::Reads;
use svanbot_strategy::action::ActionModel;
use svanbot_strategy::params::Params;
use svanbot_strategy::policy::{self, Personal};
use svanbot_strategy::pricer::{self, Budget};
use svanbot_strategy::resolve::iterations_for;

/// Deals sampled by the pricer per decision in a sim.
pub(crate) const SAMPLES: u64 = 2_000;

/// The game both sims play, from `seed`: ten-twenty blinds, a hundred big blinds deep.
pub(crate) fn game(seed: u64) -> Match {
    Match {
        blinds: Blinds { small: 10, big: 20 },
        stack: 2000,
        seed,
    }
}

/// The policy under `params`, as a decider. With `fixed` the re-solve runs that many iterations
/// on every spot; without, the compute budget sizes them by the pot, as in live play.
pub(crate) fn challenger(
    params: &Params,
    fixed: Option<u64>,
) -> impl Fn(&Spot, u64) -> Action + '_ {
    move |spot, seed| {
        let iterations = match fixed {
            Some(fixed) => u32::try_from(fixed).unwrap_or(u32::MAX),
            None => iterations_for(spot.pot, spot.big_blind, spot.board.len()),
        };
        let budget = Budget {
            samples: SAMPLES,
            seed,
        };
        policy::decide(spot, None, params, budget, iterations)
    }
}

/// How a decision takes its reads of the seats it names: [`Reads::deciding`] or
/// [`Reads::personal`].
pub(crate) type Take = fn(&Reads, &[(usize, &str)], &ActionModel, f64) -> Vec<Personal>;

/// [`challenger`] reading the sim's opponent by `reads`, as live play reads each opponent by
/// theirs. Heads up the villain follows the challenger through the mirror.
pub(crate) fn reader<'a>(
    params: &'a Params,
    fixed: Option<u64>,
    reads: &'a Reads,
) -> impl Fn(&Spot, u64) -> Action + 'a {
    // Always on: this is the sim that measures whether reads may come into play.
    let decide = reading_policy(params, fixed, 2, Reads::personal);
    move |spot, seed| decide(spot, seed, reads)
}

/// The Arena and the frozen comparison use the same policy and the same names for the seats.
/// `take` is how the decision takes the reads: [`Reads::deciding`] as in play, held until they
/// clear their gate, or [`Reads::personal`] to measure them.
pub(crate) fn reading_policy(
    params: &Params,
    fixed: Option<u64>,
    seats: usize,
    take: Take,
) -> impl Fn(&Spot, u64, &Reads) -> Action + '_ {
    let names: Vec<_> = (0..seats).map(|seat| opponent_name(seats, seat)).collect();
    let baseline = ActionModel::baseline();
    move |spot, seed, reads| {
        let iterations = match fixed {
            Some(fixed) => u32::try_from(fixed).unwrap_or(u32::MAX),
            None => iterations_for(spot.pot, spot.big_blind, spot.board.len()),
        };
        let budget = Budget {
            samples: SAMPLES,
            seed,
        };
        let named: Vec<(usize, &str)> = spot
            .seats
            .iter()
            .map(|&seat| (seat, names[seat].as_str()))
            .collect();
        let personal = take(reads, &named, &baseline, params.exploit);
        policy::decide_read(spot, None, &personal, params, budget, iterations).action
    }
}

/// The pricer, as a decider.
pub(crate) fn pricer() -> impl Fn(&Spot, u64) -> Action {
    |spot, seed| {
        let budget = Budget {
            samples: SAMPLES,
            seed,
        };
        pricer::decide(spot, None, budget).action
    }
}

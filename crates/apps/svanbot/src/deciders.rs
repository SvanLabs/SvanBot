//! The two deciders a sim plays with, shared by `svanbot paired` and `svanbot arena`: the policy
//! under a parameter set, and the pricer. Both sims ask a decider for an action at a spot and
//! hand it a seed, so the same code plays both sides of the same game.

use svanbot_core::rules::{Action, Blinds};
use svanbot_core::spot::Spot;
use svanbot_learn::paired::Match;
use svanbot_strategy::params::Params;
use svanbot_strategy::policy;
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

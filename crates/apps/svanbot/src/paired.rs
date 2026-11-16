//! `svanbot paired`: a paired sim of the policy against the pricer, heads up, then exit. It
//! touches neither the venue nor a store.
//!
//! `svanbot paired <deals> [seed] [iterations]`: each deal is played twice from mirrored seats.
//! With `iterations` the re-solve runs that many on every spot; without, the compute budget
//! sizes them by the pot, as in live play. It prints the policy's win rate in mbb/100 with its
//! 95% interval, and exits 1 when the whole interval is below 0.

use std::process::ExitCode;

use svanbot_core::rules::Action;
use svanbot_core::rules::Blinds;
use svanbot_core::spot::Spot;
use svanbot_learn::paired::{Match, Verdict, duel, verdict};
use svanbot_strategy::params::Params;
use svanbot_strategy::policy;
use svanbot_strategy::pricer::{self, Budget};
use svanbot_strategy::resolve::iterations_for;

/// Deals sampled by the pricer per decision in a sim.
const SAMPLES: u64 = 2_000;

/// Threads the deals are spread over. The box is shared with the live fleet.
const THREADS: u64 = 4;

pub(crate) fn run(args: &[String]) -> ExitCode {
    let number = |at: usize| args.get(at).map(|text| text.parse::<u64>());
    let (Some(Ok(deals)), seed, fixed) = (number(0), number(1), number(2)) else {
        eprintln!("svanbot paired: usage: svanbot paired <deals> [seed] [iterations]");
        return ExitCode::from(2);
    };
    let (Ok(seed), Ok(fixed)) = (seed.transpose(), fixed.transpose()) else {
        eprintln!("svanbot paired: the seed and the iterations are whole numbers");
        return ExitCode::from(2);
    };
    let game = Match {
        blinds: Blinds { small: 10, big: 20 },
        stack: 2000,
        seed: seed.unwrap_or(1),
    };
    let challenger = |spot: &Spot, seed: u64| {
        let iterations = match fixed {
            Some(fixed) => u32::try_from(fixed).unwrap_or(u32::MAX),
            None => iterations_for(spot.pot, spot.big_blind, spot.board.len()),
        };
        let budget = Budget {
            samples: SAMPLES,
            seed,
        };
        policy::decide(spot, None, &Params::DEFAULT, budget, iterations)
    };
    let champion = |spot: &Spot, seed: u64| {
        let budget = Budget {
            samples: SAMPLES,
            seed,
        };
        pricer::decide(spot, None, budget).action
    };
    let found = sim(&game, deals, &challenger, &champion);
    println!(
        "policy against pricer, heads up: {:+.0} mbb/100, 95% CI ±{:.0}, over {} deals (seed {})",
        found.mbb_per_100, found.ci95, found.deals, game.seed
    );
    if found.below_zero() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// Plays `deals` deals of `game` between two policies and returns the challenger's verdict.
pub(crate) fn sim(
    game: &Match,
    deals: u64,
    challenger: &(impl Fn(&Spot, u64) -> Action + Sync),
    champion: &(impl Fn(&Spot, u64) -> Action + Sync),
) -> Verdict {
    // Thread `k` plays deals k, k + THREADS, …; put back in order, the sum is the same on any
    // number of threads.
    let mut results = vec![0.0; deals as usize];
    let played: Vec<Vec<(u64, f64)>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..THREADS)
            .map(|k| {
                scope.spawn(move || {
                    (k..deals)
                        .step_by(THREADS as usize)
                        .map(|deal| (deal, duel(game, deal, challenger, champion)))
                        .collect()
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap_or_default())
            .collect()
    });
    for (deal, result) in played.into_iter().flatten() {
        results[deal as usize] = result;
    }
    verdict(&results)
}

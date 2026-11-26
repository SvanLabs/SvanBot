//! `svanbot paired`: a paired sim of the policy against the pricer, heads up, then exit. It
//! touches neither the venue nor a store.
//!
//! `svanbot paired <deals> [seed] [iterations]`: each deal is played twice from mirrored seats.
//! With `iterations` the re-solve runs that many on every spot; without, the compute budget
//! sizes them by the pot, as in live play. It prints the policy's win rate in mbb/100 with its
//! 95% interval, and exits 1 when the whole interval is below 0.

use std::process::ExitCode;

use svanbot_core::rules::Action;
use svanbot_core::spot::Spot;
use svanbot_learn::paired::{Match, Verdict, duel, verdict};
use svanbot_learn::ring::{SIX_MAX, ring};
use svanbot_strategy::params::Params;

use crate::deciders;

/// Threads the deals are spread over. The box is shared with the live fleet.
const THREADS: u64 = 4;

pub(crate) fn run(args: &[String]) -> ExitCode {
    match args.first().map(String::as_str) {
        Some("reads") => return crate::paired_reads::run(&args[1..]),
        Some("compare") => return crate::paired_compare::run(&args[1..]),
        _ => {}
    }
    let number = |at: usize| args.get(at).map(|text| text.parse::<u64>());
    let (Some(Ok(deals)), seed, fixed) = (number(0), number(1), number(2)) else {
        eprintln!("svanbot paired: usage: svanbot paired <deals> [seed] [iterations]");
        return ExitCode::from(2);
    };
    let (Ok(seed), Ok(fixed)) = (seed.transpose(), fixed.transpose()) else {
        eprintln!("svanbot paired: the seed and the iterations are whole numbers");
        return ExitCode::from(2);
    };
    let game = deciders::game(seed.unwrap_or(1));
    let challenger = deciders::challenger(&Params::DEFAULT, fixed);
    let champion = deciders::pricer();
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

/// Plays `deals` deals with `play`, thread `k` taking deals `k`, `k + THREADS`, …, and puts the
/// results back in deal order: the sum is the same on any number of threads.
fn play_deals(deals: u64, play: impl Fn(u64) -> f64 + Sync) -> Vec<f64> {
    let play = &play;
    let mut results = vec![0.0; deals as usize];
    let played: Vec<Vec<(u64, f64)>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..THREADS)
            .map(|k| {
                scope.spawn(move || {
                    (k..deals)
                        .step_by(THREADS as usize)
                        .map(|deal| (deal, play(deal)))
                        .collect()
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| {
                // A panicked worker would leave its deals at zero and call that a result; a
                // panicked sim is a bug, not a verdict.
                worker.join().expect("a paired sim worker panicked")
            })
            .collect()
    });
    for (deal, result) in played.into_iter().flatten() {
        results[deal as usize] = result;
    }
    results
}

/// Plays `deals` deals of `game` between two policies and returns the challenger's verdict.
pub(crate) fn sim(
    game: &Match,
    deals: u64,
    challenger: &(impl Fn(&Spot, u64) -> Action + Sync),
    champion: &(impl Fn(&Spot, u64) -> Action + Sync),
) -> Verdict {
    verdict(&play_deals(deals, |deal| {
        duel(game, deal, challenger, champion)
    }))
}

/// Plays `deals` deals of 6-max ring of the challenger against five seats of the champion, and
/// returns the challenger's verdict. Each deal is played from every seat, so the luck of the
/// cards cancels the way the heads-up mirror does. This is the fresh-deal confirmation.
pub(crate) fn ring_sim(
    game: &Match,
    deals: u64,
    challenger: &(impl Fn(&Spot, u64) -> Action + Sync),
    champion: &(impl Fn(&Spot, u64) -> Action + Sync),
) -> Verdict {
    verdict(&play_deals(deals, |deal| {
        ring(game, SIX_MAX, deal, challenger, &|_, spot, seed| {
            champion(spot, seed)
        })
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use svanbot_core::rules::Blinds;

    fn folds(_: &Spot, _: u64) -> Action {
        Action::Fold
    }

    #[test]
    fn a_six_max_ring_of_a_policy_against_itself_breaks_even() {
        let game = Match {
            blinds: Blinds { small: 10, big: 20 },
            stack: 2_000,
            seed: 7,
        };
        let found = ring_sim(&game, 2, &folds, &folds);
        assert_eq!(found.deals, 2);
        assert_eq!(found.mbb_per_100, 0.0);
    }
}

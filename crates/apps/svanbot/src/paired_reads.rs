//! `svanbot paired reads <deals> [seed] [warm-up]`: what playing by an opponent's own read wins,
//! heads up, then exit (V11-25). It touches neither the venue nor a store.
//!
//! Against each fixed sparring archetype, a warm-up of `warm-up` deals (default 200) builds the
//! policy's read of the opponent with the live code, and freezes it. Then each of `deals` fresh
//! deals is played by the policy with the read and without, and the difference is the result. It
//! prints a line per archetype and one over all of them, and exits 0 only when that last
//! interval is wholly above 0.

use std::process::ExitCode;

use svanbot_learn::paired::{Match, Verdict, verdict};
use svanbot_learn::read_sim::{OPPONENT, reads_on_off, warm_up};
use svanbot_sim::sparring::Archetype;
use svanbot_strategy::params::Params;

use crate::deciders;

/// Threads the deals are spread over. The box is shared with the live fleet.
const THREADS: u64 = 4;

/// The archetypes that play one strategy throughout, whose reads should pay.
const FIXED: [Archetype; 3] = [Archetype::Station, Archetype::Nit, Archetype::Maniac];

pub(crate) fn run(args: &[String]) -> ExitCode {
    let number = |at: usize| args.get(at).map(|text| text.parse::<u64>());
    let (Some(Ok(deals)), seed, warm) = (number(0), number(1), number(2)) else {
        eprintln!("svanbot paired reads: usage: svanbot paired reads <deals> [seed] [warm-up]");
        return ExitCode::from(2);
    };
    let (Ok(seed), Ok(warm)) = (seed.transpose(), warm.transpose()) else {
        eprintln!("svanbot paired reads: the seed and the warm-up are whole numbers");
        return ExitCode::from(2);
    };
    let (game, warm) = (deciders::game(seed.unwrap_or(1)), warm.unwrap_or(200));
    let params = Params::DEFAULT;
    let off = deciders::challenger(&params, None);
    let mut all = Vec::new();
    for archetype in FIXED {
        let reads = warm_up(&game, archetype, 0..warm, &off);
        let read = reads.get(OPPONENT);
        let on = deciders::reader(&params, None, &reads);
        let results = measure(&game, archetype, warm..warm + deals, &on, &off);
        let found = verdict(&results);
        println!(
            "reads on against off, {}: {}, read {} at {:.0}% of the budget",
            archetype.name(),
            line(&found),
            read.map_or("nothing", |read| read.class().name()),
            read.map_or(0.0, |read| read.earned() * 100.0)
        );
        all.extend(results);
    }
    let found = verdict(&all);
    println!(
        "reads on against off, heads up: {} (seed {}, warm-up {warm})",
        line(&found),
        game.seed
    );
    if found.above_zero() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn line(found: &Verdict) -> String {
    format!(
        "{:+.0} mbb/100, 95% CI ±{:.0}, over {} deals",
        found.mbb_per_100, found.ci95, found.deals
    )
}

/// [`reads_on_off`] over `numbers`, spread over [`THREADS`]: thread `k` plays every
/// `THREADS`-th deal from the `k`-th, and the results go back in deal order, so the verdict is
/// the same on any number of threads.
fn measure(
    game: &Match,
    archetype: Archetype,
    numbers: std::ops::Range<u64>,
    on: &(impl Fn(&svanbot_core::spot::Spot, u64) -> svanbot_core::rules::Action + Sync),
    off: &(impl Fn(&svanbot_core::spot::Spot, u64) -> svanbot_core::rules::Action + Sync),
) -> Vec<f64> {
    let start = numbers.start;
    let mut results = vec![0.0; (numbers.end - start) as usize];
    let played: Vec<Vec<(u64, f64)>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..THREADS)
            .map(|k| {
                let numbers = numbers.clone();
                scope.spawn(move || {
                    (numbers.start + k..numbers.end)
                        .step_by(THREADS as usize)
                        .map(|deal| {
                            (
                                deal,
                                reads_on_off(game, archetype, deal..deal + 1, on, off)[0],
                            )
                        })
                        .collect()
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().expect("a reads sim worker panicked"))
            .collect()
    });
    for (deal, result) in played.into_iter().flatten() {
        results[(deal - start) as usize] = result;
    }
    results
}

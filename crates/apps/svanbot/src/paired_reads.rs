//! `svanbot paired reads <deals> [seed] [warm-up]`: what playing by an opponent's own read wins,
//! heads up, then exit (V11-25). It touches neither the venue nor a store.
//!
//! Against each fixed sparring archetype, a warm-up of `warm-up` deals (default 200) builds the
//! policy's read of the opponent with the live code, and freezes it. Then each of `deals` fresh
//! deals is played by the policy with the read and without, and the difference is the result. It
//! prints a line per archetype and one across the fixed archetypes. Against the adapting
//! archetypes it runs the exploit gate ([`gate`]) on what the reads lose to them. It exits 0
//! only when the fixed archetypes' interval is wholly above 0 and the exploit gate passes.

use std::process::ExitCode;

use svanbot_learn::exploit::gate;
use svanbot_learn::paired::{Match, Verdict, verdict};
use svanbot_learn::read_sim::{OPPONENT, reads_on_off, warm_up};
use svanbot_sim::sparring::Archetype;
use svanbot_strategy::params::Params;

use crate::deciders;

/// Threads the deals are spread over. The box is shared with the live fleet.
const THREADS: u64 = 4;

/// The archetypes that play one strategy throughout, whose reads should pay.
const FIXED: [Archetype; 3] = [Archetype::Station, Archetype::Nit, Archetype::Maniac];

/// The archetypes that change how they play, which punish a read that does not keep up: the
/// exploit gate's losses come from them.
const ADAPTING: [Archetype; 2] = [Archetype::Switching, Archetype::Learning];

/// The most reads may lose a hand to the adapting archetypes, in mbb, at the gate's 95% bound.
const CAP: f64 = 25.0;

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
    let play = |archetype: Archetype| {
        let reads = warm_up(&game, archetype, 0..warm, &off);
        let read = reads.get(OPPONENT);
        let on = deciders::reader(&params, None, &reads);
        let results = measure(&game, archetype, warm..warm + deals, &on, &off);
        println!(
            "reads on against off, {}: {}, read {} at {:.0}% of the budget",
            archetype.name(),
            line(&verdict(&results)),
            read.map_or("nothing", |read| read.class().name()),
            read.map_or(0.0, |read| read.earned() * 100.0)
        );
        results
    };
    let found = verdict(&mean_per_deal(FIXED.map(play)));
    println!(
        "reads on against off, heads up: {} (seed {}, warm-up {warm})",
        line(&found),
        game.seed
    );
    // The exploit meter: what reads lose, per hand, to the opponents that punish them.
    let losses: Vec<f64> = mean_per_deal(ADAPTING.map(play))
        .into_iter()
        .map(|won| -won * 1000.0)
        .collect();
    let meter = gate(&losses, CAP);
    println!(
        "exploit gate against the adapting archetypes: {:+.1} mbb a hand lost, upper bound {:+.1}, \
         cap {CAP}: {}",
        meter.mean,
        meter.upper,
        if meter.passed { "passed" } else { "refused" }
    );
    if found.above_zero() && meter.passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// Each archetype plays the same cards on a deal. Average the rows within each deal so the
/// interval uses independent deals and keeps the covariance between archetypes.
fn mean_per_deal<const N: usize>(rows: [Vec<f64>; N]) -> Vec<f64> {
    (0..rows[0].len())
        .map(|deal| rows.iter().map(|row| row[deal]).sum::<f64>() / N as f64)
        .collect()
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

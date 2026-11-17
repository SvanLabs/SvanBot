//! `svanbot arena`: the policy against every sparring archetype and the pricer, heads up, as a
//! scoreboard, then exit. It touches neither the venue nor a store.
//!
//! `svanbot arena <bouts> [seed] [iterations] [out.json]`: each bout is 50 deals, each
//! played twice from mirrored seats, with the luck of the cards taken out. With `iterations` the
//! re-solve runs that many on every spot; without, the compute budget sizes them by the pot, as
//! in live play. The scoreboard is printed, and written as JSON to `out.json` when one is named,
//! with the commit in `SVANBOT_COMMIT` and the seed, so the run can be played again.
//!
//! Every run is also recorded in the fleet's `arena.json` (`Fleet::record_arena`), so the
//! dashboard's scoreboard and the release gate read the same numbers this printed.

use std::process::ExitCode;

use svanbot_core::rules::{Action, Blinds};
use svanbot_core::spot::Spot;
use svanbot_learn::arena::{BOUT, Row, bout, json, scoreboard};
use svanbot_learn::paired::{Match, duel, verdict};
use svanbot_runtime::fleet::{ArenaRow, ArenaRun, Fleet};
use svanbot_sim::sparring::Archetype;
use svanbot_strategy::params::Params;
use svanbot_strategy::policy;
use svanbot_strategy::pricer::{self, Budget};
use svanbot_strategy::resolve::iterations_for;

/// Deals sampled by the pricer per decision in a sim.
const SAMPLES: u64 = 2_000;

/// Threads the bouts are spread over. The box is shared with the live fleet.
const THREADS: u64 = 4;

pub(crate) fn run(args: &[String]) -> ExitCode {
    let number = |at: usize| args.get(at).map(|text| text.parse::<u64>());
    let (Some(Ok(bouts)), seed, fixed) = (number(0), number(1), number(2)) else {
        eprintln!("svanbot arena: usage: svanbot arena <bouts> [seed] [iterations] [out.json]");
        return ExitCode::from(2);
    };
    let (Ok(seed), Ok(fixed)) = (seed.transpose(), fixed.transpose()) else {
        eprintln!("svanbot arena: the seed and the iterations are whole numbers");
        return ExitCode::from(2);
    };
    let game = Match {
        blinds: Blinds { small: 10, big: 20 },
        stack: 2000,
        seed: seed.unwrap_or(1),
    };
    let challenger = |spot: &Spot, seed: u64| -> Action {
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
    let pricer = |spot: &Spot, seed: u64| -> Action {
        let budget = Budget {
            samples: SAMPLES,
            seed,
        };
        pricer::decide(spot, None, budget).action
    };
    // `None` is the pricer; the rest are the archetypes.
    let opponents: Vec<Option<Archetype>> = std::iter::once(None)
        .chain(Archetype::ALL.map(Some))
        .collect();
    let mut rows = Vec::new();
    for opponent in opponents {
        // Thread `k` plays bouts k, k + THREADS, …; put back in order, the verdict is the
        // same on any number of threads.
        let played: Vec<Vec<(u64, Vec<f64>)>> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..THREADS)
                .map(|k| {
                    let (challenger, pricer) = (&challenger, &pricer);
                    scope.spawn(move || {
                        (k..bouts)
                            .step_by(THREADS as usize)
                            .map(|number| {
                                let results = match opponent {
                                    Some(archetype) => bout(&game, archetype, number, challenger),
                                    None => (number * BOUT..(number + 1) * BOUT)
                                        .map(|deal| duel(&game, deal, challenger, pricer))
                                        .collect(),
                                };
                                (number, results)
                            })
                            .collect()
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap_or_default())
                .collect()
        });
        let mut by_bout: Vec<(u64, Vec<f64>)> = played.into_iter().flatten().collect();
        by_bout.sort_by_key(|(number, _)| *number);
        let results: Vec<f64> = by_bout
            .into_iter()
            .flat_map(|(_, results)| results)
            .collect();
        let row = Row {
            opponent: opponent.map_or("pricer", Archetype::name).to_owned(),
            verdict: verdict(&results),
        };
        eprintln!("svanbot arena: {} done", row.opponent);
        rows.push(row);
    }
    print!("{}", scoreboard(&rows));
    let commit = std::env::var("SVANBOT_COMMIT").unwrap_or_else(|_| "unknown".into());
    let run = ArenaRun {
        commit: commit.clone(),
        seed: game.seed,
        rows: rows
            .iter()
            .map(|row| ArenaRow {
                opponent: row.opponent.clone(),
                deals: row.verdict.deals,
                mbb_per_100: row.verdict.mbb_per_100,
                ci95: row.verdict.ci95,
            })
            .collect(),
    };
    if let Err(error) = Fleet::in_dir(crate::fleet_dir()).record_arena(&run) {
        eprintln!("svanbot arena: the run was not recorded: {error}");
        return ExitCode::from(1);
    }
    if let Some(path) = args.get(3)
        && let Err(error) = std::fs::write(path, json(&rows, &commit, game.seed) + "\n")
    {
        eprintln!("svanbot arena: {path} was not written: {error}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

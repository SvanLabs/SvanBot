//! `svanbot arena`: the policy against every sparring archetype and the pricer, as a scoreboard,
//! then exit. It touches neither the venue nor a store.
//!
//! `svanbot arena [6-max] <bouts> [seed] [iterations] [out.json]`: each bout is 50 deals. Heads
//! up, each deal is played twice from mirrored seats; with `6-max` it is played six times, the
//! policy in each seat of a full table against five of the opponent (`learn::ring`). Either way
//! the luck of the cards is taken out. The two leagues are recorded apart and never pooled.
//! With `iterations` the re-solve runs that many on every spot; without, the compute budget sizes
//! them by the pot, as in live play. The scoreboard is printed, and written as JSON to `out.json` when one is named,
//! with the commit in `SVANBOT_COMMIT` and the seed, so the run can be played again.
//!
//! Every run is also recorded in the fleet's `arena.json` (`Fleet::record_arena`), so the
//! dashboard's scoreboard and the release gate read the same numbers this printed.

use std::process::ExitCode;

use svanbot_learn::arena::{BOUT, Row, bout, ring_bout, scoreboard};
use svanbot_learn::paired::{duel, verdict};
use svanbot_learn::ring::{SIX_MAX, ring};
use svanbot_runtime::fleet::{ArenaRow, ArenaRun, Fleet, League};
use svanbot_sim::sparring::Archetype;

use crate::deciders;

/// Threads the bouts are spread over. The box is shared with the live fleet.
const THREADS: u64 = 4;

pub(crate) fn run(args: &[String]) -> ExitCode {
    let (league, args) = match args.first().map(String::as_str) {
        Some("6-max") => (League::SixMax, &args[1..]),
        _ => (League::HeadsUp, args),
    };
    let number = |at: usize| args.get(at).map(|text| text.parse::<u64>());
    let (Some(Ok(bouts)), seed, fixed) = (number(0), number(1), number(2)) else {
        eprintln!(
            "svanbot arena: usage: svanbot arena [6-max] <bouts> [seed] [iterations] [out.json]"
        );
        return ExitCode::from(2);
    };
    let (Ok(seed), Ok(fixed)) = (seed.transpose(), fixed.transpose()) else {
        eprintln!("svanbot arena: the seed and the iterations are whole numbers");
        return ExitCode::from(2);
    };
    // The champion is what the fleet plays by (`Fleet::champion`), which is the default until a
    // promotion: an Arena that always played the default would not move when a set is promoted.
    let fleet = Fleet::in_dir(crate::fleet_dir());
    let champion = match fleet.champion() {
        Ok(champion) => champion,
        Err(why) => {
            eprintln!("svanbot arena: the champion file is refused: {why}");
            return ExitCode::from(2);
        }
    };
    let game = deciders::game(seed.unwrap_or(1));
    let challenger = deciders::challenger(&champion, fixed);
    let pricer = deciders::pricer();
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
                                let deals = number * BOUT..(number + 1) * BOUT;
                                let results = match (league, opponent) {
                                    (League::HeadsUp, Some(archetype)) => {
                                        bout(&game, archetype, number, challenger)
                                    }
                                    (League::HeadsUp, None) => deals
                                        .map(|deal| duel(&game, deal, challenger, pricer))
                                        .collect(),
                                    (League::SixMax, Some(archetype)) => {
                                        ring_bout(&game, archetype, number, challenger)
                                    }
                                    (League::SixMax, None) => deals
                                        .map(|deal| {
                                            ring(
                                                &game,
                                                SIX_MAX,
                                                deal,
                                                challenger,
                                                &|_, spot, seed| pricer(spot, seed),
                                            )
                                        })
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
                .map(|worker| {
                    // A panicked worker is a run that would be recorded short of its deals: stop
                    // here, rather than store numbers it cannot be played again from.
                    worker.join().expect("an Arena worker panicked")
                })
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
    println!("{} league", league.name());
    print!("{}", scoreboard(&rows));
    // A local run has no commit, and is kept as such: `record_arena` tells two of them apart by
    // their own numbers, since there is no commit to play them again from.
    let commit = std::env::var("SVANBOT_COMMIT").unwrap_or_else(|_| ArenaRun::LOCAL_COMMIT.into());
    let run = ArenaRun {
        commit: commit.clone(),
        champion: champion.digest().to_string(),
        seed: game.seed,
        league,
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
        && let Err(error) = std::fs::write(path, run.as_text() + "\n")
    {
        eprintln!("svanbot arena: {path} was not written: {error}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

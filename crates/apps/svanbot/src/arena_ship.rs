//! `svanbot arena ship [seed] [iterations]`: the ship bar's Arena stage, played until it is
//! decided (V11-40). Both leagues against every opponent, a heat at a time, read
//! through the confidence sequence after each heat (`svanbot_runtime::ship::sample`). It stops
//! when the bar passes, when any row is read below zero, or at the cap of
//! [`CAP_HOURS`](svanbot_runtime::ship::CAP_HOURS) hours. Each heat records both leagues in
//! the fleet's `arena.json`, so a run cut short keeps what it played and the dashboard follows it.
//!
//! It plays from the commit in `SVANBOT_COMMIT`, which it needs: `release-check` reads only runs
//! from the tag's commit, so a run without one could never clear the bar. Run it from a build of
//! that commit, at low priority, beside the fleet (`deploy/README.md`). Exits 0 when the bar
//! passes and 1 when it is refused.

use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use svanbot_runtime::fleet::{ArenaRow, ArenaRun, Fleet, League};
use svanbot_runtime::ship::{self, Bar, Bout, CAP_HOURS};

use svanbot_strategy::policy::Policy;

use crate::arena::{Opponent, THREADS, play_bout};
use crate::deciders;

pub(crate) fn run(args: &[String]) -> ExitCode {
    let number = |at: usize| args.get(at).map(|text| text.parse::<u64>()).transpose();
    let (Ok(seed), Ok(fixed)) = (number(0), number(1)) else {
        eprintln!("svanbot arena ship: usage: svanbot arena ship [seed] [iterations]");
        return ExitCode::from(2);
    };
    let Ok(commit) = std::env::var("SVANBOT_COMMIT") else {
        eprintln!("svanbot arena ship: set SVANBOT_COMMIT to the commit the build was made from");
        return ExitCode::from(2);
    };
    let fleet = Fleet::in_dir(crate::fleet_dir());
    let champion = match fleet.champion() {
        Ok(champion) => champion,
        Err(why) => {
            eprintln!("svanbot arena ship: the champion file is refused: {why}");
            return ExitCode::from(2);
        }
    };
    let game = deciders::game(seed.unwrap_or(1));
    let fresh = |league| ArenaRun {
        commit: commit.clone(),
        champion: Some(champion.digest().to_string()),
        seed: game.seed,
        league,
        iterations: fixed,
        rows: Opponent::all()
            .into_iter()
            .map(|opponent| ArenaRow {
                opponent: opponent.name(),
                deals: 0,
                mbb_per_100: 0.0,
                ci95: 0.0,
                bouts: Vec::new(),
            })
            .collect(),
    };
    let mut runs = vec![fresh(League::HeadsUp), fresh(League::SixMax)];
    let started = Instant::now();
    let cap = Duration::from_secs(CAP_HOURS * 3600);
    let mut heats = 0;
    let mut play = |runs: &[ArenaRun], bouts: &[Bout]| {
        heats += 1;
        let won = heat(runs, bouts, |run, opponent, number| {
            play_bout(
                &game,
                run.league,
                opponent,
                number,
                &Policy::reference(champion),
                fixed,
            )
            .iter()
            .sum()
        });
        eprintln!(
            "svanbot arena ship: heat {heats} played, {:.1} h in",
            started.elapsed().as_secs_f64() / 3600.0
        );
        won
    };
    let mut record = |run: &ArenaRun| {
        if let Err(error) = fleet.record_arena(run) {
            eprintln!("svanbot arena ship: the heat was not recorded: {error}");
        }
    };
    let judged = ship::sample(
        &mut runs,
        &mut play,
        &mut || started.elapsed() >= cap,
        &mut record,
    );
    match judged {
        Bar::Clears => {
            println!("the Arena stage's bar passes at {commit}: run `svanbot release-check <tag>`");
            ExitCode::SUCCESS
        }
        Bar::Refused(why) => {
            for why in why {
                println!("refused: {why}");
            }
            ExitCode::from(1)
        }
        Bar::Open => unreachable!("a ship run always decides"),
    }
}

/// Plays a heat's bouts over [`THREADS`] threads, each taking the next bout not yet taken, so a
/// costly 6-max bout does not hold the others back. Each bout stands alone, so what it won does
/// not depend on which thread played it; the answers come back in the heat's order.
fn heat(
    runs: &[ArenaRun],
    bouts: &[Bout],
    play: impl Fn(&ArenaRun, Opponent, u64) -> f64 + Sync,
) -> Vec<f64> {
    let next = AtomicUsize::new(0);
    let mut won = vec![f64::NAN; bouts.len()];
    let played: Vec<(usize, f64)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..THREADS)
            .map(|_| {
                scope.spawn(|| {
                    let mut played = Vec::new();
                    loop {
                        let at = next.fetch_add(1, Ordering::Relaxed);
                        let Some(bout) = bouts.get(at) else {
                            return played;
                        };
                        let run = &runs[bout.run];
                        let opponent = Opponent::named(&run.rows[bout.row].opponent)
                            .expect("a ship run's rows are the Arena's opponents");
                        played.push((at, play(run, opponent, bout.number)));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("an Arena worker panicked"))
            .collect()
    });
    for (at, sum) in played {
        won[at] = sum;
    }
    won
}

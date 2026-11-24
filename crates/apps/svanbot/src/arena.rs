//! `svanbot arena`: the policy against the pricer, v10.0.2 and every sparring archetype, as a scoreboard,
//! then exit. It touches neither the venue nor a store.
//!
//! `svanbot arena ship [seed] [iterations]` plays both leagues until the ship bar is decided
//! instead of for a fixed count ([`crate::arena_ship`]).
//!
//! `svanbot arena [league] <bouts> [seed] [iterations] [out.json]`: each bout is 50 deals. The
//! league is `3-handed`, `4-handed`, `5-handed` or `6-max`, and heads up without one. Heads up,
//! each deal is played twice from mirrored seats; at a bigger table it is played once from every
//! seat, the policy against the opponent in all the others (`learn::ring`). Either way the luck
//! of the cards is taken out. Each league is recorded apart and never pooled.
//! With `iterations` the re-solve runs that many on every spot; without, the compute budget sizes
//! them by the pot, as in live play. The scoreboard is printed, and written as JSON to `out.json`
//! when one is named, with the commit in `SVANBOT_COMMIT` and the seed, so the run can be played
//! again.
//!
//! Every run is also recorded in the fleet's `arena.json` (`Fleet::record_arena`), with what each
//! bout won, so the dashboard's scoreboard and the ship bar read the same numbers this
//! printed, and the ship bar can play a bout again to check them ([`play_bout`]).

use std::process::ExitCode;

use svanbot_learn::arena::{
    BOUT, BoutReader, Row, bout, ring_bout_at, scoreboard, v10_duel, v10_ring_at,
};
use svanbot_learn::paired::{Match, duel_seen, verdict};
use svanbot_learn::reads::Reads;
use svanbot_learn::ring::ring_seen;
use svanbot_runtime::fleet::{ArenaRow, ArenaRun, Fleet, League};
use svanbot_sim::sparring::Archetype;
use svanbot_strategy::params::Params;

use crate::deciders;

/// Threads the bouts are spread over. The box is shared with the live fleet.
pub(crate) const THREADS: u64 = 4;

/// A row of the scoreboard: who the policy plays.
#[derive(Clone, Copy)]
pub(crate) enum Opponent {
    /// The pricer the strategy layer falls back to.
    Pricer,
    /// v10.0.2, ported (`sim::v10`): the bar a release must clear.
    V10,
    Sparring(Archetype),
}

impl Opponent {
    /// Every row, in the scoreboard's order.
    pub(crate) fn all() -> Vec<Self> {
        [Self::Pricer, Self::V10]
            .into_iter()
            .chain(Archetype::ALL.map(Self::Sparring))
            .collect()
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Pricer => "pricer",
            Self::V10 => svanbot_sim::v10::seat::NAME,
            Self::Sparring(archetype) => archetype.name(),
        }
    }

    /// The opponent a scoreboard row names.
    pub(crate) fn named(name: &str) -> Option<Self> {
        Self::all()
            .into_iter()
            .find(|opponent| opponent.name() == name)
    }
}

/// Plays bout `number` of `league` against `opponent`: what the challenger won on each deal, in
/// big blinds a hand. A bout stands alone, so it plays the same here, on any thread, or later in
/// the ship bar.
pub(crate) fn play_bout(
    game: &Match,
    league: League,
    opponent: Opponent,
    number: u64,
    params: &Params,
    fixed: Option<u64>,
) -> Vec<f64> {
    let seats = league.seats();
    let challenger = &BoutReader::new(deciders::reading_policy(
        params,
        fixed,
        seats,
        Reads::deciding,
    ));
    let pricer = deciders::pricer();
    let deals = number * BOUT..(number + 1) * BOUT;
    // Heads up is the mirrored duel; every bigger table is the ring at its own size.
    match (league, opponent) {
        (League::HeadsUp, Opponent::Sparring(archetype)) => {
            bout(game, archetype, number, challenger)
        }
        (League::HeadsUp, Opponent::V10) => {
            deals.map(|deal| v10_duel(game, deal, challenger)).collect()
        }
        (League::HeadsUp, Opponent::Pricer) => deals
            .map(|deal| {
                duel_seen(
                    game,
                    deal,
                    challenger,
                    &|_, spot, _, seed| pricer(spot, seed),
                    || {},
                )
            })
            .collect(),
        (_, Opponent::V10) => deals
            .map(|deal| v10_ring_at(game, seats, deal, challenger))
            .collect(),
        (_, Opponent::Sparring(archetype)) => {
            ring_bout_at(game, seats, archetype, number, challenger)
        }
        (_, Opponent::Pricer) => deals
            .map(|deal| {
                ring_seen(
                    game,
                    seats,
                    deal,
                    challenger,
                    &|_, spot, _, seed| pricer(spot, seed),
                    |_| {},
                )
            })
            .collect(),
    }
}

pub(crate) fn run(args: &[String]) -> ExitCode {
    let (league, args) = match args.first().map(String::as_str) {
        Some("ship") => return crate::arena_ship::run(&args[1..]),
        Some(name) if League::named(name).is_some() => {
            (League::named(name).unwrap_or(League::HeadsUp), &args[1..])
        }
        _ => (League::HeadsUp, args),
    };
    let number = |at: usize| args.get(at).map(|text| text.parse::<u64>());
    let (Some(Ok(bouts)), seed, fixed) = (number(0), number(1), number(2)) else {
        eprintln!(
            "svanbot arena: usage: svanbot arena [3-handed|4-handed|5-handed|6-max] <bouts> [seed] [iterations] [out.json]"
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
    let mut rows = Vec::new();
    for opponent in Opponent::all() {
        // Thread `k` plays bouts k, k + THREADS, …; put back in order, the verdict is the
        // same on any number of threads.
        let played: Vec<Vec<(u64, Vec<f64>)>> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..THREADS)
                .map(|k| {
                    let (game, champion) = (&game, &champion);
                    scope.spawn(move || {
                        (k..bouts)
                            .step_by(THREADS as usize)
                            .map(|number| {
                                (
                                    number,
                                    play_bout(game, league, opponent, number, champion, fixed),
                                )
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
        // Summed in deal order, the way the ship bar sums a bout it plays again.
        let sums: Vec<f64> = by_bout
            .iter()
            .map(|(_, results)| results.iter().sum())
            .collect();
        let results: Vec<f64> = by_bout
            .into_iter()
            .flat_map(|(_, results)| results)
            .collect();
        let row = Row {
            opponent: opponent.name().to_owned(),
            verdict: verdict(&results),
        };
        eprintln!("svanbot arena: {} done", row.opponent);
        rows.push((row, sums));
    }
    let board: Vec<Row> = rows.iter().map(|(row, _)| row.clone()).collect();
    println!("{} league", league.name());
    print!("{}", scoreboard(&board));
    // A local run has no commit, and is kept as such: `record_arena` tells two of them apart by
    // their own numbers, since there is no commit to play them again from.
    let commit = std::env::var("SVANBOT_COMMIT").unwrap_or_else(|_| ArenaRun::LOCAL_COMMIT.into());
    let run = ArenaRun {
        commit,
        champion: Some(champion.digest().to_string()),
        seed: game.seed,
        league,
        iterations: fixed,
        rows: rows
            .into_iter()
            .map(|(row, bouts)| ArenaRow {
                opponent: row.opponent,
                deals: row.verdict.deals,
                mbb_per_100: row.verdict.mbb_per_100,
                ci95: row.verdict.ci95,
                bouts,
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

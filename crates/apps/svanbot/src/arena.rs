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
use std::sync::{Mutex, mpsc};

use svanbot_learn::arena::{
    BOUT, BoutReader, Row, bout, ring_bout_at, scoreboard, v10_duel, v10_ring_at,
};
use svanbot_learn::field::Field;
use svanbot_learn::paired::{Match, duel_seen, verdict};
use svanbot_learn::play::{self, STABLEMATE};
use svanbot_learn::reads::Reads;
use svanbot_learn::ring::ring_seen;
use svanbot_runtime::fleet::{ArenaRow, ArenaRun, Fleet, League};
use svanbot_sim::sparring::Archetype;
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::policy::Policy;
use svanbot_strategy::style::Style;

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
    Stablemate(&'static Style),
}

impl Opponent {
    /// Every row, in the scoreboard's order.
    pub(crate) fn all() -> Vec<Self> {
        [Self::Pricer, Self::V10]
            .into_iter()
            .chain(Archetype::ALL.map(Self::Sparring))
            .collect()
    }

    pub(crate) fn name(self) -> String {
        match self {
            Self::Pricer => "pricer".into(),
            Self::V10 => svanbot_sim::v10::seat::NAME.into(),
            Self::Sparring(archetype) => archetype.name().into(),
            Self::Stablemate(style) => format!("{STABLEMATE}{}", style.name),
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
    policy: &Policy,
    fixed: Option<u64>,
) -> Vec<f64> {
    let seats = league.seats();
    let challenger = &BoutReader::new(play::reading(*policy, fixed, seats, Reads::deciding, None))
        .with_profile_stats(!policy.style.profile.is_empty());
    let pricer = deciders::pricer();
    let deals = number * BOUT..(number + 1) * BOUT;
    // Heads up is the mirrored duel; every bigger table is the ring at its own size.
    match (league, opponent) {
        (_, Opponent::Stablemate(style)) => {
            let rival = play::player(
                Policy::new(PolicyKind::Solving, style, policy.params),
                fixed,
            );
            deals
                .map(|deal| match league {
                    League::HeadsUp => play::rival_duel(game, deal, challenger, &rival),
                    _ => play::rival_ring_at(game, seats, deal, challenger, &rival),
                })
                .collect()
        }

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

/// Plays bout `number` of `league` against a bot's own field: the fitted population model in
/// every other seat. Returns what the challenger won on each deal, in big blinds a hand.
pub(crate) fn play_field_bout(
    game: &Match,
    league: League,
    field: &Field,
    number: u64,
    policy: &Policy,
    fixed: Option<u64>,
) -> Vec<f64> {
    let seats = league.seats();
    let challenger = &BoutReader::new(play::reading(*policy, fixed, seats, Reads::deciding, None))
        .with_profile_stats(!policy.style.profile.is_empty());
    let deals = number * BOUT..(number + 1) * BOUT;
    match league {
        League::HeadsUp => deals
            .map(|deal| {
                duel_seen(
                    game,
                    deal,
                    challenger,
                    &|_, spot, _, seed| field.decide(spot, seed),
                    || {},
                )
            })
            .collect(),
        _ => deals
            .map(|deal| {
                ring_seen(
                    game,
                    seats,
                    deal,
                    challenger,
                    &|_, spot, _, seed| field.decide(spot, seed),
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
    let reference = Policy::reference(champion);
    let rows = play_rows(
        &Opponent::all(),
        Opponent::name,
        bouts,
        |opponent, number| play_bout(&game, league, opponent, number, &reference, fixed),
    );
    let run = report(
        league,
        champion.digest().to_string(),
        game.seed,
        fixed,
        rows,
    );
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

pub(crate) fn play_rows<R: Copy + Send + Sync>(
    opponents: &[R],
    name: impl Fn(R) -> String,
    bouts: u64,
    play: impl Fn(R, u64) -> Vec<f64> + Sync,
) -> Vec<(Row, Vec<f64>)> {
    // Any idle worker can start the next row while a slow bout is still finishing.
    let pending = Mutex::new(
        (0..opponents.len()).flat_map(|row| (0..bouts).map(move |number| (row, number))),
    );
    let (send, receive) = mpsc::sync_channel(THREADS as usize);
    let mut by_row = vec![Vec::new(); opponents.len()];
    let mut finished = vec![None; opponents.len()];
    std::thread::scope(|scope| {
        for _ in 0..THREADS {
            let (send, pending, play) = (send.clone(), &pending, &play);
            scope.spawn(move || {
                loop {
                    let next = pending.lock().expect("Arena work queue poisoned").next();
                    let Some((row, number)) = next else {
                        return;
                    };
                    send.send((row, number, play(opponents[row], number)))
                        .expect("Arena result receiver closed");
                }
            });
        }
        drop(send);
        for (row, number, results) in receive {
            let deals = &mut by_row[row];
            deals.push((number, results));
            if deals.len() as u64 == bouts {
                finished[row] = Some(score_row(name(opponents[row]), std::mem::take(deals)));
            }
        }
    });
    opponents
        .iter()
        .zip(finished)
        .map(|(&opponent, row)| {
            row.unwrap_or_else(|| {
                assert_eq!(bouts, 0, "Arena finished without a row");
                score_row(name(opponent), Vec::new())
            })
        })
        .collect()
}

fn score_row(opponent: String, mut by_bout: Vec<(u64, Vec<f64>)>) -> (Row, Vec<f64>) {
    by_bout.sort_unstable_by_key(|(number, _)| *number);
    // Sum and score in the original bout and deal order so the recorded run replays exactly.
    let sums = by_bout
        .iter()
        .map(|(_, results)| results.iter().sum())
        .collect();
    let results: Vec<_> = by_bout
        .into_iter()
        .flat_map(|(_, results)| results)
        .collect();
    eprintln!("svanbot arena: {opponent} done");
    let row = Row {
        opponent,
        verdict: verdict(&results),
    };
    (row, sums)
}

pub(crate) fn report(
    league: League,
    champion: String,
    seed: u64,
    fixed: Option<u64>,
    rows: Vec<(Row, Vec<f64>)>,
) -> ArenaRun {
    let board: Vec<Row> = rows.iter().map(|(row, _)| row.clone()).collect();
    println!("{} league", league.name());
    print!("{}", scoreboard(&board));
    measurement(league, champion, seed, fixed, rows)
}

pub(crate) fn measurement(
    league: League,
    champion: String,
    seed: u64,
    fixed: Option<u64>,
    rows: Vec<(Row, Vec<f64>)>,
) -> ArenaRun {
    // A local run has no commit, and is kept as such: `record_arena` tells two of them apart by
    // their own numbers, since there is no commit to play them again from.
    let commit = std::env::var("SVANBOT_COMMIT").unwrap_or_else(|_| ArenaRun::LOCAL_COMMIT.into());
    ArenaRun {
        commit,
        champion: Some(champion),
        seed,
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
    }
}

#[cfg(test)]
mod tests {
    use super::{Opponent, play_rows};

    #[test]
    fn a_field_row_is_not_an_arena_opponent() {
        let name = svanbot_learn::field::row_name("SurSvan");
        assert!(
            Opponent::named(&name).is_none(),
            "a field row stays out of Opponent::all()"
        );
    }

    #[test]
    fn short_rows_share_workers_and_keep_scoreboard_order() {
        use std::sync::{Mutex, mpsc};
        use std::time::Duration;

        let (ready, called) = mpsc::channel();
        let called = Mutex::new(called);
        let rows = play_rows(&["first", "second"], str::to_owned, 2, |opponent, bout| {
            if opponent == "first" && bout == 0 {
                called
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap();
                vec![1.0, 2.0]
            } else {
                if opponent == "second" && bout == 0 {
                    ready.send(()).unwrap();
                }
                vec![3.0 + bout as f64, 4.0]
            }
        });
        assert_eq!(rows[0].0.opponent, "first");
        assert_eq!(rows[0].1, [3.0, 8.0]);
        assert_eq!(rows[1].0.opponent, "second");
        assert_eq!(rows[1].1, [7.0, 8.0]);
    }
}

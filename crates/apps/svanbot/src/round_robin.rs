//! `svanbot round-robin`: the standing test of each bot's gauntlet, one bounded pass, at low
//! priority under the fleet's supervisor.
//!
//! A pass plays every assigned bot's gauntlet in order: the ten shared opponents heads up, the
//! same ten at the 6-max ring, then a 6-max row against each champion in that bot's archive
//! (`learn::round_robin`). The rows play one after another, never in parallel, and the pass stops
//! at its row budget. Each row's paired bouts are read through `learn::probe::Sequence`, and the
//! archive rows are the evidence `learn::archive::regression` reads for a promotion.
//!
//! The pass is recorded in the fleet's own `round-robin.json`, with every bot's matrix, the
//! archive it played, and the regression verdict. The matrix is standing: each pass appends a
//! row's new bouts to its evidence (up to `learn::round_robin::KEEP_BOUTS`), and a champion change
//! starts every row afresh. The release Arena's `arena.json` is never written: this record is read
//! by the round-robin itself, the regression rule and the dashboard, never by the ship bar.
//!
//! `svanbot round-robin [bouts] [seed] [iterations]`; `SVANBOT_ROUND_ROBIN_ROWS` shrinks or grows
//! the pass's row budget. Each assigned bot plays as its assignment, with its own champion slot
//! (`Fleet::champion_for`); a bot with no assignment has no line until it is assigned.

use std::process::ExitCode;

use svanbot_learn::archive::{Archive, Archived};
use svanbot_learn::arena::{BOUT, BoutReader};
use svanbot_learn::paired::Match;
use svanbot_learn::play;
use svanbot_learn::reads::Reads;
use svanbot_learn::round_robin::{self, Budget, MBB_PER_100, Row, Table};
use svanbot_runtime::fleet::{Fleet, League};
use svanbot_runtime::round_robin::{ArchivedChampion, BotRecord, KEEP, Record, RowRecord};
use svanbot_strategy::params::Params;
use svanbot_strategy::policy::Policy;

use crate::arena::{self, Opponent};
use crate::deciders;

/// The bouts one row plays unless the caller names another count.
const DEFAULT_BOUTS: u64 = 1;

/// The re-solve iterations one row runs unless the caller names another count: the pass is a
/// standing test, bounded enough to run beside the live fleet.
const DEFAULT_ITERATIONS: u64 = 2;

/// One pass's settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Settings {
    bouts: u64,
    seed: u64,
    iterations: Option<u64>,
}

/// One bot's gauntlet, ready to play.
struct Gauntlet {
    bot: String,
    policy: Policy,
    champion: Params,
    /// The bot's archived champions, newest first, with their numbers.
    archive: Vec<ArchivedChampion>,
    /// The archive the plan was made from: the same champions, minus the current one.
    rules: Archive,
    plan: round_robin::Plan,
}

/// How a pass plays one row: the row's paired bouts in mbb/100, or why it could not be played.
type Play<'a> = dyn FnMut(&Gauntlet, &Row) -> Result<Vec<f64>, String> + 'a;

/// Plays one pass and records it. `play` supplies a row's paired bouts in mbb/100; the app's sim
/// does the playing, and a test can stand in for it.
fn pass(
    fleet: &Fleet,
    settings: Settings,
    budget: usize,
    previous: Option<&Record>,
    play: &mut Play<'_>,
) -> Result<Record, String> {
    let gauntlets = gauntlets(fleet, previous)?;
    let planned: usize = gauntlets
        .iter()
        .map(|gauntlet| gauntlet.plan.rows.len())
        .sum();
    let mut played = 0;
    let mut bots = Vec::with_capacity(gauntlets.len());
    for gauntlet in &gauntlets {
        let remaining = budget.saturating_sub(played);
        let mut failure = None;
        let run = round_robin::run(
            &gauntlet.plan,
            Budget { rows: remaining },
            |row| match play(gauntlet, row) {
                Ok(bouts) => bouts,
                Err(why) => {
                    failure = Some(why);
                    Vec::new()
                }
            },
        );
        if let Some(why) = failure {
            return Err(why);
        }
        played += run.rows.len();
        let standing = standing(gauntlet, previous, &run.rows);
        let rows = standing
            .iter()
            .map(|scored| RowRecord {
                row: scored.row.name.clone(),
                table: scored.row.table,
                archive: scored.row.archive,
                bouts: scored.bouts.clone(),
                mean: scored.reading.mean,
                radius: scored
                    .reading
                    .radius
                    .is_finite()
                    .then_some(scored.reading.radius),
            })
            .collect();
        bots.push(BotRecord {
            bot: gauntlet.bot.clone(),
            policy: gauntlet.policy.name(),
            champion: gauntlet.champion,
            archive: gauntlet.archive.clone(),
            rows,
            regression: round_robin::regression(&gauntlet.rules, &standing),
        });
    }
    Ok(Record {
        recorded_ms: svanbot::now_ms(),
        seed: settings.seed,
        bouts: settings.bouts,
        iterations: settings.iterations,
        budget,
        played,
        skipped: planned - played,
        bots,
    })
}

/// Every assigned bot's gauntlet, in bot-name order. A bot whose champion changed since the
/// previous pass archives the champion it had, so the regression rule can measure the new one
/// against it. The archive keeps five, newest first (V11-96).
fn gauntlets(fleet: &Fleet, previous: Option<&Record>) -> Result<Vec<Gauntlet>, String> {
    let shared: Vec<String> = Opponent::shared().into_iter().map(Opponent::name).collect();
    let mut gauntlets = Vec::new();
    for (bot, assignment, _) in fleet.assignments()? {
        let champion = fleet.champion_for(&bot)?;
        let policy = Policy::new(assignment.kind, assignment.style, champion);
        let mut archive = previous
            .and_then(|record| record.bot(&bot))
            .map(|held| held.archive.clone())
            .unwrap_or_default();
        if let Some(held) = previous.and_then(|record| record.bot(&bot))
            && held.champion != champion
        {
            remember(&mut archive, held.champion);
        }
        // The current champion is not an opponent to itself.
        let rules = rules(&archive, champion.digest());
        let plan = round_robin::plan(&bot, champion.digest(), &shared, &rules);
        gauntlets.push(Gauntlet {
            bot,
            policy,
            champion,
            archive,
            rules,
            plan,
        });
    }
    Ok(gauntlets)
}

/// The whole plan as a standing matrix: every row's previous evidence (while the champion is the
/// same) with this pass's new bouts appended, the newest [`round_robin::KEEP_BOUTS`] kept. A row
/// the budget left unplayed keeps its previous evidence. A champion change starts every row
/// afresh: evidence from another policy is not evidence about this one.
fn standing(
    gauntlet: &Gauntlet,
    previous: Option<&Record>,
    played: &[round_robin::Scored],
) -> Vec<round_robin::Scored> {
    let held = previous.and_then(|record| record.bot(&gauntlet.bot));
    let carry = held.is_some_and(|held| {
        held.champion == gauntlet.champion && held.policy == gauntlet.policy.name()
    });
    gauntlet
        .plan
        .rows
        .iter()
        .map(|row| {
            let new = played
                .iter()
                .find(|scored| scored.row == *row)
                .map(|scored| scored.bouts.as_slice())
                .unwrap_or_default();
            let mut bouts = if carry {
                held.and_then(|held| held.rows.iter().find(|held| same(held, row)))
                    .map(|held| held.bouts.clone())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            bouts.extend_from_slice(new);
            if bouts.len() > round_robin::KEEP_BOUTS {
                bouts.drain(..bouts.len() - round_robin::KEEP_BOUTS);
            }
            round_robin::Scored {
                row: row.clone(),
                reading: round_robin::read(&bouts),
                bouts,
            }
        })
        .collect()
}

/// Whether a record row is `row`'s line: same name, table and archived champion.
fn same(held: &RowRecord, row: &Row) -> bool {
    held.row == row.name && held.table == row.table && held.archive == row.archive
}

/// `archive` with `params` as its newest champion, five kept, one entry a digest.
fn remember(archive: &mut Vec<ArchivedChampion>, params: Params) {
    let digest = params.digest();
    archive.retain(|held| held.digest() != digest);
    archive.insert(0, ArchivedChampion { params });
    archive.truncate(KEEP);
}

/// The archive the plan and the regression rule read: every held champion except `current`,
/// oldest first so `Archive::record` leaves it newest first.
fn rules(archive: &[ArchivedChampion], current: svanbot_strategy::params::Digest) -> Archive {
    let mut rules = Archive::new();
    for held in archive.iter().filter(|held| held.digest() != current).rev() {
        rules.record(Archived {
            digest: held.digest(),
            promotion: Vec::new(),
        });
    }
    rules
}

/// Plays one row: `settings.bouts` paired bouts, each as mbb/100. A shared row meets the named
/// Arena opponent; an archive row meets the archived champion's numbers in every other seat.
fn play_row(
    gauntlet: &Gauntlet,
    row: &Row,
    game: &Match,
    settings: Settings,
) -> Result<Vec<f64>, String> {
    let league = league(row.table);
    (0..settings.bouts)
        .map(|number| {
            let results = match row.archive {
                Some(champion) => {
                    let held = gauntlet
                        .archive
                        .iter()
                        .find(|held| held.digest() == champion)
                        .ok_or_else(|| {
                            format!("archive row {champion} has no archived champion")
                        })?;
                    play_archived(
                        game,
                        league,
                        &gauntlet.policy,
                        held.params,
                        number,
                        settings.iterations,
                    )
                }
                None => {
                    let opponent = Opponent::named(&row.name)
                        .ok_or_else(|| format!("no shared opponent is called {}", row.name))?;
                    arena::play_bout(
                        game,
                        league,
                        opponent,
                        number,
                        &gauntlet.policy,
                        settings.iterations,
                    )
                }
            };
            let sum: f64 = results.iter().sum();
            Ok(sum / BOUT as f64 * MBB_PER_100)
        })
        .collect()
}

/// Plays bout `number` of `league` against a rival with the archived champion's numbers, in the
/// bot's own kind and style.
fn play_archived(
    game: &Match,
    league: League,
    policy: &Policy,
    rival: Params,
    number: u64,
    fixed: Option<u64>,
) -> Vec<f64> {
    let seats = league.seats();
    let challenger = BoutReader::new(play::reading(*policy, fixed, seats, Reads::deciding, None))
        .with_profile_stats(!policy.style.profile.is_empty());
    let rival = play::player(Policy::new(policy.kind, policy.style, rival), fixed);
    let deals = number * BOUT..(number + 1) * BOUT;
    deals
        .map(|deal| match league {
            League::HeadsUp => play::rival_duel(game, deal, &challenger, &rival),
            _ => play::rival_ring_at(game, seats, deal, &challenger, &rival),
        })
        .collect()
}

fn league(table: Table) -> League {
    match table {
        Table::HeadsUp => League::HeadsUp,
        Table::SixMax => League::SixMax,
    }
}

/// The pass's row budget: `SVANBOT_ROUND_ROBIN_ROWS`, or the plan's own bound.
fn budget() -> Result<usize, String> {
    match std::env::var("SVANBOT_ROUND_ROBIN_ROWS") {
        Ok(text) => text
            .parse()
            .map_err(|_| "SVANBOT_ROUND_ROBIN_ROWS is not a count".to_owned()),
        Err(_) => Ok(round_robin::MAX_ROWS),
    }
}

/// The pass's settings from `[bouts] [seed] [iterations]`, with the bounded defaults a bare
/// supervisor run uses.
fn settings(args: &[String]) -> Result<Settings, String> {
    let number = |at: usize| args.get(at).map(|text| text.parse::<u64>());
    let bouts = match number(0) {
        None => DEFAULT_BOUTS,
        Some(Ok(bouts @ 1..)) => bouts,
        Some(_) => return Err("bouts must be a whole number of at least one".into()),
    };
    let seed = match number(1) {
        None => 1,
        Some(Ok(seed)) => seed,
        Some(_) => return Err("the seed must be a whole number".into()),
    };
    let iterations = match number(2) {
        None => Some(DEFAULT_ITERATIONS),
        Some(Ok(iterations)) => Some(iterations),
        Some(_) => return Err("the iterations must be a whole number".into()),
    };
    if args.len() > 3 {
        return Err("usage: svanbot round-robin [bouts] [seed] [iterations]".into());
    }
    Ok(Settings {
        bouts,
        seed,
        iterations,
    })
}

pub(crate) fn run(args: &[String]) -> ExitCode {
    let settings = match settings(args) {
        Ok(settings) => settings,
        Err(why) => {
            eprintln!("svanbot round-robin: {why}");
            return ExitCode::from(2);
        }
    };
    let Ok(budget) = budget() else {
        eprintln!("svanbot round-robin: SVANBOT_ROUND_ROBIN_ROWS is not a count");
        return ExitCode::from(2);
    };
    let dir = crate::fleet_dir();
    let fleet = Fleet::in_dir(&dir);
    let previous = match Record::load(&dir) {
        Ok(record) => record,
        Err(why) => {
            eprintln!("svanbot round-robin: the previous record is refused: {why}");
            return ExitCode::from(2);
        }
    };
    let game = deciders::game(settings.seed);
    let mut play = |gauntlet: &Gauntlet, row: &Row| play_row(gauntlet, row, &game, settings);
    let record = match pass(&fleet, settings, budget, previous.as_ref(), &mut play) {
        Ok(record) => record,
        Err(why) => {
            eprintln!("svanbot round-robin: {why}");
            return ExitCode::from(1);
        }
    };
    if record.bots.is_empty() {
        println!("svanbot round-robin: no bot is assigned a gauntlet yet");
        return ExitCode::SUCCESS;
    }
    if let Err(why) = record.save(&dir) {
        eprintln!("svanbot round-robin: the record was not written: {why}");
        return ExitCode::from(1);
    }
    println!(
        "svanbot round-robin: {} bot(s), {} row(s) played, {} left, in {}",
        record.bots.len(),
        record.played,
        record.skipped,
        svanbot_runtime::round_robin::path(&dir).display()
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests;

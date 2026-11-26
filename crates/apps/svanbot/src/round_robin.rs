//! `svanbot round-robin`: the standing test of each bot's gauntlet, one bounded pass, at low
//! priority under the fleet's supervisor.
//!
//! A pass plays every assigned bot's gauntlet in order: the ten shared opponents heads up, the
//! same ten at the 6-max ring, then a 6-max row against each champion in that bot's archive
//! (`learn::round_robin`). Each pass also takes one bounded step of the exploiter search against
//! each bot's champion (`learn::exploiter`), so the brain's exploitability is measured from the
//! best exploiter found; once measured, the bot meets that exploiter as the gauntlet's last row.
//! The rows play one after another, never in parallel, and the pass stops at its row budget.
//! Each row's paired bouts are read through `learn::probe::Sequence`, and the
//! archive rows are the evidence `learn::archive::regression` reads for a promotion. Exploiter
//! rows carry no archive and are never evidence for the regression rule: they report, they do not
//! gate.
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
use svanbot_learn::arena::BOUT;
use svanbot_learn::exploiter;
use svanbot_learn::paired::{Match, Verdict};
use svanbot_learn::round_robin::{self, Budget, MBB_PER_100, Row, Table};
use svanbot_learn::search::{Entry, Step};
use svanbot_runtime::fleet::{Fleet, League};
use svanbot_runtime::round_robin::{
    ArchivedChampion, BotRecord, ExploiterRecord, KEEP, Record, RowRecord,
};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::params::Params;
use svanbot_strategy::policy::Policy;

use crate::arena::{self, Opponent};
use crate::deciders;

mod step;
#[cfg(test)]
mod tests;

use step::{exploiter_sim, play_rival, step_exploiter};

/// The bouts one row plays unless the caller names another count.
const DEFAULT_BOUTS: u64 = 1;

/// The re-solve iterations one row runs unless the caller names another count: the pass is a
/// standing test, bounded enough to run beside the live fleet.
const DEFAULT_ITERATIONS: u64 = 2;

/// The deals one exploiter screening sim plays: a fifth of a confirmation, as the learner's.
const EXPLOITER_SCREEN_DEALS: u64 = 100;

/// The deals one exploiter confirmation plays: enough to measure a win rate, bounded for a
/// standing pass. One step runs a pass, so a search costs at most one of these a bot.
const EXPLOITER_CONFIRM_DEALS: u64 = 500;

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
    /// The exploiter search's log against `champion`, carried while the champion holds.
    exploiter_log: Vec<Entry>,
    /// The best exploiter a confirmation has put above `champion`, when one has.
    exploiter: Option<ExploiterRecord>,
}

/// How a pass plays one row: the row's paired bouts in mbb/100, or why it could not be played.
type Play<'a> = dyn FnMut(&Gauntlet, &Row) -> Result<Vec<f64>, String> + 'a;

/// How a pass plays one exploiter sim: the challenger's verdict against the bot's champion.
type Search<'a> = dyn FnMut(&Gauntlet, &Step, u64) -> Verdict + 'a;

/// Plays one pass and records it. `play` supplies a row's paired bouts in mbb/100 and `search`
/// one exploiter sim's verdict; the app's sim does the playing, and a test can stand in for it.
fn pass(
    fleet: &Fleet,
    settings: Settings,
    budget: usize,
    previous: Option<&Record>,
    play: &mut Play<'_>,
    search: &mut Search<'_>,
) -> Result<Record, String> {
    let mut gauntlets = gauntlets(fleet, previous)?;
    for gauntlet in &mut gauntlets {
        // One bounded step a pass: the search is the learner's, spread over the standing test.
        if gauntlet.exploiter.is_none() {
            step_exploiter(gauntlet, search);
        }
        // An exploiter that has been measured joins the gauntlet as its last row; the row is
        // heads up and carries no archive, so the regression rule never reads it.
        if let Some(found) = &gauntlet.exploiter {
            let row = Row::exploited(found.params.digest());
            if !gauntlet.plan.rows.contains(&row) {
                gauntlet.plan.rows.push(row);
            }
        }
    }
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
                exploiter: scored.row.exploiter,
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
            exploiter_log: gauntlet.exploiter_log.clone(),
            exploiter: gauntlet.exploiter.clone(),
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
        // A champion or policy change starts the exploiter search afresh: evidence about another
        // policy is not evidence about this one.
        let holding = previous
            .and_then(|record| record.bot(&bot))
            .filter(|held| held.champion == champion && held.policy == policy.name());
        let exploiter_log = holding
            .map(|held| held.exploiter_log.clone())
            .unwrap_or_default();
        let exploiter = holding
            .and_then(|held| held.exploiter.clone())
            .filter(|found| found.champion == champion.digest());
        let plan = round_robin::plan_with_exploiter(
            &bot,
            champion.digest(),
            &shared,
            &rules,
            exploiter.as_ref().map(|found| found.params.digest()),
        );
        gauntlets.push(Gauntlet {
            bot,
            policy,
            champion,
            archive,
            rules,
            plan,
            exploiter_log,
            exploiter,
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

/// Whether a record row is `row`'s line: same name, table, archived champion and exploiter.
fn same(held: &RowRecord, row: &Row) -> bool {
    held.row == row.name
        && held.table == row.table
        && held.archive == row.archive
        && held.exploiter == row.exploiter
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
            params: held.params,
            // The regression rule does not read the style; the archive's own writer supplies it.
            style: "standard",
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
                None if row.exploiter.is_some() => {
                    let digest = row.exploiter.expect("an exploiter row names its exploiter");
                    let found = gauntlet
                        .exploiter
                        .as_ref()
                        .filter(|found| found.params.digest() == digest)
                        .ok_or_else(|| {
                            format!("exploiter row {digest} has no measured exploiter")
                        })?;
                    play_rival(
                        game,
                        league,
                        &gauntlet.policy,
                        Policy::new(PolicyKind::Exploiting, exploiter::style(), found.params),
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
    play_rival(
        game,
        league,
        policy,
        Policy::new(policy.kind, policy.style, rival),
        number,
        fixed,
    )
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
    let mut search = |gauntlet: &Gauntlet, step: &Step, deals: u64| {
        exploiter_sim(gauntlet, step, deals, &game, settings.iterations)
    };
    let record = match pass(
        &fleet,
        settings,
        budget,
        previous.as_ref(),
        &mut play,
        &mut search,
    ) {
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

//! The ship bar (V11-22): the clauses `svanbot release-check` holds a tag to, in two stages.
//!
//! The **Arena stage** runs before the tag is pushed. It reads the fleet's `arena.json` and lets
//! a release through only when the newest run of each league:
//! - was played from the tag's own commit, under the fleet's champion
//! - plays again as recorded: [`REPLAYS`] bouts drawn from the run's seed come out bit for bit as
//!   stored, and each row's win rate is its bouts' total
//! - beats v10.0.2 heads up over [`V10_DEALS`] deals (a million hands), its 95% interval wholly
//!   above zero. The 6-max v10.0.2 row is shown, never required.
//! - has no regression: no other row, in either league, has its 95% interval wholly below zero.
//!
//! The **canary stage** runs before the fleet takes the release ([`canary`]).
//!
//! Every clause that fails is named, so one check reports all of them, not the first.

use svanbot_core::rng::Rng;
use svanbot_learn::arena::BOUT;
use svanbot_learn::paired::verdict;

use crate::fleet::{ArenaRow, ArenaRun, League};

pub mod window;

/// The heads-up deals against v10.0.2 a release must be measured over: a deal is two hands, so
/// this is a million.
pub const V10_DEALS: usize = 500_000;

/// The bouts of each run the ship bar plays again.
pub const REPLAYS: usize = 3;

/// The v10.0.2 row's name, as `sim::v10::seat::NAME` gives it.
const V10: &str = "v10.0.2";

/// mbb/100 in a big blind a hand.
const MBB_PER_100: f64 = 100_000.0;

/// The Arena stage: `Ok` when the newest run of each league from `commit` clears every clause,
/// or every clause it fails. `replay` plays one bout of a run's row again and returns what the
/// policy won over it, summed in deal order, the way `svanbot arena` stored it.
pub fn arena(
    runs: &[ArenaRun],
    commit: &str,
    champion: &str,
    replay: &mut dyn FnMut(&ArenaRun, &ArenaRow, u64) -> f64,
) -> Result<(), Vec<String>> {
    let mut failed = Vec::new();
    for league in [League::HeadsUp, League::SixMax] {
        let name = league.name();
        // Newest first, so the first from the commit is the newest.
        let Some(run) = runs
            .iter()
            .find(|run| run.league == league && run.commit == commit)
        else {
            failed.push(format!("no {name} run from {}", short(commit)));
            continue;
        };
        if run.champion != champion {
            failed.push(format!(
                "the {name} run played champion {}, not the fleet's {champion}",
                run.champion
            ));
        }
        failed.extend(run.rows.iter().filter_map(|row| totals(name, row)));
        failed.extend(replays(run, replay));
        failed.extend(bar(run));
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(failed)
    }
}

/// Why a row's numbers are not its bouts', if they are not.
fn totals(league: &str, row: &ArenaRow) -> Option<String> {
    let opponent = &row.opponent;
    if row.bouts.len() * BOUT as usize != row.deals {
        return Some(format!(
            "the {league} {opponent} row has {} bouts for {} deals",
            row.bouts.len(),
            row.deals
        ));
    }
    let won: f64 = row.bouts.iter().sum();
    let rate = won / row.deals.max(1) as f64 * MBB_PER_100;
    // The scoreboard summed deal by deal, the bouts bout by bout: the same total, rounded apart.
    let close = (rate - row.mbb_per_100).abs() <= 1e-9 * row.mbb_per_100.abs().max(1.0);
    (!close).then(|| format!("the {league} {opponent} row's win rate is not its bouts' total"))
}

/// The bouts drawn from `run`'s seed that did not play again as recorded.
fn replays(
    run: &ArenaRun,
    replay: &mut dyn FnMut(&ArenaRun, &ArenaRow, u64) -> f64,
) -> Vec<String> {
    let played: Vec<&ArenaRow> = run
        .rows
        .iter()
        .filter(|row| !row.bouts.is_empty())
        .collect();
    if played.is_empty() {
        return Vec::new();
    }
    let mut rng = Rng::stream(run.seed, run.league as u64);
    (0..REPLAYS)
        .filter_map(|_| {
            let row = played[rng.below(played.len() as u64) as usize];
            let bout = rng.below(row.bouts.len() as u64);
            let recorded = row.bouts[bout as usize];
            let again = replay(run, row, bout);
            (again.to_bits() != recorded.to_bits()).then(|| {
                format!(
                    "the {} {} row's bout {bout} did not play again as recorded: {recorded} then {again}",
                    run.league.name(),
                    row.opponent
                )
            })
        })
        .collect()
}

/// The v10.0.2 clause, heads up, and the regression clause, in either league.
fn bar(run: &ArenaRun) -> Vec<String> {
    let mut failed = Vec::new();
    let league = run.league.name();
    if run.league == League::HeadsUp {
        match run.rows.iter().find(|row| row.opponent == V10) {
            None => failed.push(format!("the {league} run has no {V10} row")),
            Some(row) if row.deals < V10_DEALS => failed.push(format!(
                "{V10} heads up: {} deals, short of {V10_DEALS}",
                row.deals
            )),
            Some(row) if row.mbb_per_100 - row.ci95 <= 0.0 => failed.push(format!(
                "{V10} heads up: {:+.0} mbb/100 ±{:.0}, the interval is not above zero",
                row.mbb_per_100, row.ci95
            )),
            Some(_) => {}
        }
    }
    for row in run.rows.iter().filter(|row| row.opponent != V10) {
        if row.mbb_per_100 + row.ci95 < 0.0 {
            failed.push(format!(
                "regression: the {} beats the policy at {league}, {:+.0} mbb/100 ±{:.0}",
                row.opponent, row.mbb_per_100, row.ci95
            ));
        }
    }
    failed
}

/// The hands the canary must have played on the release, and the fleet over the same window,
/// before the canary stage measures them.
pub const CANARY_HANDS: usize = 20_000;

/// The canary stage: what the canary won on each hand since it took the release, and what the
/// rest of the fleet won on each hand over the same window, in big blinds. `Ok` with the line
/// measured when the canary does not lose to the fleet beyond the noise, the 95% interval of the
/// difference not wholly below zero. Short of [`CANARY_HANDS`] on either side, it is not yet
/// measured, and refused.
pub fn canary(canary: &[f64], fleet: &[f64]) -> Result<String, String> {
    if canary.len() < CANARY_HANDS {
        return Err(format!(
            "the canary is not yet measured: {} of its {CANARY_HANDS} hands",
            canary.len()
        ));
    }
    if fleet.len() < CANARY_HANDS {
        return Err(format!(
            "the fleet is not yet measured over the canary's window: {} of {CANARY_HANDS} hands",
            fleet.len()
        ));
    }
    let (ours, theirs) = (verdict(canary), verdict(fleet));
    let gap = ours.mbb_per_100 - theirs.mbb_per_100;
    // Two independent samples: the errors add in quadrature.
    let ci95 = ours.ci95.hypot(theirs.ci95);
    let line = format!("{gap:+.0} mbb/100 ±{ci95:.0} over {} hands", canary.len());
    if gap + ci95 < 0.0 {
        Err(format!("the canary loses to the fleet: {line}"))
    } else {
        Ok(format!("the canary against the fleet: {line}"))
    }
}

/// The release a tag (`v11.0.0`) or a slot (`11.0.0`) names: three whole numbers, as the release
/// workflow makes them. Anything else names no release.
pub fn version(name: &str) -> Option<&str> {
    let version = name.strip_prefix('v').unwrap_or(name);
    let parts: Vec<&str> = version.split('.').collect();
    let numbers = parts
        .iter()
        .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
    (parts.len() == 3 && numbers).then_some(version)
}

/// A commit as a reader recognises it.
fn short(commit: &str) -> &str {
    &commit[..commit.len().min(12)]
}

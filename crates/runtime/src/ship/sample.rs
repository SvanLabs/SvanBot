//! The ship run (V11-40): the Arena stage's evidence, played until the bar is decided.
//!
//! Both leagues are played together a **heat** at a time, a heat being one more bout of every
//! row of every run. After each heat both runs are recorded and the bar is read
//! ([`judge`]): the run stops when it passes or is refused, and a run still undecided when the
//! cap runs out is refused. The rows are read through a confidence sequence, so stopping at
//! whichever heat decides costs the 95% level nothing.
//!
//! Nothing here knows the time or plays a hand: the caller passes the bout player, the cap and
//! the recorder in, so the loop is the same under test as on the box.

use svanbot_learn::arena::BOUT;
use svanbot_learn::paired::verdict;

use super::{Reading, bar, reading};
use crate::fleet::{ArenaRow, ArenaRun};

/// The cap on a ship run, in hours of the box's wall clock at nice 15, both leagues together.
pub const CAP_HOURS: u64 = 48;

/// One bout a heat asks for: row `row` of run `run`, bout `number`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bout {
    pub run: usize,
    pub row: usize,
    pub number: u64,
}

/// Where a ship run stands against the bar.
#[derive(Clone, Debug, PartialEq)]
pub enum Bar {
    /// v10.0.2 is read below the policy in every league, and no opponent above it.
    Clears,
    /// The bar is refused, for every reason named.
    Refused(Vec<String>),
    /// Not yet decided.
    Open,
}

/// The bar as `runs` stand now: refused the moment any row is read below zero, passed once
/// every clause clears, and running otherwise.
pub fn judge(runs: &[ArenaRun]) -> Bar {
    let failed: Vec<String> = runs.iter().flat_map(bar).collect();
    let lost = runs
        .iter()
        .flat_map(|run| &run.rows)
        .any(|row| reading(row) == Reading::Below);
    if failed.is_empty() {
        Bar::Clears
    } else if lost {
        Bar::Refused(failed)
    } else {
        Bar::Open
    }
}

/// Plays one heat: handed the runs as they stand and the heat's bouts, it returns what the
/// policy won over each bout, summed in deal order.
pub type Player<'a> = dyn FnMut(&[ArenaRun], &[Bout]) -> Vec<f64> + 'a;

/// Plays `runs` a heat at a time until the bar is decided, recording each run after every
/// heat. `out_of_time` is asked after each heat that leaves the bar undecided. Never returns
/// [`Bar::Open`].
pub fn sample(
    runs: &mut [ArenaRun],
    play: &mut Player<'_>,
    out_of_time: &mut dyn FnMut() -> bool,
    record: &mut dyn FnMut(&ArenaRun),
) -> Bar {
    let mut heats = 0;
    loop {
        let bouts: Vec<Bout> = runs
            .iter()
            .enumerate()
            .flat_map(|(run, played)| {
                played.rows.iter().enumerate().map(move |(row, line)| Bout {
                    run,
                    row,
                    number: line.bouts.len() as u64,
                })
            })
            .collect();
        let won = play(runs, &bouts);
        assert_eq!(won.len(), bouts.len(), "a bout player answers every bout");
        for (bout, won) in bouts.iter().zip(won) {
            add(&mut runs[bout.run].rows[bout.row], won);
        }
        heats += 1;
        for run in runs.iter() {
            record(run);
        }
        match judge(runs) {
            Bar::Open if out_of_time() => {
                let mut why = vec![format!(
                    "the bar is not decided within the cap: {heats} heats"
                )];
                why.extend(runs.iter().flat_map(bar));
                return Bar::Refused(why);
            }
            Bar::Open => {}
            decided => return decided,
        }
    }
}

/// Adds one bout's winnings to `row`, and keeps its numbers its bouts': the win rate their total
/// over its deals, and the scoreboard's 95% interval from the spread of the bouts.
fn add(row: &mut ArenaRow, won: f64) {
    row.bouts.push(won);
    row.deals = row.bouts.len() * BOUT as usize;
    let total: f64 = row.bouts.iter().sum();
    row.mbb_per_100 = total / row.deals as f64 * super::MBB_PER_100;
    let per_bout: Vec<f64> = row.bouts.iter().map(|won| won / BOUT as f64).collect();
    row.ci95 = verdict(&per_bout).ci95;
}

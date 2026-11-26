//! The round-robin (V11-93, spec Phase 3): the standing test of one bot's gauntlet, and the
//! bounded, sequential runner that plays it.
//!
//! One bot's gauntlet is ten shared opponents heads up, the same ten at the 6-max ring, and then
//! one 6-max row against each champion in its archive. The plan fixes the rows and their order;
//! the runner plays them in that order and stops at its budget. The rows are read through
//! [`Sequence`] (`learn::probe`), and the archive rows are the paired evidence
//! [`crate::archive::regression`] reads for a promotion.
//!
//! The module is pure: the caller passes the paired result of each row in, and a pass is the same
//! for the same plan, budget, and results. Playing the sims, reading the fleet and writing the
//! record are the app's work.

use svanbot_strategy::params::Digest;

use crate::archive::{Archive, Bouts, Regression, regression as archive_regression};
use crate::probe::Sequence;

/// mbb/100 in a big blind a hand.
pub const MBB_PER_100: f64 = 100_000.0;

/// The bouts the sequence is tightest around, as in the margin and the archive.
const TUNED_BOUTS: f64 = 200.0;

/// The most rows one pass plays without a smaller budget from its caller. Five bots of ten
/// shared pairs in two leagues and five archived champions each is 125 rows, so this bounds a
/// whole fleet's gauntlet with room to spare.
pub const MAX_ROWS: usize = 250;

/// The most bouts a row's standing evidence keeps: the newest this many, so the record stays
/// bounded while a sequence read grows over passes.
pub const KEEP_BOUTS: usize = 200;

/// The table a row is played at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    HeadsUp,
    SixMax,
}

impl Table {
    /// The seats at the table.
    pub const fn seats(self) -> usize {
        match self {
            Self::HeadsUp => 2,
            Self::SixMax => 6,
        }
    }

    /// The table's name, as the Arena's leagues are named.
    pub const fn name(self) -> &'static str {
        match self {
            Self::HeadsUp => "heads-up",
            Self::SixMax => "6-max",
        }
    }
}

/// One row of a gauntlet: who the bot meets, and at which table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The opponent's name, or `archive <digest>` for an archived champion.
    pub name: String,
    pub table: Table,
    /// For an archive row, the champion this row measures the bot against. A shared row has none.
    pub archive: Option<Digest>,
}

impl Row {
    fn shared(name: String, table: Table) -> Self {
        Self {
            name,
            table,
            archive: None,
        }
    }

    fn archived(champion: Digest) -> Self {
        Self {
            name: format!("archive {champion}"),
            table: Table::SixMax,
            archive: Some(champion),
        }
    }
}

/// One bot's gauntlet, in play order: ten shared pairs heads up, the same ring at 6-max, then a
/// 6-max row against each archived champion, newest first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub bot: String,
    pub champion: Digest,
    pub rows: Vec<Row>,
}

/// The plan for `bot`'s gauntlet: `shared` names the ten opponents, and `archive` supplies the
/// champions it has replaced, newest first.
pub fn plan(
    bot: impl Into<String>,
    champion: Digest,
    shared: &[String],
    archive: &Archive,
) -> Plan {
    let mut rows = Vec::with_capacity(shared.len() * 2 + archive.len());
    for name in shared {
        rows.push(Row::shared(name.clone(), Table::HeadsUp));
    }
    for name in shared {
        rows.push(Row::shared(name.clone(), Table::SixMax));
    }
    for held in archive.champions() {
        rows.push(Row::archived(held.digest));
    }
    Plan {
        bot: bot.into(),
        champion,
        rows,
    }
}

/// A row's paired bouts read through [`Sequence`]: the mean and half-width in mbb/100, and how
/// many bouts supplied them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub mean: f64,
    pub radius: f64,
    pub bouts: usize,
}

/// Reads `bouts` (the row's paired results in mbb/100) through a 95% confidence sequence.
pub fn read(bouts: &[f64]) -> Reading {
    let mut sequence = Sequence::new(TUNED_BOUTS);
    for &bout in bouts {
        sequence.observe(bout);
    }
    Reading {
        mean: sequence.mean(),
        radius: sequence.radius(0.05),
        bouts: bouts.len(),
    }
}

/// One played row: the plan's row, the paired bouts it played (mbb/100, one a bout), and their
/// reading. For an archive row the bout is the bot minus its archived champion.
#[derive(Clone, Debug, PartialEq)]
pub struct Scored {
    pub row: Row,
    pub bouts: Vec<f64>,
    pub reading: Reading,
}

/// How many rows one pass may play.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub rows: usize,
}

/// What a pass played, and how many rows its budget left unplayed at the end of the plan.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub rows: Vec<Scored>,
    pub left: usize,
}

/// Plays `plan` in order, at most `budget.rows` rows, and reads each row. `play` supplies the
/// row's paired bouts in mbb/100; the caller's sim does the playing.
pub fn run(plan: &Plan, budget: Budget, mut play: impl FnMut(&Row) -> Vec<f64>) -> Run {
    let mut rows = Vec::with_capacity(plan.rows.len().min(budget.rows));
    for row in &plan.rows {
        if rows.len() >= budget.rows {
            break;
        }
        let bouts = play(row);
        rows.push(Scored {
            row: row.clone(),
            reading: read(&bouts),
            bouts,
        });
    }
    Run {
        left: plan.rows.len() - rows.len(),
        rows,
    }
}

/// The archive rows of `played` as the paired evidence against every archived champion, read by
/// [`crate::archive::regression`].
pub fn regression(archive: &Archive, played: &[Scored]) -> Regression {
    let bouts: Vec<Bouts> = played
        .iter()
        .filter_map(|scored| {
            scored.row.archive.map(|champion| Bouts {
                champion,
                differences: scored.bouts.clone(),
            })
        })
        .collect();
    archive_regression(archive, &bouts)
}

//! The Archive (V11-96, spec Phase 3): a bot's last five champions, and the regression rule that
//! a promotion must clear against every one of them.
//!
//! One bot's archive is its own, like every other thing it learns (`docs/adr/0004-reads-are-per-bot.md`):
//! a stablemate never reads it. Each champion is kept by its digest, with the paired bouts that
//! promoted it. The newest five are held, newest first; a sixth promotion evicts the oldest.
//!
//! The regression rule reads the candidate's paired bouts against each archived champion and
//! refuses the promotion when the candidate's edge is wholly below zero at the 95% bound. A tie,
//! too few bouts or missing evidence holds the incumbent. The exploitability bound is the Phase 4
//! hook: once the exploiters report, a promotion may not raise exploitability past it.

use std::collections::BTreeMap;

use svanbot_strategy::params::Digest;

use crate::margin::MIN_BOUTS;
use crate::probe::Sequence;

/// The champions a bot's archive keeps.
pub const KEEP: usize = 5;

/// The most a promotion may lose to an archived champion, in mbb/100.
///
/// A loss is refused only when the candidate's whole 95% interval is below minus this, so zero
/// means "convincingly losing": a tie never refuses, it holds.
pub const REGRESSION_MARGIN: f64 = 0.0;

/// One archived champion: the parameter set, and the paired bouts that promoted it.
#[derive(Clone, Debug, PartialEq)]
pub struct Archived {
    pub digest: Digest,
    /// The bout differences against the champion it replaced, in mbb/100, candidate minus
    /// champion.
    pub promotion: Vec<f64>,
}

/// One bot's champions, newest first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Archive {
    entries: Vec<Archived>,
}

impl Archive {
    pub fn new() -> Self {
        Self::default()
    }

    /// How many champions the archive holds: at most [`KEEP`].
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The champions, newest first.
    pub fn champions(&self) -> impl ExactSizeIterator<Item = &Archived> {
        self.entries.iter()
    }

    /// Record a promotion: the champion becomes the newest, and the oldest beyond [`KEEP`] goes.
    pub fn record(&mut self, champion: Archived) {
        self.entries.retain(|held| held.digest != champion.digest);
        self.entries.insert(0, champion);
        self.entries.truncate(KEEP);
    }
}

/// Every bot's archive, kept apart: a bot's champions never reach a stablemate's archive.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Archives {
    bots: BTreeMap<String, Archive>,
}

impl Archives {
    pub fn new() -> Self {
        Self::default()
    }

    /// One bot's archive, or `None` before its first promotion.
    pub fn get(&self, bot: &str) -> Option<&Archive> {
        self.bots.get(bot)
    }

    /// Record a promotion in `bot`'s own archive.
    pub fn record(&mut self, bot: &str, champion: Archived) {
        self.bots
            .entry(bot.to_owned())
            .or_default()
            .record(champion);
    }
}

/// A candidate's paired sim against one archived champion: the bout differences, in mbb/100,
/// candidate minus champion.
#[derive(Clone, Debug, PartialEq)]
pub struct Bouts {
    pub champion: Digest,
    pub differences: Vec<f64>,
}

/// What the regression rule found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Regression {
    /// The candidate beats every archived champion at the 95% bound.
    Admitted,
    /// The candidate's edge against `champion` is wholly below zero: the promotion is refused.
    Refused { champion: Digest },
    /// A tie, too few bouts or missing evidence: the incumbent holds.
    Unresolved,
}

/// Read the candidate's paired `bouts` against every champion in `archive`.
///
/// The rule refuses when the candidate's edge against any champion is wholly below zero at the
/// 95% bound, so a tie never refuses. It admits only when the candidate beats all of them; a
/// champion with no resolved evidence holds the incumbent.
pub fn regression(archive: &Archive, bouts: &[Bouts]) -> Regression {
    let mut unresolved = false;
    for champion in archive.champions() {
        let Some(reading) = bouts
            .iter()
            .find(|reading| reading.champion == champion.digest)
        else {
            unresolved = true;
            continue;
        };
        if reading.differences.len() < MIN_BOUTS {
            unresolved = true;
            continue;
        }
        let (mean, radius) = edge(&reading.differences);
        if mean + radius < -REGRESSION_MARGIN {
            return Regression::Refused {
                champion: champion.digest,
            };
        }
        if mean - radius <= 0.0 {
            unresolved = true;
        }
    }
    if unresolved {
        Regression::Unresolved
    } else {
        Regression::Admitted
    }
}

/// The candidate's mean edge and the half-width of its 95% interval, in mbb/100.
fn edge(differences: &[f64]) -> (f64, f64) {
    let mut sequence = Sequence::new(200.0);
    for &difference in differences {
        sequence.observe(difference);
    }
    (sequence.mean(), sequence.radius(0.05))
}

/// The most a promotion may raise a bot's exploitability, in mbb/100: +5 bb/100 (2026-11-26).
pub const EXPLOITABILITY_BOUND: f64 = 5_000.0;

/// The exploitability hook the Phase 4 exploiters feed the regression rule.
///
/// An exploiter is a policy searched to beat one brain's champion in the paired sim; the best
/// one's win rate against the candidate, in mbb/100, is that candidate's exploitability. A
/// promotion may not raise it past [`EXPLOITABILITY_BOUND`]. Until the exploiters exist nothing
/// measures it, and [`Exploitability::Unmeasured`] passes: the bound arrives with them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Exploitability {
    /// No exploiter has measured the candidate yet.
    Unmeasured,
    /// The best exploiter's win rate against the candidate, in mbb/100.
    Measured(f64),
}

impl Exploitability {
    /// Whether the candidate is within the bound. Unmeasured passes, because the gate arrives
    /// with the exploiters in Phase 4.
    pub fn within_bound(self) -> bool {
        match self {
            Self::Unmeasured => true,
            Self::Measured(mbb_per_100) => mbb_per_100 <= EXPLOITABILITY_BOUND,
        }
    }
}

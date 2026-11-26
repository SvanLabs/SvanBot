//! Refresh (V11-81, spec Phase 2 item 6): a lineage dominated in the round-robin selects the best
//! eligible champion of its own style from its archive.
//!
//! v10 copied the tournament's winner into a dominated lineage, which here would erase the style
//! (ADR 0005). Instead the round-robin only says whether the lineage is dominated; the archive's
//! own promotion evidence chooses the champion to try, and only a champion of the bot's own style
//! is eligible. A refresh never changes the slot: it records the evidence and the selected digest,
//! and the champion is run through the same live guard as any promotion (`probe::Kind::Guard`).
//! Only a `Verdict::Cleared` lets the caller put it in the slot (`svanbot::play::apply_refresh`).
//!
//! This is the pure selection. The standing round-robin that produces [`Row`]s is #1247, and the
//! per-bot probe that runs the guard is #1222.

use svanbot_strategy::params::{Digest, Params};

use crate::archive::{Archive, Archived};
use crate::paired::Verdict as Pairing;
use crate::probe::Verdict as Guard;

/// One lineage's row in the round-robin matrix: its current champion, the style it plays, and
/// how every other lineage did against it.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub bot: String,
    pub style: &'static str,
    pub champion: Params,
    pub matchups: Vec<Matchup>,
}

/// One pairing in the round-robin matrix: another row's paired result over this one, in mbb/100,
/// read at the 95% bound.
#[derive(Clone, Debug, PartialEq)]
pub struct Matchup {
    /// The other row's name: another lineage's bot, or a shared row.
    pub name: String,
    pub verdict: Pairing,
}

/// Why the incumbent stays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// No other lineage's edge over this one is wholly above zero.
    NotDominated,
    /// The archive holds no champion of this style that is not the incumbent and whose promotion
    /// is still established at the 95% bound.
    NoEligibleChampion,
}

/// What a refresh found: the round-robin evidence for the row, and either the archived champion
/// to run through the live guard or the reason the incumbent stays.
#[derive(Clone, Debug, PartialEq)]
pub struct Refresh {
    pub bot: String,
    pub style: &'static str,
    pub incumbent: Digest,
    /// The matchups that dominate the lineage at the 95% bound, strongest first. Recorded
    /// whether or not a champion is selected.
    pub evidence: Vec<Matchup>,
    pub outcome: Outcome,
}

/// The refresh's decision.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The incumbent stays.
    NoChange(Reason),
    /// Run this champion through the live guard; the slot changes only on `Verdict::Cleared`.
    Selected(Archived),
}

/// Selects a refresh for `row` from its `archive`.
///
/// A lineage is dominated when any other lineage's edge over it is wholly above zero at the 95%
/// bound. A dominated lineage takes its best eligible same-style archived champion; a lineage that
/// is not dominated, or has none, keeps its incumbent. The record carries the dominating evidence
/// and, when selected, the champion's digest.
pub fn refresh(row: &Row, archive: &Archive) -> Refresh {
    let mut evidence: Vec<Matchup> = row
        .matchups
        .iter()
        .filter(|matchup| matchup.verdict.above_zero())
        .cloned()
        .collect();
    // Strongest lower bound first, then by name: two equal runs record the same evidence.
    evidence.sort_by(|left, right| {
        lower(&right.verdict)
            .total_cmp(&lower(&left.verdict))
            .then_with(|| left.name.cmp(&right.name))
    });
    let outcome = if evidence.is_empty() {
        Outcome::NoChange(Reason::NotDominated)
    } else {
        match archive.best_eligible(row.style, row.champion.digest()) {
            Some(selected) => Outcome::Selected(selected.clone()),
            None => Outcome::NoChange(Reason::NoEligibleChampion),
        }
    };
    Refresh {
        bot: row.bot.clone(),
        style: row.style,
        incumbent: row.champion.digest(),
        evidence,
        outcome,
    }
}

impl Refresh {
    /// The selected champion's digest, when one is selected.
    pub fn selected(&self) -> Option<Digest> {
        match &self.outcome {
            Outcome::Selected(selected) => Some(selected.digest()),
            Outcome::NoChange(_) => None,
        }
    }

    /// The parameter set to put in the slot, but only once the live guard cleared it: the same
    /// `probe::Verdict::Cleared` every promotion needs. Anything else leaves the incumbent in play.
    pub fn through_guard(&self, verdict: Guard) -> Option<Params> {
        match (&self.outcome, verdict) {
            (Outcome::Selected(selected), Guard::Cleared) => Some(selected.params),
            _ => None,
        }
    }
}

/// The lower 95% bound of a paired result, in mbb/100.
fn lower(verdict: &Pairing) -> f64 {
    verdict.mbb_per_100 - verdict.ci95
}

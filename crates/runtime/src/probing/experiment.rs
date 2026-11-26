//! The experiment ledger's side of the probe (V11-129): a challenger put forward for a bot's own
//! hypothesis claims it when the canary takes it up, and the guard's verdict writes the same
//! evidence into the record. Nothing here gates the probe: a ledger that cannot be written is a
//! note to the operator, never a stopped probe.

use svanbot_learn::experiment::{self, Outcome};
use svanbot_learn::probe::Verdict;

use super::Probing;
use crate::fleet::{Evidence, Proposal};

impl Probing<'_> {
    /// Claims the waiting experiment whose challenger is this proposal's. The note says whether
    /// it was claimed, or why the ledger could not be read.
    pub(super) fn claim_experiment(&self, proposal: &Proposal, now_ms: i64) -> Option<String> {
        match self
            .fleet
            .claim_experiment(&self.bot, &proposal.challenger.digest(), now_ms)
        {
            Ok(true) => Some("; its experiment is claimed".to_owned()),
            Ok(false) => None,
            Err(error) => Some(format!(". The experiment ledger was not read: {error}")),
        }
    }

    /// Writes the guard's reading into the bot's testing experiment: the verdict, the live edge
    /// and half-width, and the hands counted.
    pub(super) fn record_experiment(
        &self,
        id: u64,
        verdict: Verdict,
        edge: f64,
        radius: f64,
        now_ms: i64,
    ) -> Option<String> {
        let evidence = Evidence {
            verdict: super::verdict_name(verdict).to_owned(),
            edge,
            radius,
            hands: self.hands(id),
        };
        match self.fleet.record_experiment(
            &self.bot,
            Some(id),
            experiment::outcome(verdict),
            evidence,
            now_ms,
        ) {
            Ok(Some(note)) => Some(format!(". {note}")),
            Ok(None) => None,
            Err(error) => Some(format!(". The experiment ledger was not written: {error}")),
        }
    }

    /// Ends the bot's testing experiment undecided, without a verdict from the guard: a frozen
    /// reference or a probe the store left open. The bot is left exactly as it was.
    pub(super) fn end_experiment_undecided(
        &self,
        id: u64,
        why: &str,
        edge: f64,
        radius: f64,
        now_ms: i64,
    ) {
        let evidence = Evidence {
            verdict: why.to_owned(),
            edge,
            radius,
            hands: self.hands(id),
        };
        let _ =
            self.fleet
                .record_experiment(&self.bot, Some(id), Outcome::Undecided, evidence, now_ms);
    }

    /// Ends the waiting experiment whose proposal the canary dropped: claims it, then records it
    /// undecided, so the hypothesis and its outcome both stay in the ledger.
    pub(super) fn retire_experiment(&self, proposal: &Proposal, why: &str, now_ms: i64) {
        let Ok(true) =
            self.fleet
                .claim_experiment(&self.bot, &proposal.challenger.digest(), now_ms)
        else {
            return;
        };
        let evidence = Evidence {
            verdict: why.to_owned(),
            edge: 0.0,
            radius: 0.0,
            hands: 0,
        };
        let _ = self
            .fleet
            .record_experiment(&self.bot, None, Outcome::Undecided, evidence, now_ms);
    }

    /// The hands the probe `id` has counted in this store, 0 when they cannot be read.
    fn hands(&self, id: u64) -> u64 {
        self.store
            .orbits(id)
            .map(|orbits| {
                orbits
                    .iter()
                    .map(|&(_, _, _, hands)| u64::from(hands))
                    .sum()
            })
            .unwrap_or(0)
    }
}

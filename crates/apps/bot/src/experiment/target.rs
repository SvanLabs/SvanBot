//! Evidence targets (0267): the learner publishes which one-knob challengers remain unresolved by
//! simulation; the fleet runs the first one without a live verdict.
//!
//! The queue is scoped like the rejection ledger — champion version and evidence watermark — so a
//! promotion or a refit retires every target at once. Order: the survivor now in fresh-deal
//! confirmation, then undecided ledger transitions (positive point estimate, 95% upper bound above
//! +1 bb/100, never rejected by a completed confirmation), most likely to clear the bar first.

use serde::{Deserialize, Serialize};
use sv10_core::policy::Params;

use crate::promotion::MIN_EDGE_BB;
use crate::search_ledger::{Ledger, LedgerEntry, is_decisive, transition_key};

/// KV key of the learner's published [`TargetQueue`].
pub const TARGETS_KEY: &str = "learner.experiment-targets.v1";

/// Where a target came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The search survivor in fresh-deal confirmation.
    Confirmation,
    /// An undecided transition in the rejection ledger.
    Ledger,
}

/// One versioned hypothesis: "this challenger beats the champion live".
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Target {
    /// Stable id: champion version, evidence watermark and transition.
    pub id: String,
    /// The one knob that differs.
    pub knob: String,
    /// Champion value.
    pub old: f64,
    /// Challenger value.
    pub new: f64,
    /// Complete challenger parameters (the champion's knobs with `knob` moved).
    pub challenger: Params,
    /// The simulated paired evidence behind it.
    pub sim: LedgerEntry,
    /// Where it came from.
    pub source: Source,
}

impl Target {
    /// A target for `knob` moved from `old` to `new`, scoped to (champion version, watermark).
    pub fn new(
        (champion, refit_rowid): (&str, i64),
        knob: &str,
        old: f64,
        new: f64,
        challenger: Params,
        sim: LedgerEntry,
        source: Source,
    ) -> Self {
        Target {
            id: format!("{champion}@{refit_rowid}:{}", transition_key(knob, old, new)),
            knob: knob.to_string(),
            old,
            new,
            challenger,
            sim,
            source,
        }
    }

    /// Short human label: `fold_scale 0.900 -> 0.950`.
    pub fn label(&self) -> String {
        format!("{} {:.3} -> {:.3}", self.knob, self.old, self.new)
    }

    /// z of the simulated mean above the +1 bb/100 bar (the ranking key).
    fn z_to_bar(&self) -> f64 {
        if self.sim.se_bb > 0.0 { (self.sim.mean_bb - MIN_EDGE_BB) / self.sim.se_bb } else { f64::NEG_INFINITY }
    }
}

/// The learner's published queue.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TargetQueue {
    /// Champion version the targets were measured against.
    pub champion: String,
    /// Evidence watermark they were measured against.
    pub refit_rowid: i64,
    /// Unix seconds of publication.
    pub updated: f64,
    /// The survivor in fresh-deal confirmation, while one is.
    pub confirming: Option<Target>,
    /// Undecided ledger transitions, best first.
    pub candidates: Vec<Target>,
}

/// Whether a ledger measurement leaves its transition open: ahead, its upper bound above the bar,
/// and not already decided dead by a full screening budget.
pub fn undecided(entry: &LedgerEntry, tables: usize, hands: usize) -> bool {
    entry.differing > 0 && entry.mean_bb > 0.0 && entry.upper_95() > MIN_EDGE_BB && !is_decisive(entry, tables, hands)
}

/// Build the queue from the ledger. `proposals` maps a transition key to the challenger this
/// champion's search proposes for it; transitions without one (a proposal from another step size)
/// cannot be replayed exactly and are left out.
pub fn build_queue(
    ledger: &Ledger,
    proposals: &[(String, f64, f64, Params)],
    confirming: Option<Target>,
    tables: usize,
    hands: usize,
    now: f64,
) -> TargetQueue {
    let mut candidates: Vec<Target> = proposals
        .iter()
        .filter_map(|(knob, old, new, params)| {
            let key = transition_key(knob, *old, *new);
            let entry = ledger.entries.get(&key)?;
            if !undecided(entry, tables, hands) || ledger.confirm_rejected.contains(&key) {
                return None;
            }
            Some(Target::new((&ledger.champion, ledger.refit_rowid), knob, *old, *new, params.clone(), entry.clone(), Source::Ledger))
        })
        .filter(|t| confirming.as_ref().is_none_or(|c| c.id != t.id))
        .collect();
    candidates.sort_by(|a, b| b.z_to_bar().total_cmp(&a.z_to_bar()).then_with(|| a.id.cmp(&b.id)));
    candidates.dedup_by(|a, b| a.id == b.id);
    TargetQueue { champion: ledger.champion.clone(), refit_rowid: ledger.refit_rowid, updated: now, confirming, candidates }
}

/// The target the pair should run, or why none is safe. `live_champion` is the version live play
/// has installed; `done` says whether a target already has a live verdict.
pub fn select<'a>(queue: Option<&'a TargetQueue>, live_champion: &str, done: impl Fn(&str) -> bool) -> Result<&'a Target, String> {
    let queue = queue.ok_or("the learner has published no experiment targets yet")?;
    if queue.champion != live_champion {
        return Err(format!("targets were measured against {}, live play runs {live_champion}", queue.champion));
    }
    queue
        .confirming
        .iter()
        .chain(&queue.candidates)
        .find(|t| !done(&t.id))
        .ok_or_else(|| "no undecided challenger: every simulated transition is resolved".to_string())
}

/// The first target of this scope the experiment pair supported live and no completed
/// confirmation has rejected: the learner confirms it next instead of searching (0267).
pub fn live_supported(queue: Option<&TargetQueue>, verdicts: &crate::experiment::Verdicts, ledger: &Ledger) -> Option<Target> {
    let queue = queue.filter(|q| q.champion == ledger.champion && q.refit_rowid == ledger.refit_rowid)?;
    queue
        .confirming
        .iter()
        .chain(&queue.candidates)
        .find(|t| {
            verdicts.get(&t.id).is_some_and(|v| v.verdict == crate::experiment::Verdict::LiveSupported)
                && !ledger.confirm_rejected.contains(&transition_key(&t.knob, t.old, t.new))
        })
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(mean_bb100: f64, se_bb100: f64, hands: u64) -> LedgerEntry {
        LedgerEntry { hands, mean_bb: mean_bb100 / 100.0, se_bb: se_bb100 / 100.0, differing: hands / 2 }
    }

    fn proposal(knob: &str, old: f64, new: f64) -> (String, f64, f64, Params) {
        (knob.to_string(), old, new, Params { fold_scale: new, ..Params::default() })
    }

    fn ledger(entries: &[(&str, f64, f64, LedgerEntry)]) -> Ledger {
        let mut l = Ledger { champion: "sv10-ev-40".into(), refit_rowid: 7, ..Default::default() };
        for (knob, old, new, e) in entries {
            l.entries.insert(transition_key(knob, *old, *new), e.clone());
        }
        l
    }

    #[test]
    fn only_undecided_positive_transitions_become_targets_best_first() {
        let l = ledger(&[
            ("a", 1.0, 1.1, entry(0.8, 1.0, 4_000)),  // z -0.2: open
            ("b", 1.0, 0.9, entry(2.5, 1.0, 4_000)),  // z 1.5: open, first
            ("c", 1.0, 1.2, entry(-0.5, 1.0, 4_000)), // behind: never a target
            ("d", 1.0, 0.8, entry(0.2, 0.3, 4_000)),  // upper 0.79 < +1: decided dead
        ]);
        let props = vec![
            proposal("a", 1.0, 1.1),
            proposal("b", 1.0, 0.9),
            proposal("c", 1.0, 1.2),
            proposal("d", 1.0, 0.8),
            proposal("e", 1.0, 1.3),
        ];
        let q = build_queue(&l, &props, None, 4, 100, 1.0);
        let labels: Vec<String> = q.candidates.iter().map(Target::label).collect();
        assert_eq!(labels, vec!["b 1.000 -> 0.900", "a 1.000 -> 1.100"]);
        assert!(q.candidates[0].id.starts_with("sv10-ev-40@7:"), "scoped to champion and watermark");
    }

    #[test]
    fn a_completed_confirmation_rejection_is_never_resurrected() {
        let mut l = ledger(&[("b", 1.0, 0.9, entry(2.5, 1.0, 4_000))]);
        l.confirm_rejected.insert(transition_key("b", 1.0, 0.9));
        assert!(build_queue(&l, &[proposal("b", 1.0, 0.9)], None, 4, 100, 1.0).candidates.is_empty());
    }

    #[test]
    fn selection_prefers_confirmation_skips_decided_and_refuses_a_foreign_champion() {
        let l = ledger(&[("a", 1.0, 1.1, entry(0.8, 1.0, 4_000)), ("b", 1.0, 0.9, entry(2.5, 1.0, 4_000))]);
        let (k, o, n, p) = proposal("z", 1.0, 1.05);
        let confirming = Target::new(("sv10-ev-40", 7), &k, o, n, p, entry(3.0, 1.0, 9_000), Source::Confirmation);
        let q = build_queue(&l, &[proposal("a", 1.0, 1.1), proposal("b", 1.0, 0.9)], Some(confirming.clone()), 4, 100, 1.0);
        assert_eq!(select(Some(&q), "sv10-ev-40", |_| false).unwrap().id, confirming.id);
        let next = select(Some(&q), "sv10-ev-40", |id| id == confirming.id).unwrap();
        assert_eq!(next.knob, "b");
        assert!(select(Some(&q), "sv10-ev-41", |_| false).unwrap_err().contains("live play runs sv10-ev-41"));
        assert!(select(Some(&q), "sv10-ev-40", |_| true).unwrap_err().contains("no undecided"));
        assert!(select(None, "sv10-ev-40", |_| false).is_err());
    }

    #[test]
    fn live_support_prioritizes_confirmation_only_within_scope_and_only_once() {
        use crate::experiment::{Estimate, Verdict, VerdictRecord, Verdicts};
        let mut l = ledger(&[("b", 1.0, 0.9, entry(2.5, 1.0, 4_000))]);
        let q = build_queue(&l, &[proposal("b", 1.0, 0.9)], None, 4, 100, 1.0);
        let mut verdicts = Verdicts::new();
        assert!(live_supported(Some(&q), &verdicts, &l).is_none(), "no verdict, no priority");
        let record = |verdict| VerdictRecord { verdict, at: 1.0, label: "b".into(), estimate: Estimate::default() };
        verdicts.insert(q.candidates[0].id.clone(), record(Verdict::Inconclusive));
        assert!(live_supported(Some(&q), &verdicts, &l).is_none());
        verdicts.insert(q.candidates[0].id.clone(), record(Verdict::LiveSupported));
        assert_eq!(live_supported(Some(&q), &verdicts, &l).map(|t| t.knob), Some("b".to_string()));
        let mut other = l.clone();
        other.refit_rowid = 8;
        assert!(live_supported(Some(&q), &verdicts, &other).is_none(), "a refit retires the scope");
        l.confirm_rejected.insert(transition_key("b", 1.0, 0.9));
        assert!(live_supported(Some(&q), &verdicts, &l).is_none(), "a completed confirmation decides it");
    }
}

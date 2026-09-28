//! Persistent rejection ledger for the learner's successive-halving search (0285).
//!
//! The search re-proposes the same one-knob transitions cycle after cycle (138 distinct
//! transitions evaluated 6,402 times: 97.8% repeats), and the per-round accumulator that combines
//! repeated measurements dies with the cycle, so every repeat is simulated from scratch. The
//! ledger persists the *combined* [`PairedResult`](sv10_core::sim::PairedResult) per transition,
//! scoped to the champion version and the evidence epoch: a repeat then continues the interval
//! instead of restarting it, and a transition whose combined interval is decisively below the
//! +1 bb/100 promotion bar is not proposed while the scope is valid.
//!
//! Invalidation is by construction, not by expiry: a promotion changes the champion version, the
//! evidence epoch advances as the fleet plays (see [`crate::pacing::evidence_epoch`]), and both are
//! part of the scope, so a stale entry can never bar a live candidate. The fresh-deal confirmation
//! path (`sv10_bot::promotion`) is untouched — the ledger only screens, it never promotes.

use std::collections::BTreeMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sv10_core::policy::Params;
use sv10_core::sim::PairedResult;
use sv10_store::store::Store;

use crate::promotion::MIN_EDGE_BB;

/// KV key holding the serialized [`Ledger`].
pub const LEDGER_KEY: &str = "learner.rejection-ledger.v1";

/// One combined measurement. Lossless with [`PairedResult`]: the interval rebuilds exactly.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    /// Hands compared, across every cycle that measured this transition.
    pub hands: u64,
    /// Challenger minus champion, big blinds per hand.
    pub mean_bb: f64,
    /// Standard error of `mean_bb`.
    pub se_bb: f64,
    /// Hands whose outcome differed between the two policies.
    pub differing: u64,
}

impl LedgerEntry {
    /// Rebuild the interval arithmetic exactly as the search accumulates it.
    pub fn to_paired(&self) -> PairedResult {
        PairedResult { hands: self.hands, mean_bb: self.mean_bb, se_bb: self.se_bb, differing: self.differing }
    }

    /// Freeze an accumulated search result for storage.
    pub fn from_paired(r: &PairedResult) -> LedgerEntry {
        LedgerEntry { hands: r.hands, mean_bb: r.mean_bb, se_bb: r.se_bb, differing: r.differing }
    }

    /// Upper end of the 95% interval, the same bar the search drops candidates by.
    pub fn upper_95(&self) -> f64 {
        self.mean_bb + 1.96 * self.se_bb
    }
}

/// The persisted ledger: one scope, one combined entry per measured transition.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Ledger {
    /// Champion version the entries were measured against; a promotion retires the scope.
    pub champion: String,
    /// Evidence epoch the entries were measured against; the epoch advancing retires the scope.
    pub refit_rowid: i64,
    /// Combined entries by [`transition_key`].
    pub entries: BTreeMap<String, LedgerEntry>,
    /// Transitions a completed fresh-deal confirmation rejected under this scope: never offered
    /// again as a live experiment target (0291). Absent in ledgers stored before it.
    #[serde(default)]
    pub confirm_rejected: std::collections::BTreeSet<String>,
}

/// One search candidate with its accumulated result, as the halving loop carries it.
pub type Candidate = (String, f64, f64, Params, Option<PairedResult>);

/// Stable key for a one-knob transition. Nine decimals separate every step the search proposes
/// (the finest is 0.01) while keeping distinct transitions distinct.
pub fn transition_key(knob: &str, old: f64, new: f64) -> String {
    format!("{knob}|{old:.9}|{new:.9}")
}

/// A stored entry bars its candidate while the scope is valid: at least one full screening
/// budget of evidence (`tables * hands`) whose 95% upper bound sits below the +1 bb/100 bar —
/// the same bar the search drops by. A zero-difference entry is subsumed: with hands behind it
/// its interval collapses onto zero, which clears the bar by even more.
pub fn is_decisive(entry: &LedgerEntry, tables: usize, hands: usize) -> bool {
    entry.hands >= tables as u64 * hands as u64 && entry.upper_95() < MIN_EDGE_BB
}

/// Load the ledger for this scope. Anything stored under another scope — a promotion or an
/// evidence refresh since — is retired, and every transition is proposed again.
pub fn load(store: &Store, champion: &str, refit_rowid: i64) -> Ledger {
    let stored: Option<Ledger> = store.get_kv(LEDGER_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok());
    match stored {
        Some(l) if l.champion == champion && l.refit_rowid == refit_rowid => l,
        _ => Ledger { champion: champion.to_string(), refit_rowid, ..Default::default() },
    }
}

/// Persist the ledger. Store write failures are surfaced: a ledger that silently stops
/// recording would spend the search budget twice without anyone noticing.
pub fn save(store: &Store, ledger: &Ledger) -> Result<()> {
    store.put_kv(LEDGER_KEY, &serde_json::to_string(ledger)?)?;
    Ok(())
}

/// Seed each candidate's accumulator with its stored measurement and drop the decided-dead.
/// Returns the takeable pool and how many transitions the ledger barred. An empty ledger — a
/// fresh store — returns the pool untouched, so behaviour without history is exactly today's.
pub fn seed_and_filter(pool: Vec<Candidate>, ledger: &Ledger, tables: usize, hands: usize) -> (Vec<Candidate>, usize) {
    let mut kept = Vec::with_capacity(pool.len());
    let mut barred = 0usize;
    for (knob, old, new, params, acc) in pool {
        debug_assert!(acc.is_none(), "seed_and_filter takes a fresh pool");
        match ledger.entries.get(&transition_key(&knob, old, new)) {
            Some(entry) if is_decisive(entry, tables, hands) => barred += 1,
            Some(entry) => kept.push((knob, old, new, params, Some(entry.to_paired()))),
            None => kept.push((knob, old, new, params, None)),
        }
    }
    (kept, barred)
}

/// Merge finished search results back. Each result already contains its seeded history (the
/// rounds combined onto it), so overwriting is combining, not restarting.
pub fn record(ledger: &mut Ledger, done: &[(String, PairedResult)]) {
    for (key, r) in done {
        ledger.entries.insert(key.clone(), LedgerEntry::from_paired(r));
    }
}

/// `review ledger`: the learner's rejection ledger for the live champion and evidence watermark —
/// which transitions are barred as decided-dead and which are still accumulating evidence (0285).
pub fn review(store: &Store) -> Result<String> {
    let lineage: Vec<String> = store.get_kv(crate::LEARNER_LINEAGE_KEY)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let champion = lineage.last().cloned().unwrap_or_default();
    let pace: crate::pacing::PacingState =
        store.get_kv(crate::pacing::PACING_KEY)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    // The same epoch a search would record under, so `review ledger` names the entries that are
    // actually in force rather than looking for a watermark no search has ever stored (#314).
    let ledger = load(store, &champion, crate::pacing::evidence_epoch(pace.refit_rowid));
    let hw = sv10_core::hardware::detect();
    Ok(describe(&ledger, hw.tuning.learner_tables, hw.tuning.learner_hands))
}

/// Human-readable ledger state for `review ledger`: the scope, then every dead transition with
/// the interval that barred it, then the live ones still accumulating evidence.
pub fn describe(ledger: &Ledger, tables: usize, hands: usize) -> String {
    let mut out = format!(
        "rejection ledger for {} @ refit_rowid {} ({} transitions, bar: {} hands and upper < +1 bb/100)\n",
        ledger.champion,
        ledger.refit_rowid,
        ledger.entries.len(),
        tables * hands
    );
    let mut dead: Vec<(&String, &LedgerEntry)> = Vec::new();
    let mut live: Vec<(&String, &LedgerEntry)> = Vec::new();
    for (key, entry) in &ledger.entries {
        if is_decisive(entry, tables, hands) { dead.push((key, entry)) } else { live.push((key, entry)) }
    }
    out.push_str(&format!("barred ({}):\n", dead.len()));
    for (key, e) in dead {
        out.push_str(&format!(
            "  {key}: {:+.2} bb/100 (95% {:+.2}..{:+.2}) over {} hands ({} differing)\n",
            e.mean_bb * 100.0,
            (e.mean_bb - 1.96 * e.se_bb) * 100.0,
            e.upper_95() * 100.0,
            e.hands,
            e.differing
        ));
    }
    out.push_str(&format!("accumulating ({}):\n", live.len()));
    for (key, e) in live {
        out.push_str(&format!(
            "  {key}: {:+.2} bb/100 (95% {:+.2}..{:+.2}) over {} hands\n",
            e.mean_bb * 100.0,
            (e.mean_bb - 1.96 * e.se_bb) * 100.0,
            e.upper_95() * 100.0,
            e.hands
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(knob: &str, old: f64, new: f64) -> Candidate {
        (knob.to_string(), old, new, Params::default(), None)
    }

    fn entry(hands: u64, mean_bb: f64, se_bb: f64) -> LedgerEntry {
        LedgerEntry { hands, mean_bb, se_bb, differing: hands / 2 }
    }

    #[test]
    fn empty_ledger_returns_the_pool_untouched() {
        // A fresh store behaves exactly as today: no seed, no bar.
        let ledger = Ledger::default();
        let pool = vec![candidate("open_bb", 2.5, 3.0), candidate("four_bet", 2.3, 2.5)];
        let (kept, barred) = seed_and_filter(pool, &ledger, 4, 100);
        assert_eq!(barred, 0);
        assert_eq!(kept.len(), 2);
        assert!(kept.iter().all(|c| c.4.is_none()));
    }

    #[test]
    fn decisive_entry_bars_its_transition() {
        // 46 rejections of open_bb 2.5 -> 3.0 combine below the bar: never simulated again.
        let mut ledger = Ledger::default();
        ledger.entries.insert(transition_key("open_bb", 2.5, 3.0), entry(4_600, -0.02, 0.005));
        let pool = vec![candidate("open_bb", 2.5, 3.0), candidate("four_bet", 2.3, 2.5)];
        let (kept, barred) = seed_and_filter(pool, &ledger, 4, 100);
        assert_eq!(barred, 1);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].0, "four_bet");
    }

    #[test]
    fn zero_difference_with_evidence_bars_without_simulation() {
        let mut ledger = Ledger::default();
        ledger.entries.insert(transition_key("open_bb", 2.5, 3.0), LedgerEntry { hands: 800, mean_bb: 0.0, se_bb: 0.0, differing: 0 });
        let (kept, barred) = seed_and_filter(vec![candidate("open_bb", 2.5, 3.0)], &ledger, 4, 100);
        assert_eq!(barred, 1);
        assert!(kept.is_empty());
    }

    #[test]
    fn wide_interval_seeds_but_does_not_bar() {
        // A non-decisive entry continues its interval instead of restarting it.
        let mut ledger = Ledger::default();
        ledger.entries.insert(transition_key("open_bb", 2.5, 3.0), entry(400, 0.005, 0.01));
        let (kept, barred) = seed_and_filter(vec![candidate("open_bb", 2.5, 3.0)], &ledger, 4, 100);
        assert_eq!(barred, 0);
        let seeded = kept[0].4.clone().expect("seeded accumulator");
        assert_eq!(seeded.hands, 400);
    }

    #[test]
    fn thin_evidence_does_not_bar_even_below_the_bar() {
        // One bad round is not a verdict: below the hands bar, the candidate is proposed again.
        let mut ledger = Ledger::default();
        ledger.entries.insert(transition_key("open_bb", 2.5, 3.0), entry(50, -0.05, 0.005));
        let (kept, barred) = seed_and_filter(vec![candidate("open_bb", 2.5, 3.0)], &ledger, 4, 100);
        assert_eq!(barred, 0);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn promotion_or_refresh_retires_the_scope() {
        // A stale entry can never bar a live candidate: both ids are in the key.
        let json = serde_json::to_string(&Ledger {
            champion: "sv10-ev-34".to_string(),
            refit_rowid: 1000,
            entries: BTreeMap::from([(transition_key("open_bb", 2.5, 3.0), entry(4_600, -0.02, 0.005))]),
            ..Default::default()
        })
        .unwrap();
        let load_scope = |champion: &str, refit_rowid: i64| -> Ledger {
            let stored: Option<Ledger> = serde_json::from_str(&json).ok();
            match stored {
                Some(l) if l.champion == champion && l.refit_rowid == refit_rowid => l,
                _ => Ledger { champion: champion.to_string(), refit_rowid, ..Default::default() },
            }
        };
        assert_eq!(load_scope("sv10-ev-34", 1000).entries.len(), 1);
        assert!(load_scope("sv10-ev-35", 1000).entries.is_empty());
        assert!(load_scope("sv10-ev-34", 2000).entries.is_empty());
    }

    #[test]
    fn record_combines_instead_of_restarting() {
        // Seed, simulate one more round, record: hands are the sum and the bar sees everything.
        let first = PairedResult { hands: 400, mean_bb: 0.0, se_bb: 0.01, differing: 200 };
        let round = PairedResult { hands: 200, mean_bb: -0.03, se_bb: 0.02, differing: 100 };
        let combined = first.combine(&round);
        let mut ledger = Ledger::default();
        record(&mut ledger, &[("k".to_string(), combined.clone())]);
        let stored = &ledger.entries["k"];
        assert_eq!(stored.hands, 600);
        assert_eq!(stored.differing, 300);
        assert!((stored.mean_bb - combined.mean_bb).abs() < 1e-12);
        assert_eq!(stored.to_paired().upper_95(), combined.upper_95());
    }

    #[test]
    fn keys_separate_transitions() {
        assert_ne!(transition_key("open_bb", 2.5, 3.0), transition_key("open_bb", 2.5, 2.75));
        assert_ne!(transition_key("open_bb", 2.5, 3.0), transition_key("four_bet", 2.5, 3.0));
        assert_eq!(transition_key("open_bb", 2.5, 3.0), transition_key("open_bb", 2.5, 3.0));
    }
}

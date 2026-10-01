//! Frozen recorded starting-stack distributions: hero seat zero, actual opponent asymmetry.
use serde::{Deserialize, Serialize};
use sv10_store::store::Store;

/// Version of the paired stack-distribution objective.
pub const CONTRACT: u32 = 1;
/// Bounded source window and retained deterministic sample.
const WINDOW: usize = 4096;
const SAMPLE: usize = 256;

/// A search's immutable stack objective, persisted with its accumulators.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Fixture {
    /// Zero means an incompatible legacy equal-stack run.
    pub contract: u32,
    /// Hand-row cutoff.
    pub upto: i64,
    /// Eligible six-seat records in the window.
    pub eligible: usize,
    /// Records excluded for missing/invalid stacks or another table size.
    pub excluded: usize,
    /// Identity of the exact objective, included in ledger and target scope.
    pub digest: String,
    /// Chip stacks at the simulator's 20-chip big blind; hero first.
    pub layouts: Vec<[i64; 6]>,
}

#[derive(Deserialize)]
struct StackHand {
    bb: i64,
    players: Vec<(usize, String)>,
    stacks: Vec<(usize, i64)>,
}

fn layout(hero: Option<i64>, json: &str) -> Option<[i64; 6]> {
    let hero = usize::try_from(hero?).ok()?;
    let h: StackHand = serde_json::from_str(json).ok()?;
    if h.bb <= 0 || h.players.len() != 6 || h.stacks.len() != 6 {
        return None;
    }
    let mut seats: Vec<_> = h.players.iter().map(|(s, _)| *s).collect();
    seats.sort_unstable();
    seats.dedup();
    if seats.len() != 6 || !seats.contains(&hero) {
        return None;
    }
    seats.retain(|s| *s != hero);
    seats.insert(0, hero);
    let mut out = [0; 6];
    for (i, seat) in seats.into_iter().enumerate() {
        let mut matches = h.stacks.iter().filter(|(s, _)| *s == seat);
        let chips = matches.next()?.1;
        if matches.next().is_some() || chips <= 0 {
            return None;
        }
        let normalized = (chips as i128 * 20 / h.bb as i128).max(1);
        // Reject corrupt/unbounded records rather than overflowing engine pot arithmetic.
        if normalized > 20_000_000 {
            return None;
        }
        out[i] = normalized as i64;
    }
    Some(out)
}

impl Fixture {
    /// Freeze a bounded, treatment-excluded distribution at the search watermark.
    pub fn capture(store: &Store, upto: i64) -> anyhow::Result<Self> {
        let rows = store.stack_samples(upto, WINDOW)?;
        let eligible: Vec<_> = rows.iter().filter_map(|(hero, json)| layout(*hero, json)).collect();
        let count = eligible.len();
        let take = count.min(SAMPLE);
        let layouts = if count == 0 { vec![[2000; 6]] } else { (0..take).map(|i| eligible[i * count / take]).collect() };
        let digest = format!("stacks-v{CONTRACT}-{}", super::run::digest(&serde_json::to_string(&layouts)?));
        Ok(Self { contract: CONTRACT, upto, eligible: count, excluded: rows.len() - count, digest, layouts })
    }

    /// Compact experiment evidence; defaults are explicit when no valid live fixture exists.
    pub fn evidence(&self) -> serde_json::Value {
        serde_json::json!({"contract":self.contract,"id":self.digest,"cutoff":self.upto,"eligible":self.eligible,
            "excluded":self.excluded,"sampled":self.layouts.len(),"basis":if self.eligible>0 {"recorded six-seat starting stacks"} else {"default 100bb: no valid six-seat stack records"}})
    }

    /// Legacy or malformed persisted fixtures cannot contribute to current paired accumulators.
    pub fn valid(&self) -> bool {
        self.contract == CONTRACT
            && !self.layouts.is_empty()
            && self.layouts.len() <= SAMPLE
            && self.layouts.iter().flatten().all(|s| (1..=20_000_000).contains(s))
            && serde_json::to_string(&self.layouts).is_ok_and(|j| self.digest == format!("stacks-v{CONTRACT}-{}", super::run::digest(&j)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv10_store::store::{HandRow, HandTag, TREATMENT_ARM};
    #[test]
    fn capture_freezes_hero_asymmetry_cutoff_and_provenance() {
        let shared = crate::live::Shared::for_test("stack-capture", &["A"]);
        let json = serde_json::json!({"bb":40,"players":[[0,"a"],[1,"b"],[2,"c"],[3,"hero"],[4,"d"],[5,"e"]],"stacks":[[0,100],[1,200],[2,300],[3,20000],[4,400],[5,500]]}).to_string();
        let hand =
            |id: &str| HandRow { bot: "A".into(), hand_id: id.into(), hero_seat: Some(3), summary: json.clone(), ..Default::default() };
        let cutoff = shared.store.insert_hand_tagged(&hand("ordinary"), None).unwrap();
        shared
            .store
            .insert_hand_tagged(&hand("treatment"), Some(&HandTag { target: "x".into(), arm: TREATMENT_ARM.into(), record: "{}".into() }))
            .unwrap();
        shared.store.insert_hand_tagged(&hand("future"), None).unwrap();
        let frozen = Fixture::capture(&shared.store, cutoff).unwrap();
        assert_eq!(frozen.layouts, vec![[10000, 50, 100, 150, 200, 250]]);
        assert_eq!((frozen.eligible, frozen.excluded), (1, 0));
        assert!(frozen.valid());
        let all = shared.store.stack_samples(i64::MAX, 1).unwrap();
        assert_eq!(all.len(), 1);
        let latest = Fixture::capture(&shared.store, i64::MAX).unwrap();
        assert_eq!(latest.eligible, 2, "treatment excluded before sampling");
        assert_eq!(Fixture::capture(&shared.store, cutoff).unwrap().digest, frozen.digest, "the frozen cutoff is reproducible");
    }

    #[test]
    fn invalid_or_other_size_records_are_excluded_and_empty_basis_is_explicit() {
        assert!(layout(Some(0), r#"{"bb":20,"players":[],"stacks":[]}"#).is_none());
        assert!(layout(None, "{}").is_none());
        let shared = crate::live::Shared::for_test("empty-stack-capture", &["A"]);
        let fixture = Fixture::capture(&shared.store, 0).unwrap();
        assert!(fixture.valid());
        assert!(fixture.evidence()["basis"].as_str().unwrap().contains("no valid"));
    }

    #[test]
    fn incompatible_ledger_measurements_do_not_seed_a_new_stack_objective() {
        use crate::search_ledger::{self, Ledger, LedgerEntry};
        let shared = crate::live::Shared::for_test("stack-ledger", &["A"]);
        let mut old = Ledger { champion: "sv10-ev-1".into(), refit_rowid: 500, ..Default::default() };
        old.entries.insert("dead".into(), LedgerEntry { hands: 10000, mean_bb: -1.0, se_bb: 0.0, differing: 10 });
        search_ledger::save(&shared.store, &old).unwrap();
        let fresh = search_ledger::load_evaluated(&shared.store, &old.champion, 500, "recorded-fixture");
        assert!(fresh.entries.is_empty());
        let mut current = fresh;
        current.entries.insert("current".into(), LedgerEntry::default());
        search_ledger::save(&shared.store, &current).unwrap();
        assert_eq!(search_ledger::load_evaluated(&shared.store, &old.champion, 500, "recorded-fixture").entries.len(), 1);
        assert!(search_ledger::load_evaluated(&shared.store, &old.champion, 500, "different-fixture").entries.is_empty());
    }
}

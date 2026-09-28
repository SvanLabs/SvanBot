//! Why search candidates die, rolled up by hour for the dashboard (#317).
//!
//! The rejection ledger (`crate::search_ledger`) answers "is this transition decided dead". It does
//! not answer "what is killing candidates": every entry it records, and every entry in
//! `learner.experiments`, is a rejection with a prose rationale, so the panel could only ever show a
//! wall of `rejected`. Whether the search is starved of candidates that move a decision, churning
//! transitions the ledger already barred, or losing every survivor on fresh deals was not readable
//! at all — three causes that call for three different responses.
//!
//! This is the rollup: one counter per hour per outcome, written where the candidate dies. It is
//! deliberately *not* scoped to the champion version and the evidence watermark the way the ledger
//! is — that scope exists to invalidate evidence, and a promotion must not erase the hours the
//! search spent losing. Buckets older than [`KEEP_HOURS`] are dropped on every write, so the whole
//! thing is bounded at two days of counters whatever the fleet does.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sv10_store::store::Store;

use super::now;

/// KV key holding the serialized [`Funnel`].
pub const FUNNEL_KEY: &str = "learner.search-funnel.v1";
/// Hours of buckets kept. Twice the window the dashboard reads, so a summary never loses its tail
/// between two writes.
pub const KEEP_HOURS: i64 = 48;
/// Hours the dashboard sums, and the only window this rollup claims.
pub const WINDOW_HOURS: i64 = 24;
/// Candidates the ledger kept out of the pool: decided dead under this champion and evidence.
pub const BARRED: &str = "ledger/barred";
/// Candidates the search was handed after that filter.
pub const PROPOSED: &str = "search/proposed";
/// Candidates that never changed a decision.
pub const NO_EFFECT: &str = "search/no-effect";
/// Candidates whose 95% upper bound sat below the +1 bb/100 bar.
pub const BELOW_BAR: &str = "search/below-bar";
/// Candidates the round ranking left out — alive, but not in the better half.
pub const HALVED_OUT: &str = "search/halved-out";
/// Candidates promoted after fresh-deal confirmation.
pub const PROMOTED: &str = "promotion/promoted";

/// The hour bucket an instant belongs to.
fn hour(ts: f64) -> i64 {
    (ts / 3600.0).floor() as i64
}

/// One hour of outcomes: how many times each happened, and in which knob's name.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Bucket {
    /// Counts by outcome key, one of the constants in this module or `confirm/<code>`.
    pub outcomes: BTreeMap<String, u32>,
    /// Counts by knob, for the outcomes that happened to a named candidate. Absent in buckets
    /// stored before it.
    #[serde(default)]
    pub knobs: BTreeMap<String, u32>,
}

/// The rollup, one bucket per hour the learner wrote in.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Funnel {
    /// Hour index (`unix seconds / 3600`) to its bucket.
    pub buckets: BTreeMap<i64, Bucket>,
}

impl Funnel {
    /// Count `count` outcomes of `key` at `ts`, attributed to `knob` when one names the candidate,
    /// and drop the buckets that have aged out of [`KEEP_HOURS`].
    pub fn record(&mut self, ts: f64, key: &str, knob: Option<&str>, count: u32) {
        if count == 0 {
            return;
        }
        let bucket = self.buckets.entry(hour(ts)).or_default();
        *bucket.outcomes.entry(key.to_string()).or_default() += count;
        if let Some(knob) = knob {
            *bucket.knobs.entry(knob.to_string()).or_default() += count;
        }
        self.trim(ts);
    }

    /// Drop the buckets older than [`KEEP_HOURS`]. Called on every write, so the stored value can
    /// only grow with time, not with the fleet's activity.
    pub fn trim(&mut self, now: f64) {
        let oldest = hour(now) - KEEP_HOURS + 1;
        self.buckets.retain(|h, _| *h >= oldest);
    }

    /// The last [`WINDOW_HOURS`], ranked: `total` is every outcome counted, `outcomes` and `knobs`
    /// are count-ordered so the panel reads the biggest cause first.
    pub fn summary(&self, now: f64) -> Value {
        let oldest = hour(now) - WINDOW_HOURS + 1;
        let (mut outcomes, mut knobs): (BTreeMap<&str, u32>, BTreeMap<&str, u32>) = (BTreeMap::new(), BTreeMap::new());
        for bucket in self.buckets.range(oldest..).map(|(_, b)| b) {
            for (key, n) in &bucket.outcomes {
                *outcomes.entry(key.as_str()).or_default() += n;
            }
            for (knob, n) in &bucket.knobs {
                *knobs.entry(knob.as_str()).or_default() += n;
            }
        }
        let total = outcomes.values().sum::<u32>();
        json!({"hours": WINDOW_HOURS, "total": total, "outcomes": ranked(outcomes), "knobs": ranked(knobs)})
    }
}

/// `{key, count}` rows, biggest first, ties by key so two reads of one state agree.
fn ranked(counts: BTreeMap<&str, u32>) -> Vec<Value> {
    let mut rows: Vec<(&str, u32)> = counts.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    rows.into_iter().map(|(key, count)| json!({"key": key, "count": count})).collect()
}

/// Load the rollup. An unreadable or absent one reads as empty rather than failing the cycle: a
/// missing count costs the dashboard a number, and nothing else reads this key.
pub fn load(store: &Store) -> Funnel {
    store.get_kv(FUNNEL_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Count one outcome and persist it. A write that does not land loses a count, not the cycle, so it
/// warns — the same bargain `push_experiment` makes with the experiment list.
pub fn note(store: &Store, key: &str, knob: Option<&str>, count: u32) {
    let mut funnel = load(store);
    funnel.record(now(), key, knob, count);
    let stored = serde_json::to_string(&funnel).map_err(anyhow::Error::from).and_then(|j| store.put_kv(FUNNEL_KEY, &j));
    if let Err(e) = stored {
        tracing::warn!("search funnel not recorded ({e})");
    }
}

/// The rollup as `training.search_funnel` for the dashboard.
pub fn dashboard(store: &Store) -> Value {
    load(store).summary(now())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: f64 = 3600.0;

    #[test]
    fn outcomes_land_in_the_hour_they_happened_in() {
        let mut f = Funnel::default();
        f.record(100.0 * HOUR, NO_EFFECT, Some("open_bb"), 2);
        f.record(100.0 * HOUR + 60.0, NO_EFFECT, Some("fold_scale"), 1);
        f.record(101.0 * HOUR, BELOW_BAR, None, 3);
        assert_eq!(f.buckets.len(), 2);
        assert_eq!(f.buckets[&100].outcomes[NO_EFFECT], 3);
        assert_eq!(f.buckets[&100].knobs["open_bb"], 2);
        assert_eq!(f.buckets[&101].outcomes[BELOW_BAR], 3);
        assert!(f.buckets[&101].knobs.is_empty());
    }

    #[test]
    fn nothing_is_counted_when_nothing_died() {
        let mut f = Funnel::default();
        f.record(100.0 * HOUR, BARRED, None, 0);
        assert!(f.buckets.is_empty());
    }

    #[test]
    fn buckets_age_out_of_the_kept_window_on_the_next_write() {
        let mut f = Funnel::default();
        f.record(100.0 * HOUR, NO_EFFECT, None, 1);
        f.record((100.0 + KEEP_HOURS as f64) * HOUR, NO_EFFECT, None, 1);
        assert_eq!(f.buckets.len(), 1);
        assert_eq!(f.buckets[&(100 + KEEP_HOURS)].outcomes[NO_EFFECT], 1);
    }

    #[test]
    fn the_summary_sums_the_window_and_leaves_the_rest_out() {
        let now = 200.0 * HOUR;
        let mut f = Funnel::default();
        // Inside the window: 24 hours back from `now`, inclusive.
        f.record(now - (WINDOW_HOURS as f64 - 1.0) * HOUR, NO_EFFECT, Some("open_bb"), 4);
        f.record(now, HALVED_OUT, Some("open_bb"), 2);
        // One hour older than the window, and still inside what is kept.
        f.record(now - WINDOW_HOURS as f64 * HOUR, PROMOTED, Some("fold_scale"), 9);
        let s = f.summary(now);
        assert_eq!(s["hours"], WINDOW_HOURS);
        assert_eq!(s["total"], 6);
        assert_eq!(s["outcomes"][0]["key"], NO_EFFECT);
        assert_eq!(s["outcomes"][0]["count"], 4);
        assert_eq!(s["outcomes"][1]["key"], HALVED_OUT);
        assert_eq!(s["knobs"][0], json!({"key": "open_bb", "count": 6}));
        assert_eq!(s["knobs"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn the_biggest_cause_reads_first_and_ties_are_stable() {
        let now = 200.0 * HOUR;
        let mut f = Funnel::default();
        f.record(now, BELOW_BAR, None, 2);
        f.record(now, NO_EFFECT, None, 7);
        f.record(now, HALVED_OUT, None, 7);
        let s = f.summary(now);
        let keys: Vec<&str> = s["outcomes"].as_array().unwrap().iter().map(|r| r["key"].as_str().unwrap()).collect();
        assert_eq!(keys, vec![HALVED_OUT, NO_EFFECT, BELOW_BAR]);
    }

    #[test]
    fn a_stored_rollup_survives_the_round_trip() {
        let mut f = Funnel::default();
        f.record(100.0 * HOUR, NO_EFFECT, Some("open_bb"), 5);
        let back: Funnel = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back.buckets, f.buckets);
        // A bucket written before the knob map existed still loads.
        let old: Funnel = serde_json::from_str(r#"{"buckets":{"100":{"outcomes":{"search/no-effect":2}}}}"#).unwrap();
        assert_eq!(old.buckets[&100].outcomes[NO_EFFECT], 2);
        assert!(old.buckets[&100].knobs.is_empty());
    }
}

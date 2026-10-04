//! Gene transfer between lineages (ADR 0002, stage 3). A transition that cleared the gate for one
//! lineage is queued for the others, and each tests it in its next search against its own champion
//! with the same gate: it is offered as a candidate, never copied.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sv10_core::policy::Params;
use sv10_store::store::Store;

/// The queue of offers still waiting for some lineage.
pub const TRANSFERS_KEY: &str = "learner.transfers.v1";
/// Candidates a lineage got from another carry this knob prefix, so they are never offered on again.
pub const PREFIX: &str = "transfer/";

/// One promoted change and the lineages that have not yet been offered it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transfer {
    /// The lineage that promoted it.
    pub from: String,
    pub knob: String,
    pub old: f64,
    pub new: f64,
    /// The `Params` fields the promotion changed, as JSON.
    pub delta: Map<String, Value>,
    /// Lineages not yet offered it.
    pub to: Vec<String>,
}

/// The fields that differ between two parameter sets.
pub fn delta(before: &Params, after: &Params) -> Map<String, Value> {
    let (Ok(Value::Object(b)), Ok(Value::Object(a))) = (serde_json::to_value(before), serde_json::to_value(after)) else {
        return Map::new();
    };
    a.into_iter().filter(|(k, v)| b.get(k) != Some(v)).collect()
}

/// `champion` with the change applied, or `None` when it changes nothing or does not fit.
pub fn apply(champion: &Params, change: &Map<String, Value>) -> Option<Params> {
    let Ok(Value::Object(mut fields)) = serde_json::to_value(champion) else { return None };
    fields.extend(change.clone());
    let moved: Params = serde_json::from_value(Value::Object(fields)).ok()?;
    (!delta(champion, &moved).is_empty()).then_some(moved)
}

fn load(store: &Store) -> Vec<Transfer> {
    store.get_kv(TRANSFERS_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Queue a promotion for `to`. A promotion that changed nothing, or was itself a transfer, is not offered.
pub fn offer(store: &Store, from: &str, knob: &str, (old, new): (f64, f64), before: &Params, after: &Params, to: Vec<String>) {
    let delta = delta(before, after);
    if knob.starts_with(PREFIX) || delta.is_empty() || to.is_empty() {
        return;
    }
    let mut queue = load(store);
    queue.push(Transfer { from: from.to_string(), knob: knob.to_string(), old, new, delta, to });
    if let Err(e) = serde_json::to_string(&queue).map_err(anyhow::Error::from).and_then(|j| store.put_kv(TRANSFERS_KEY, &j)) {
        tracing::warn!("gene transfer of {knob} not queued ({e})");
    }
}

/// What is waiting for `lane`, removed from the queue: each offer is made once.
pub fn take(store: &Store, lane: &str) -> Vec<Transfer> {
    let mut queue = load(store);
    let mine: Vec<Transfer> = queue.iter().filter(|t| t.to.iter().any(|l| l == lane)).cloned().collect();
    if mine.is_empty() {
        return mine;
    }
    for t in &mut queue {
        t.to.retain(|l| l != lane);
    }
    queue.retain(|t| !t.to.is_empty());
    if let Err(e) = serde_json::to_string(&queue).map_err(anyhow::Error::from).and_then(|j| store.put_kv(TRANSFERS_KEY, &j)) {
        tracing::warn!("gene transfers for {lane} not marked offered ({e})");
    }
    mine
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_promotion_is_offered_once_to_each_other_lineage_as_a_change_not_a_copy() {
        let shared = crate::live::Shared::for_test("transfer", &["A"]);
        let store = &shared.store;
        let before = Params::default();
        let after = Params { call_margin: 0.31, ..Params::default() };
        offer(store, "A", "call_margin", (0.0, 0.31), &before, &after, vec!["B".into(), "C".into()]);
        offer(store, "A", "transfer/call_margin", (0.0, 0.31), &before, &after, vec!["B".into()]);
        offer(store, "A", "call_margin", (0.0, 0.0), &before, &before, vec!["B".into()]);
        let for_b = take(store, "B");
        assert_eq!(for_b.len(), 1, "a transfer is not offered on, and a no-op is not offered");
        assert_eq!(for_b[0].delta.keys().collect::<Vec<_>>(), ["call_margin"]);
        assert!(take(store, "B").is_empty(), "offered once");
        assert_eq!(take(store, "C").len(), 1);
        assert!(store.get_kv(TRANSFERS_KEY).unwrap().is_some_and(|q| q == "[]"));
        // B keeps its own other knobs: only the changed field moves.
        let b_champion = Params { fold_scale: 0.7, ..Params::default() };
        let moved = apply(&b_champion, &for_b[0].delta).unwrap();
        assert_eq!((moved.fold_scale, moved.call_margin), (0.7, 0.31));
        assert!(apply(&moved, &for_b[0].delta).is_none(), "already there: nothing to test");
    }
}

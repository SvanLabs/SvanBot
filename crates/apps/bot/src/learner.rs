//! The learner's jobs as short resumable steps (0334): an evidence refresh or a champion search
//! is a stored [`run::Run`] that the `learner` process advances one step at a time, each at most
//! about [`run::STEP_TARGET_SECS`] of work, so nothing it does holds the machine for more than
//! two minutes and a release waits at most one step.

pub mod funnel;
pub mod pool;
pub mod progress;
pub mod refit;
pub mod run;
pub mod search;

use serde_json::Value;
use sv10_core::model::ModelStore;
use sv10_core::policy::Params;
use sv10_store::store::Store;

use crate::experiment::target::{self as experiment_target, TARGETS_KEY, Target};
use crate::search_ledger;

/// Opponents need this many observed hands to join the clone pool.
pub const MIN_OPPONENT_HANDS: f32 = 30.0;
/// Opponent-model snapshot refreshed on the dashboard hand threshold and reused by searches.
pub const POPULATION_MODELS_KEY: &str = "learner.population-models.v1";
/// A stored neural model younger than this is reused instead of retrained.
pub const NN_REUSE_SECS: f64 = 1800.0;

/// What a step needs from the process: the store, the repo root and the machine's sizing.
pub struct Ctx<'a> {
    /// The live store.
    pub store: &'a Store,
    /// Repository root (`artifacts/`, `target/release/`).
    pub root: &'a std::path::Path,
    /// Learner threads (the rayon pool).
    pub threads: usize,
    /// Tables per evaluation unit.
    pub tables: usize,
    /// Hands per table.
    pub hands: usize,
    /// Decision samples the simulated hero uses.
    pub decision_samples: usize,
}

/// Seconds since the epoch.
pub fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// The fleet's current opponent models.
pub fn load_models(store: &Store) -> ModelStore {
    store.get_kv(crate::MODELS_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// The champion's stored parameters.
pub fn load_params(store: &Store) -> Params {
    store.get_kv(crate::PARAMS_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// The promoted versions, oldest first.
pub fn load_lineage(store: &Store) -> Vec<String> {
    store
        .get_kv(crate::LEARNER_LINEAGE_KEY)
        .ok()
        .flatten()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| vec!["sv10-ev-1".to_string()])
}

/// Publish the learner's status for the dashboard. A write that does not land says so (issue #326):
/// the update progress reads this key, and a status stuck on the previous run reports a swap that
/// already happened as still in flight.
pub fn status(store: &Store, mut v: Value) {
    // The build that wrote it: the dashboard's update progress confirms the learner's swap (0236).
    if let Some(o) = v.as_object_mut() {
        o.insert("commit".into(), Value::from(crate::BUILD_COMMIT));
    }
    if let Err(e) = store.put_kv(crate::LEARNER_STATUS_KEY, &v.to_string()) {
        tracing::warn!("the learner status was not published ({e})");
    }
}

/// Record an experiment for the dashboard (newest first, 40 kept). The list is read back and
/// rewritten whole, so a write that does not land loses this experiment rather than deferring it:
/// the next call reads the list as it was before, and the entry is gone for good (issue #326).
pub fn push_experiment(store: &Store, e: Value) {
    if let Err(e) = store.update_kv(crate::LEARNER_EXPERIMENTS_KEY, |old| {
        let mut list: Vec<Value> = old.and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default();
        list.insert(0, e);
        list.truncate(40);
        Ok(serde_json::to_string(&list)?)
    }) {
        tracing::warn!("an experiment did not reach the dashboard list ({e})");
    }
}

/// Publish the experiment pair's target queue for this scope. Both step sizes of this champion's
/// search are offered, so a transition the ledger measured in either kind of cycle can be replayed
/// exactly. A failed write leaves the older queue, which the fleet refuses once its scope moves on.
pub fn publish_targets(
    store: &Store,
    ledger: &search_ledger::Ledger,
    champion: &Params,
    cycle: u64,
    confirming: Option<Target>,
    (tables, hands): (usize, usize),
) {
    let mut proposals = pool::challengers(champion, cycle);
    proposals.extend(pool::challengers(champion, cycle + 1));
    let queue = experiment_target::build_queue(ledger, &proposals, confirming, tables, hands, now());
    if let Err(e) = serde_json::to_string(&queue).map_err(anyhow::Error::from).and_then(|j| store.put_kv(TARGETS_KEY, &j)) {
        tracing::warn!("cycle {cycle}: experiment targets not published ({e})");
    }
}

/// Log a step's time, and warn when it broke the two-minute budget for live-system work.
pub fn log_step(what: &str, took: std::time::Duration) {
    match crate::jobs::over_budget(what, took) {
        Some(warning) => tracing::warn!("{warning}"),
        None => tracing::info!("{what} took {:.0}s", took.as_secs_f64()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn concurrent_rollups_keep_every_count_and_experiment() {
        let dir = std::env::temp_dir().join(format!("sv10-learner-rollups-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = Store::open(&dir.join("state.db")).unwrap();
        let second = Store::open(&dir.join("state.db")).unwrap();
        std::thread::scope(|scope| {
            for (writer, store) in [&first, &second].into_iter().enumerate() {
                scope.spawn(move || {
                    for i in 0..100 {
                        funnel::note(store, funnel::PROPOSED, None, 1);
                        if i < 20 {
                            push_experiment(store, json!({"writer": writer, "i": i}));
                        }
                        std::thread::yield_now();
                    }
                });
            }
        });
        assert_eq!(funnel::dashboard(&first)["total"], 200);
        let list: Vec<Value> = serde_json::from_str(&first.get_kv(crate::LEARNER_EXPERIMENTS_KEY).unwrap().unwrap()).unwrap();
        assert_eq!(list.len(), 40);
        for writer in 0..2 {
            for i in 0..20 {
                assert!(list.contains(&json!({"writer": writer, "i": i})));
            }
        }
        push_experiment(&second, json!({"newest": true}));
        let list: Vec<Value> = serde_json::from_str(&first.get_kv(crate::LEARNER_EXPERIMENTS_KEY).unwrap().unwrap()).unwrap();
        assert_eq!(list.len(), 40);
        assert_eq!(list[0], json!({"newest": true}));
        std::fs::remove_dir_all(dir).unwrap();
    }
}

// Generated-by: codex/gpt-6
// Offline evidence harness; run only against the frozen experiment copy.
use anyhow::{Context, Result};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};
use sv10_bot::StoredNet;
extern crate self as sv10_engine;
extern crate self as sv10_model;
extern crate self as sv10_nn;
pub use sv10_core::{agents, engine, model, nn, policy, situation};
// The study's other arms are helpers this entry point does not call; they stay with the study.
#[allow(dead_code)]
#[path = "study_sim.rs"]
mod study_sim;
use sv10_core::{agents::live_pool, model::ModelStore, policy::Params};
use sv10_store::store::Store;
fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(args.get(2).context("mode root candidate seed")?);
    assert!(root.to_string_lossy().contains("neural-fixes/experiment"), "offline copy required");
    let store = Store::open(&root.join("artifacts/svanbot10.db"))?;
    let models: ModelStore = serde_json::from_str(&store.get_kv("models.v1")?.context("models")?)?;
    let candidate_path = root.join("candidate.json");
    let seed: u64 = args.get(3).context("seed")?.parse()?;
    let candidate: StoredNet = serde_json::from_slice(&std::fs::read(candidate_path)?)?;
    let params: Params = serde_json::from_str(&store.get_kv("params.v1")?.context("params")?)?;
    let opponents = live_pool(&models, 30.0, 16, seed);
    let mut layouts = Vec::new();
    for (hero, raw) in store.stack_samples(i64::MAX, 4096)? {
        let Some(hero) = hero else { continue };
        let h: model::HandSummary = serde_json::from_str(&raw)?;
        if h.stacks.len() != 6 || h.bb <= 0 {
            continue;
        }
        let mut seats: Vec<_> = h.stacks.iter().map(|(s, n)| (*s, *n * 20 / h.bb)).collect();
        seats.sort_by_key(|(s, _)| (s + 6 - hero as usize) % 6);
        let layout: [i64; 6] = seats.iter().map(|(_, n)| *n).collect::<Vec<_>>().try_into().unwrap();
        if layout.iter().all(|n| *n > 0) {
            layouts.push(layout)
        }
    }
    anyhow::ensure!(!layouts.is_empty(), "no recorded stacks");
    let step = (layouts.len() / 256).max(1);
    let layouts: Vec<_> = layouts.into_iter().step_by(step).take(256).collect();
    let arm = study_sim::Arm { params: &params, nn: Some(Arc::new(candidate.net)) };
    let logs = study_sim::observed_logs(&arm, &opponents, &models, &layouts, seed);
    println!("{}", json!({"seed":seed,"logs":logs,"layouts":layouts.len(),"opponents":opponents.len()}));
    Ok(())
}

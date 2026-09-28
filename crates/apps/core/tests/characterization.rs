//! Characterization tests: golden snapshots of what the decision core does today, so a refactor,
//! file split or speed change that alters behaviour fails here. Situations come from a seeded
//! simulation, so they cover every street, position and pot type the simulator produces.
//!
//! An intended behaviour change regenerates the snapshot: `UPDATE_GOLDEN=1 cargo test --release
//! -p sv10-core --test characterization`, and the diff of `tests/golden/core.json` is reviewed.

use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use sv10_core::agents::{ARCHETYPES, Agent, ArchetypeAgent, archetype};
use sv10_core::engine::Action;
use sv10_core::model::{HandSummary, ModelStore};
use sv10_core::oprange::{RangeParams, estimate_ranges};
use sv10_core::policy::{Params, decide_with};
use sv10_core::sim::run_table_observed;
use sv10_core::situation::Situation;
use sv10_rng::SeedableRng;
use sv10_rng::rngs::SmallRng;

/// Plays like an archetype and records every situation and finished hand it sees.
struct Recorder {
    inner: ArchetypeAgent,
    situations: Arc<Mutex<Vec<Situation>>>,
    hands: Arc<Mutex<Vec<HandSummary>>>,
}

impl Agent for Recorder {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn act(&mut self, sit: &Situation, rng: &mut SmallRng) -> Action {
        self.situations.lock().unwrap().push(sit.clone());
        self.inner.act(sit, rng)
    }
    fn observe(&mut self, hand: &HandSummary) {
        self.hands.lock().unwrap().push(hand.clone());
    }
}

fn corpus() -> (Vec<Situation>, Vec<HandSummary>, ModelStore, Value) {
    let situations = Arc::new(Mutex::new(Vec::new()));
    let hands = Arc::new(Mutex::new(Vec::new()));
    let mut agents: Vec<Box<dyn Agent>> = vec![Box::new(Recorder {
        inner: ArchetypeAgent { a: archetype("tag"), label: "hero".into() },
        situations: situations.clone(),
        hands: hands.clone(),
    })];
    for (i, k) in ["station", "maniac", "nit", "lag", "rock"].iter().enumerate() {
        agents.push(Box::new(ArchetypeAgent { a: archetype(k), label: format!("{k}{i}") }));
    }
    let mut models = ModelStore::default();
    let tallies = run_table_observed(&mut agents, 120, 100, 20260915, Some(&mut models));
    let mut t: Vec<(String, f64)> = tallies.into_iter().map(|(k, v)| (k, v.net)).collect();
    t.sort_by(|a, b| a.0.cmp(&b.0));
    let sits = situations.lock().unwrap().clone();
    let hs = hands.lock().unwrap().clone();
    (sits, hs, models, json!(t))
}

fn round(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn snapshot() -> Value {
    let (sits, hands, models, tallies) = corpus();
    // A spread of situations across the run: every 4th, at most 40.
    let picked: Vec<&Situation> = sits.iter().step_by(4).take(40).collect();
    let params = Params::default();
    let decisions: Vec<Value> = picked
        .iter()
        .enumerate()
        .map(|(i, sit)| {
            let mut rng = SmallRng::seed_from_u64(1_000 + i as u64);
            let d = decide_with(sit, &models, &params, None, &mut rng);
            json!({
                "street": sit.street.name(), "pot": sit.pot, "call": sit.call_amount,
                "action": d.action_name, "amount": d.amount, "equity": d.equity.map(round),
                "candidates": d.candidates.iter().map(|c| json!([c.action, c.amount, round(c.ev), round(c.fold_prob)])).collect::<Vec<_>>(),
            })
        })
        .collect();
    let ranges: Vec<Value> = picked
        .iter()
        .map(|sit| {
            let r = estimate_ranges(sit, &models, &RangeParams::default());
            let mut seats: Vec<(&usize, &sv10_core::range::Range)> = r.iter().collect();
            seats.sort_by_key(|(s, _)| **s);
            json!(
                seats
                    .iter()
                    .map(|(s, r)| {
                        let total: f64 = r.w.iter().map(|&w| w as f64).sum();
                        let mut top: Vec<(usize, f32)> = r.w.iter().copied().enumerate().collect();
                        top.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
                        json!({"seat": s, "total": round(total), "top": top.iter().take(5).map(|(i, _)| *i).collect::<Vec<_>>()})
                    })
                    .collect::<Vec<_>>()
            )
        })
        .collect();
    let archetypes: Vec<Value> = ARCHETYPES
        .iter()
        .map(|k| {
            let mut agent = ArchetypeAgent { a: archetype(k), label: k.to_string() };
            let mut rng = SmallRng::seed_from_u64(77);
            let acts: Vec<String> = picked.iter().map(|sit| format!("{:?}", agent.act(sit, &mut rng))).collect();
            json!({"kind": k, "actions": acts})
        })
        .collect();
    // Hands that reached the flop, so board-texture features are exercised.
    let features: Vec<Value> = hands
        .iter()
        .filter(|h| h.history.iter().any(|r| r.street.index() > 0))
        .take(12)
        .map(|h| {
            let samples = sv10_core::features::samples_from_hand(h, &models, &["hero".to_string()]);
            let first: Vec<Vec<f64>> = samples.iter().map(|(s, _)| s.x.iter().map(|&v| round(v as f64)).collect()).collect();
            json!({"samples": samples.len(), "x": first, "labels": samples.iter().map(|(s, _)| s.label).collect::<Vec<_>>()})
        })
        .collect();
    let opponents: Vec<(sv10_core::agents::Archetype, f64)> = ARCHETYPES.iter().map(|k| (archetype(k), 1.0)).collect();
    let challenger = Params { fold_scale: 0.7, open_bb: 3.0, ..Params::default() };
    let paired = sv10_core::sim::paired_eval(&Params::default(), &challenger, &opponents, &ModelStore::default(), None, 2, 150, 100, 4242);
    let paired = json!({"hands": paired.hands, "mean_bb": round(paired.mean_bb * 100.0), "se_bb": round(paired.se_bb * 100.0), "differing": paired.differing});
    json!({"paired": paired, "situations": sits.len(), "hands": hands.len(), "tallies": tallies, "decisions": decisions, "ranges": ranges, "archetypes": archetypes, "features": features})
}

/// Numbers compare with a small relative tolerance (instruction-set differences in float
/// contraction); everything else must match exactly.
fn same(a: &Value, b: &Value, path: &str, diffs: &mut Vec<String>) {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            if (x - y).abs() > 1e-3 * x.abs().max(y.abs()).max(1.0) {
                diffs.push(format!("{path}: {x} != {y}"));
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                same(p, q, &format!("{path}[{i}]"), diffs);
            }
        }
        (Value::Object(x), Value::Object(y)) if x.len() == y.len() => {
            for (k, p) in x {
                match y.get(k) {
                    Some(q) => same(p, q, &format!("{path}.{k}"), diffs),
                    None => diffs.push(format!("{path}.{k}: missing")),
                }
            }
        }
        _ if a == b => {}
        _ => diffs.push(format!("{path}: {a} != {b}")),
    }
}

#[test]
fn decision_core_matches_golden_snapshot() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/core.json");
    let now = snapshot();
    // A missing golden is a failure, not a fresh start: only an intended change rewrites it (0225).
    assert!(
        path.exists() || std::env::var_os("UPDATE_GOLDEN").is_some(),
        "{} is missing; restore it or run with UPDATE_GOLDEN=1",
        path.display()
    );
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&now).unwrap()).unwrap();
        eprintln!("wrote {}", path.display());
        return;
    }
    let golden: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let mut diffs = Vec::new();
    same(&golden, &now, "$", &mut diffs);
    assert!(diffs.is_empty(), "{} differences from the golden snapshot, first: {:#?}", diffs.len(), &diffs[..diffs.len().min(15)]);
}

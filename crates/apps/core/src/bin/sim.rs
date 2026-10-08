//! `sim [hands_per_table] [tables] [stack_bb] [mix]` — policy vs archetype tables.
//! mix: "soft" (stations/maniacs heavy), "tough" (tag/lag/nit), or "all".

use rayon::prelude::*;
use std::collections::HashMap;
use sv10_core::agents::{ARCHETYPES, Agent, ArchetypeAgent, PolicyAgent, archetype};
use sv10_core::model::ModelStore;
use sv10_core::model::PlayerStats;
use sv10_core::sim::{Tally, run_table_observed};
use sv10_rng::SeedableRng;
use sv10_rng::rngs::SmallRng;
use sv10_rng::seq::SliceRandom;

fn paired(args: &[String]) {
    let tables: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(16);
    let hands: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1500);
    // SIM_A / SIM_B: Params JSON (missing fields default; e.g. {"range": {...}} changes only the range model).
    let parse = sv10_core::inputs::params;
    let (a, b) = (parse("SIM_A"), parse("SIM_B"));
    // SIM_MODELS=<ModelStore JSON>: play profile clones of the 16 most-observed live opponents, with
    // the policy using those models (the learner's population, 0099). SIM_STACK_BB sets stack depth.
    let stack_bb: i64 = std::env::var("SIM_STACK_BB").ok().and_then(|v| v.parse().ok()).unwrap_or(100);
    let seed: u64 = std::env::var("SIM_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(4242);
    let t0 = std::time::Instant::now();
    // SIM_NEURAL=<nn.response.v1 JSON>: the stored response network the policy prices its responses with (#908).
    let nn = std::env::var("SIM_NEURAL").ok().map(|path| std::sync::Arc::new(load_net(&path)));
    let r = match std::env::var("SIM_MODELS").ok() {
        Some(path) => {
            let models: ModelStore = load_models(&path);
            let opps = sv10_core::agents::live_pool(&models, 30.0, 16, seed);
            eprintln!("live pool: {} profile clones", opps.len());
            sv10_core::sim::paired_eval(&a, &b, &opps, &models, nn.clone(), tables, hands, stack_bb, seed)
        }
        None => {
            let opps: Vec<(sv10_core::agents::Archetype, f64)> = ARCHETYPES.iter().map(|k| (archetype(k), 1.0)).collect();
            sv10_core::sim::paired_eval(&a, &b, &opps, &ModelStore::default(), nn.clone(), tables, hands, stack_bb, seed)
        }
    };
    println!(
        "paired B-A over {} hands: {:+.2} bb/100 (95% {:+.2} .. {:+.2}) in {:.0}s",
        r.hands,
        r.mean_bb * 100.0,
        r.lower_95() * 100.0,
        r.upper_95() * 100.0,
        t0.elapsed().as_secs_f64()
    );
}

/// `sim variance [tables] [hands] [salts]` (0190): replay the paired A/B comparison with the same
/// cards and `salts` decision streams, and report how much of its variance is decision randomness
/// (the ceiling for AIVAT action corrections) versus card-conditional.
fn variance(args: &[String]) {
    let tables: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(16);
    let hands: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(600);
    let salts: usize = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(4).max(2);
    let parse = sv10_core::inputs::params;
    let (a, b) = (parse("SIM_A"), parse("SIM_B"));
    let stack_bb: i64 = std::env::var("SIM_STACK_BB").ok().and_then(|v| v.parse().ok()).unwrap_or(100);
    let seed: u64 = std::env::var("SIM_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(4242);
    // SIM_SALT_SEATS=hero|opponents|all: whose decision randomness varies between salts.
    let seats = match std::env::var("SIM_SALT_SEATS").as_deref() {
        Ok("hero") => 1,
        Ok("opponents") => !1,
        _ => sv10_core::sim::ALL_SEATS,
    };
    // SIM_LEARN=0: the hero does not update its opponent models during the run (isolates in-sim learning).
    let learn = std::env::var("SIM_LEARN").map(|v| v != "0").unwrap_or(true);
    let t0 = std::time::Instant::now();
    let v = match std::env::var("SIM_MODELS").ok() {
        Some(path) => {
            let models: ModelStore = load_models(&path);
            let opps = sv10_core::agents::live_pool(&models, 30.0, 16, seed);
            eprintln!("live pool: {} profile clones", opps.len());
            sv10_core::sim::paired_variance_split(&a, &b, &opps, &models, tables, hands, stack_bb, seed, salts, seats, learn)
        }
        None => {
            let opps: Vec<(sv10_core::agents::Archetype, f64)> = ARCHETYPES.iter().map(|k| (archetype(k), 1.0)).collect();
            sv10_core::sim::paired_variance_split(&a, &b, &opps, &ModelStore::default(), tables, hands, stack_bb, seed, salts, seats, learn)
        }
    };
    println!(
        "B-A {:+.2} bb/100 (±{:.2} per salt); variance per hand (bb^2) over {} cells x {} salts: total {:.4}, decision (within) {:.4} = {:.0}%, card-conditional {:.4}; \
         a perfect action correction would need {:.0}% of today's hands (x{:.2} faster) in {:.0}s",
        v.mean * 100.0,
        1.96 * v.se * 100.0,
        v.cells,
        v.salts,
        v.total,
        v.within,
        100.0 * v.within / v.total.max(1e-12),
        v.between,
        100.0 * v.hands_ratio(),
        1.0 / v.hands_ratio().max(1e-9),
        t0.elapsed().as_secs_f64()
    );
}

/// `SIM_MODELS`, or a clean exit naming the file and what is wrong with it — not a panic.
/// The response network out of a stored `nn.response.v1` value (`{"net": ..., ...}`).
fn load_net(path: &str) -> sv10_core::nn::Mlp {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("sim: SIM_NEURAL={path}: {e}");
        std::process::exit(2)
    });
    let value: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("sim: SIM_NEURAL={path} is not JSON: {e}");
        std::process::exit(2)
    });
    serde_json::from_value(value["net"].clone()).unwrap_or_else(|e| {
        eprintln!("sim: SIM_NEURAL={path} holds no response network: {e}");
        std::process::exit(2)
    })
}

fn load_models(path: &str) -> ModelStore {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("sim: SIM_MODELS={path}: {e}");
        std::process::exit(2)
    });
    serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("sim: SIM_MODELS={path} is not a ModelStore: {e}");
        std::process::exit(2)
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s == "paired").unwrap_or(false) {
        return paired(&args);
    }
    if args.get(1).map(|s| s == "variance").unwrap_or(false) {
        return variance(&args);
    }
    let hands: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let tables: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(8);
    let stack_bb: i64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(100);
    let mix = args.get(4).cloned().unwrap_or_else(|| "all".into());
    let pool: Vec<&str> = match mix.as_str() {
        "soft" => vec!["station", "station", "maniac", "rock", "tag", "lag"],
        "tough" => vec!["tag", "lag", "nit", "tag", "lag", "rock"],
        _ => ARCHETYPES.to_vec(),
    };
    let t0 = std::time::Instant::now();
    let results: Vec<(HashMap<String, Tally>, ModelStore)> = (0..tables)
        .into_par_iter()
        .map(|t| {
            let mut rng = SmallRng::seed_from_u64(t as u64 * 7919);
            let mut opps: Vec<&str> = pool.clone();
            opps.shuffle(&mut rng);
            let mut agents: Vec<Box<dyn Agent>> = vec![Box::new(PolicyAgent {
                label: "svanbot10".into(),
                models: ModelStore::default(),
                params: std::env::var("SIM_PARAMS").ok().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default(),
                learn: true,
                nn: None,
            })];
            for (i, k) in opps.iter().take(5).enumerate() {
                agents.push(Box::new(ArchetypeAgent { a: archetype(k), label: format!("{k}#{i}") }));
            }
            let mut obs = ModelStore::default();
            let r = run_table_observed(&mut agents, hands, stack_bb, 1_000 + t as u64, Some(&mut obs));
            (r, obs)
        })
        .collect();
    let mut by_kind: HashMap<String, Tally> = HashMap::new();
    let mut stats: HashMap<String, PlayerStats> = HashMap::new();
    for (r, obs) in &results {
        for (name, t) in r {
            let kind = name.split('#').next().unwrap().to_string();
            by_kind.entry(kind.clone()).or_default().merge(t);
        }
        for (name, st) in &obs.players {
            let kind = name.split('#').next().unwrap().to_string();
            stats.entry(kind).or_default().merge(st);
        }
    }
    let mut rows: Vec<_> = by_kind.into_iter().collect();
    rows.sort_by(|a, b| b.1.bb100(20.0).total_cmp(&a.1.bb100(20.0)));
    println!("{} tables x {} hands, {}bb stacks, mix={mix}, {:.1}s", tables, hands, stack_bb, t0.elapsed().as_secs_f64());
    for (k, t) in rows {
        let st = stats.get(&k).cloned().unwrap_or_default();
        let r = |c: &sv10_core::model::Counter| if c.opp > 0.0 { c.hit / c.opp } else { f32::NAN };
        println!(
            "{:>10}  {:>8.1} bb/100 ± {:>5.1} ({} h) | vpip {:.2} pfr {:.2} 3b {:.2} f3b {:.2} | bet1st {:.2}/{:.2}/{:.2} fvb {:.2}/{:.2}/{:.2} rvb {:.2} | wtsd {:.2} sd {:.0}",
            k,
            t.bb100(20.0),
            1.96 * t.se100(20.0),
            t.hands,
            r(&st.vpip),
            r(&st.pfr),
            r(&st.three_bet),
            r(&st.fold_to_3bet),
            r(&st.bet_first[0]),
            r(&st.bet_first[1]),
            r(&st.bet_first[2]),
            r(&st.fold_vs_bet[0]),
            r(&st.fold_vs_bet[1]),
            r(&st.fold_vs_bet[2]),
            r(&st.raise_vs_bet[0]),
            r(&st.wtsd),
            st.showdowns
        );
    }
}

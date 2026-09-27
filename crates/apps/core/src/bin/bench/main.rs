//! `bench [suite] [--repeat N] [--profile] [--fixture PATH]` — samples/sec as the fleet does the
//! work (0335). Three suites, fixed seeds, each with a checksum so a speed-only change is shown
//! to compute the same thing:
//!
//! - `learner`: the paired evaluation the learner runs (champion against a one-knob challenger on
//!   identical deals, against the population clones), in table runs per wall and per CPU second.
//! - `live`: one decision at a time at the live sample budget, over fixed simulated spots; the
//!   latency distribution p50..p999.
//! - `micro`: the 7-card evaluator, heads-up equity against a full range, the RNG.
//!
//! `draw` is the one suite that measures no workload: the deal-rejection loop against a branch-free
//! draw, in-process and paired, because the question is a few percent of a metric that is a few
//! percent of the deal (0354). It is not part of `all`.
//!
//! The workload is frozen in a fixture the learner writes from the live store
//! (`learner bench-fixture`: the champion as searches play it, the population models, the live
//! response net and the machine's sample budgets); without one the archetype pool and default
//! parameters stand in. One JSON line per repeat goes to stdout, so `scripts/bench-ab.py` can
//! alternate two builds and compare them on paired repeats. `--allocs` counts allocations (off by
//! default: counting costs time); `--profile` samples where the CPU time goes (build with
//! `--profile profiling` for source lines); `--threads N` sizes the pool.

// A measuring tool, never part of play: the counting allocator, the SIGPROF sampler and getrusage
// need `unsafe` (each block carries its SAFETY argument); the libraries it measures stay safe code.
#![allow(unsafe_code)]

mod alloc;
mod draw;
mod profiler;

use serde_json::{Value, json};
use std::time::Instant;
use sv10_core::agents::{Agent, Archetype, OpponentSpec, PolicyAgent, archetype, live_pool};
use sv10_core::cards::Card;
use sv10_core::engine::Action;
use sv10_core::model::{HandSummary, ModelStore};
use sv10_core::nn::Mlp;
use sv10_core::policy::{Params, decide_with};
use sv10_core::range::Range;
use sv10_core::sim::{Arm, paired_eval_arms};
use sv10_core::situation::Situation;
use sv10_rng::rngs::SmallRng;
use sv10_rng::{Rng, SeedableRng};

#[global_allocator]
static COUNTING: alloc::Counting = alloc::Counting;

/// Learner suite size: 16 tables x 2 arms of 400 hands (~10 s on the i7-4770K with eight threads).
const LEARNER_TABLES: usize = 16;
const LEARNER_HANDS: usize = 400;
/// Live suite: this many spots, one decision each.
const LIVE_SPOTS: usize = 240;

/// The frozen workload.
struct Fixture {
    source: String,
    params: Params,
    models: ModelStore,
    nn: Option<std::sync::Arc<Mlp>>,
    decision_samples: usize,
    live_samples: usize,
    live_deal_chunks: usize,
}

fn fixture(path: &str) -> Fixture {
    let parsed: Option<Value> = std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok());
    match parsed {
        Some(v) => Fixture {
            source: path.to_string(),
            params: serde_json::from_value(v["params"].clone()).expect("fixture params"),
            models: serde_json::from_value(v["models"].clone()).expect("fixture models"),
            nn: serde_json::from_value::<Option<Mlp>>(v["nn"].clone()).ok().flatten().map(std::sync::Arc::new),
            decision_samples: v["decision_samples"].as_u64().unwrap_or(2_500) as usize,
            live_samples: v["live_samples"].as_u64().unwrap_or(400_000) as usize,
            live_deal_chunks: v["live_deal_chunks"].as_u64().unwrap_or(8) as usize,
        },
        None => Fixture {
            source: "defaults (no fixture: run `learner bench-fixture`)".into(),
            params: Params::default(),
            models: ModelStore::default(),
            nn: None,
            decision_samples: 2_500,
            live_samples: 400_000,
            live_deal_chunks: 8,
        },
    }
}

/// Process CPU seconds (user + system, every thread).
fn cpu_secs() -> f64 {
    // SAFETY: getrusage writes the struct it is given.
    let ru = unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut ru);
        ru
    };
    let t = |tv: libc::timeval| tv.tv_sec as f64 + tv.tv_usec as f64 / 1e6;
    t(ru.ru_utime) + t(ru.ru_stime)
}

/// Peak resident set (MB) of the process so far.
fn peak_rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmHWM:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok()))
        .map_or(0.0, |kb| kb / 1024.0)
}

/// FNV-1a over the bytes that identify a result.
fn fnv(h: &mut u64, bytes: &[u8]) {
    for b in bytes {
        *h ^= u64::from(*b);
        *h = h.wrapping_mul(0x100_0000_01b3);
    }
}

/// Run `f`, measuring wall, CPU, allocations and the heap peak around it.
fn measured(f: impl FnOnce() -> Value) -> Value {
    alloc::reset_peak();
    let (a0, c0, t0) = (alloc::snapshot(), cpu_secs(), Instant::now());
    let mut v = f();
    let (wall, cpu, a1) = (t0.elapsed().as_secs_f64(), cpu_secs() - c0, alloc::snapshot());
    let o = v.as_object_mut().expect("suite result is an object");
    o.insert("wall_s".into(), json!(wall));
    o.insert("cpu_s".into(), json!(cpu));
    if alloc::enabled() {
        o.insert("allocs".into(), json!(a1.allocs - a0.allocs));
        o.insert("alloc_mb".into(), json!((a1.bytes - a0.bytes) as f64 / 1e6));
        o.insert("heap_peak_mb".into(), json!(alloc::peak_bytes() as f64 / 1e6));
    }
    v
}

fn eval_params(fx: &Fixture, p: &Params) -> Params {
    Params { samples: fx.decision_samples, deal_chunks: 1, ..p.clone() }
}

fn learner_suite<O: OpponentSpec>(fx: &Fixture, pool: &[(O, f64)]) -> Value {
    let champion = eval_params(fx, &fx.params);
    let challenger = Params { call_margin: champion.call_margin + 0.01, ..champion.clone() };
    let t = Instant::now();
    let c0 = cpu_secs();
    let r = paired_eval_arms(
        &Arm { params: &champion, nn: fx.nn.clone() },
        &[Arm { params: &challenger, nn: fx.nn.clone() }],
        pool,
        &fx.models,
        LEARNER_TABLES,
        LEARNER_HANDS,
        100,
        1,
    )
    .pop()
    .expect("one arm");
    let (wall, cpu) = (t.elapsed().as_secs_f64(), cpu_secs() - c0);
    let runs = 2 * LEARNER_TABLES;
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for x in [r.mean_bb.to_bits(), r.se_bb.to_bits(), r.hands, r.differing] {
        fnv(&mut h, &x.to_le_bytes());
    }
    json!({"suite": "learner", "table_runs": runs, "hands": runs * LEARNER_HANDS,
        "table_runs_per_s": runs as f64 / wall, "table_runs_per_cpu_s": runs as f64 / cpu,
        "hands_per_cpu_s": (runs * LEARNER_HANDS) as f64 / cpu,
        "checksum": format!("{h:016x}"), "mean_bb100": r.mean_bb * 100.0})
}

/// Records every situation the hero decides in, then decides like the policy agent it wraps.
struct Recorder {
    inner: PolicyAgent,
    spots: std::sync::Arc<std::sync::Mutex<Vec<Situation>>>,
}

impl Agent for Recorder {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn act(&mut self, sit: &Situation, rng: &mut SmallRng) -> Action {
        self.spots.lock().expect("spots").push(sit.clone());
        self.inner.act(sit, rng)
    }
    fn observe(&mut self, hand: &HandSummary) {
        self.inner.observe(hand)
    }
}

/// Fixed spots: the hero's decisions over a few seeded six-max tables against the pool.
fn spots<O: OpponentSpec>(fx: &Fixture, pool: &[(O, f64)]) -> Vec<Situation> {
    let spots = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut seed = 0u64;
    while spots.lock().expect("spots").len() < LIVE_SPOTS && seed < 64 {
        let hero = PolicyAgent {
            label: "svanbot10".into(),
            models: fx.models.clone(),
            params: eval_params(fx, &fx.params),
            learn: false,
            nn: fx.nn.clone(),
        };
        let mut agents: Vec<Box<dyn Agent>> = vec![Box::new(Recorder { inner: hero, spots: spots.clone() })];
        for i in 0..5 {
            agents.push(pool[(seed as usize + i) % pool.len()].0.agent(i));
        }
        sv10_core::sim::run_table(&mut agents, 60, 100, 5_000 + seed);
        seed += 1;
    }
    let mut v = spots.lock().expect("spots").clone();
    v.truncate(LIVE_SPOTS);
    v
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

fn live_suite(fx: &Fixture, spots: &[Situation]) -> Value {
    let params = Params { samples: fx.live_samples, deal_chunks: fx.live_deal_chunks, ..fx.params.clone() };
    let mut ms = Vec::with_capacity(spots.len());
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut streets = [0usize; 4];
    for (i, sit) in spots.iter().enumerate() {
        let mut rng = SmallRng::seed_from_u64(900 + i as u64);
        let t = Instant::now();
        let d = decide_with(sit, &fx.models, &params, fx.nn.as_deref(), &mut rng);
        ms.push(t.elapsed().as_secs_f64() * 1000.0);
        fnv(&mut h, format!("{:?}", d.action).as_bytes());
        fnv(&mut h, &d.equity.to_bits().to_le_bytes());
        streets[sit.board.len().saturating_sub(2).min(3)] += 1;
    }
    let total: f64 = ms.iter().sum();
    ms.sort_by(f64::total_cmp);
    json!({"suite": "live", "decisions": spots.len(), "samples_per_decision": fx.live_samples,
        "streets_pre_flop_turn_river": streets,
        "p50_ms": pct(&ms, 0.5), "p95_ms": pct(&ms, 0.95), "p99_ms": pct(&ms, 0.99), "p999_ms": pct(&ms, 0.999), "max_ms": ms.last(),
        "mean_ms": total / spots.len().max(1) as f64,
        "samples_per_s": (fx.live_samples * spots.len()) as f64 / (total / 1000.0), "checksum": format!("{h:016x}")})
}

/// A soft top-`pct` range over hand classes, like the narrowed ranges decisions build.
fn top_range(pct: f64) -> Range {
    let mut r = Range::full();
    for (i, w) in r.w.iter_mut().enumerate() {
        // A fixed pseudo-random half of the combos, weighted, so samplers and ratios have work.
        let h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40;
        *w = if (h % 1000) as f64 / 1000.0 < pct { 0.25 + (h % 7) as f32 / 8.0 } else { 0.0 };
    }
    r
}

fn micro_suite() -> Value {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    // Evaluator on 64k fixed 7-card masks (512 KB: L2-resident on the i7-4770K), 300 passes.
    let mut x = 1u64;
    let masks: Vec<u64> = (0..65_536)
        .map(|_| {
            let mut m = 0u64;
            while m.count_ones() < 7 {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                m |= 1 << (x % 52);
            }
            m
        })
        .collect();
    let t = Instant::now();
    let mut acc = 0u64;
    for pass in 0..300u64 {
        for &m in &masks {
            acc = acc.wrapping_add(u64::from(sv10_core::eval::eval(m)) ^ pass);
        }
    }
    let eval_ns = t.elapsed().as_nanos() as f64 / (300.0 * masks.len() as f64);
    fnv(&mut h, &acc.to_le_bytes());
    // Combo sampler: 20M draws from a half-weighted range.
    let range = top_range(0.5);
    let dead = Card::parse("7s").unwrap().bit() | Card::parse("8s").unwrap().bit() | Card::parse("2c").unwrap().bit();
    let sampler = sv10_core::equity::ComboSampler::new(&range, dead);
    let mut rng = SmallRng::seed_from_u64(3);
    let t = Instant::now();
    let mut acc = 0usize;
    for _ in 0..20_000_000 {
        acc = acc.wrapping_add(sampler.sample(&mut rng));
    }
    let sample_ns = t.elapsed().as_nanos() as f64 / 20e6;
    fnv(&mut h, &acc.to_le_bytes());
    // Shared deals, one thread: a flop against one and two ranges, then 20 candidate reweightings.
    let hole = [Card::parse("Ah").unwrap(), Card::parse("Kd").unwrap()];
    let board = [Card::parse("7s").unwrap(), Card::parse("8s").unwrap(), Card::parse("2c").unwrap()];
    let narrowed = top_range(0.3);
    let mut deal = serde_json::Map::new();
    for k in [1usize, 2] {
        let opps: Vec<&Range> = vec![&range; k];
        let n = 400_000;
        let mut rng = SmallRng::seed_from_u64(11);
        let t = Instant::now();
        let deals = sv10_core::equity::SharedDeals::new(hole, &board, &opps, n, &mut rng);
        let deal_ns = t.elapsed().as_nanos() as f64 / n as f64;
        let subset: Vec<(usize, Option<&Range>)> = (0..k).map(|j| (j, if j == 0 { Some(&narrowed) } else { None })).collect();
        let t = Instant::now();
        let mut eq = 0.0;
        for _ in 0..20 {
            eq += deals.equity(&subset).unwrap_or(0.0);
        }
        let reweight_ns = t.elapsed().as_nanos() as f64 / (20.0 * n as f64);
        fnv(&mut h, &eq.to_bits().to_le_bytes());
        deal.insert(format!("deal_ns_per_sample_{k}opp"), json!(deal_ns));
        deal.insert(format!("reweight_ns_per_sample_{k}opp"), json!(reweight_ns));
    }
    // Heads-up equity against a full range on a flop (the hardware tuning's own benchmark), one thread.
    let full = Range::full();
    let mut rng = SmallRng::seed_from_u64(42);
    let samples = 2_000_000;
    let t = Instant::now();
    let eq = sv10_core::equity::equity_vs_ranges(hole, &board, &[&full], samples, &mut rng);
    let equity_sps = samples as f64 / t.elapsed().as_secs_f64();
    fnv(&mut h, &eq.to_bits().to_le_bytes());
    // RNG.
    let n_rng = 200_000_000u64;
    let t = Instant::now();
    let mut r = SmallRng::seed_from_u64(7);
    let mut s = 0u64;
    for _ in 0..n_rng {
        s = s.wrapping_add(r.next_u64());
    }
    let rng_ns = t.elapsed().as_nanos() as f64 / n_rng as f64;
    fnv(&mut h, &s.to_le_bytes());
    let mut out = json!({"suite": "micro", "eval_ns_per_hand": eval_ns, "sample_ns_per_combo": sample_ns,
        "equity_hu_samples_per_s": equity_sps, "rng_ns_per_u64": rng_ns, "checksum": format!("{h:016x}")});
    out.as_object_mut().expect("object").extend(deal);
    out
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    let repeat: usize = flag("--repeat").and_then(|v| v.parse().ok()).unwrap_or(1);
    let fixture_path = flag("--fixture").unwrap_or_else(|| "artifacts/bench-fixture.json".into());
    let suite =
        args.iter().find(|a| ["learner", "live", "micro", "draw", "all"].contains(&a.as_str())).cloned().unwrap_or_else(|| "all".into());
    // The ablation builds its own inputs and needs no fixture, pool or threads; `--repeat` is its
    // round count (each round runs every arm once, in either direction) and defaults high, because
    // the answer rests on the interval rather than on one pass.
    if suite == "draw" {
        println!("{}", draw::run(if flag("--repeat").is_some() { repeat } else { draw::ROUNDS }));
        return;
    }
    if let Some(t) = flag("--threads").and_then(|v| v.parse().ok()) {
        rayon::ThreadPoolBuilder::new().num_threads(t).build_global().expect("thread pool");
    }
    let fx = fixture(&fixture_path);
    let clones = live_pool(&fx.models, 30.0, 16, 7_001);
    let archetypes: Vec<(Archetype, f64)> = sv10_core::bench::FROZEN_POOL.iter().map(|k| (archetype(k), 1.0)).collect();
    eprintln!(
        "bench: fixture {}, {} clones, net {}, {} threads, decision samples {}, live {} in {} chunks",
        fx.source,
        clones.len(),
        if fx.nn.is_some() { "on" } else { "off" },
        rayon::current_num_threads(),
        fx.decision_samples,
        fx.live_samples,
        fx.live_deal_chunks
    );
    let live_spots = if suite == "live" || suite == "all" {
        if clones.is_empty() { spots(&fx, &archetypes) } else { spots(&fx, &clones) }
    } else {
        Vec::new()
    };
    if args.iter().any(|a| a == "--allocs") {
        alloc::enable();
    }
    let profile = args.iter().any(|a| a == "--profile");
    if profile {
        profiler::start(1_000);
    }
    for rep in 0..repeat {
        let mut out: Vec<Value> = Vec::new();
        if suite == "learner" || suite == "all" {
            out.push(measured(|| if clones.is_empty() { learner_suite(&fx, &archetypes) } else { learner_suite(&fx, &clones) }));
        }
        if suite == "live" || suite == "all" {
            out.push(measured(|| live_suite(&fx, &live_spots)));
        }
        if suite == "micro" || suite == "all" {
            out.push(measured(micro_suite));
        }
        for mut v in out {
            if let Some(o) = v.as_object_mut() {
                o.insert("repeat".into(), json!(rep));
                o.insert("peak_rss_mb".into(), json!(peak_rss_mb()));
            }
            println!("{v}");
        }
    }
    if profile {
        profiler::stop();
        eprintln!("{}", profiler::report(40));
    }
}

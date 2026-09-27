//! Detect the host machine and derive runtime tuning, so a fresh install on any
//! computer picks sensible thread counts and Monte Carlo budgets by itself.

use serde::{Deserialize, Serialize};
use std::time::Instant;
use sv10_cards::cards::Card;
use sv10_cards::range::Range;
use sv10_equity::equity::equity_vs_ranges;
use sv10_rng::SeedableRng;
use sv10_rng::rngs::SmallRng;

/// What the host machine is and the tuning derived from it (logged at start, served by the API).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HardwareProfile {
    /// CPU model name from `/proc/cpuinfo`.
    pub cpu_model: String,
    /// Hardware threads available.
    pub logical_cores: usize,
    /// Physical cores (logical count when unknown).
    pub physical_cores: usize,
    /// Total memory.
    pub memory_gb: f64,
    /// Whether the CPU reports AVX2.
    pub avx2: bool,
    /// Measured single-thread equity throughput (heads-up Monte Carlo samples per second).
    pub samples_per_sec: f64,
    /// Runtime settings derived from the measurements.
    pub tuning: Tuning,
}

/// Live decisions use this many times the simulation Monte Carlo budget on a machine at least as fast
/// as the reference (`decision_samples` is at most 2,500, so at most 1.6M samples; 0161). History:
/// 16x (40k) until 0148, when the analyst's re-solves put the loss from live sampling noise at 0.012 bb
/// per decision (+3.5 bb/100); 160x (400k) ran at p50 46 ms / p95 97 ms with CPU load 3-4 of 8 and a
/// 45 s turn window, so 640x (about 190 ms p50) still uses under 1% of the window.
pub const LIVE_SAMPLE_FACTOR: usize = 640;
/// Floor of the live budget in simulation budgets, however slow the machine (the 0148 budget).
const MIN_LIVE_FACTOR: usize = 160;
/// Heads-up samples per second of one core on the reference machine (i7-4770K class).
const REFERENCE_SPS: f64 = 13_000_000.0;

/// Live Monte Carlo samples per decision: [`LIVE_SAMPLE_FACTOR`] simulation budgets, scaled down in
/// proportion on machines slower than the reference so decision time stays about the same, and never
/// below [`MIN_LIVE_FACTOR`] budgets.
pub fn live_budget(decision_samples: usize, samples_per_sec: f64) -> usize {
    let speed = (samples_per_sec / REFERENCE_SPS).clamp(0.0, 1.0);
    ((decision_samples * LIVE_SAMPLE_FACTOR) as f64 * speed).round().max((decision_samples * MIN_LIVE_FACTOR) as f64) as usize
}

/// Runtime settings sized to the machine.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tuning {
    /// Monte Carlo samples per equity estimate in the learner's simulations (the policy the learner tunes).
    pub decision_samples: usize,
    /// Monte Carlo samples per live decision: the idle cores buy a more precise estimate at the table.
    pub live_samples: usize,
    /// Parallel chunks the live decision's deals are split into (one per logical core).
    pub live_deal_chunks: usize,
    /// Threads the background learner uses (every logical core; it runs niced, so live play preempts it).
    pub learner_threads: usize,
    /// Parallel simulated tables per learner evaluation.
    pub learner_tables: usize,
    /// Hands per simulated table per evaluation.
    pub learner_hands: usize,
    /// Bots the machine can run comfortably (every bot is light; memory is the bound).
    pub max_bots: usize,
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

fn cpu_info() -> (String, usize, bool) {
    let info = read("/proc/cpuinfo");
    let model = info
        .lines()
        .find(|l| l.starts_with("model name"))
        .and_then(|l| l.split(':').nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| std::env::consts::ARCH.to_string());
    let mut cores = std::collections::HashSet::new();
    let mut phys = String::new();
    for l in info.lines() {
        if l.starts_with("physical id") {
            phys = l.split(':').nth(1).unwrap_or("").trim().to_string();
        } else if l.starts_with("core id") {
            let core = l.split(':').nth(1).unwrap_or("").trim().to_string();
            cores.insert((phys.clone(), core));
        }
    }
    let avx2 = info.lines().any(|l| l.starts_with("flags") && l.contains(" avx2"));
    (model, cores.len(), avx2)
}

fn memory_gb() -> f64 {
    read("/proc/meminfo")
        .lines()
        .find(|l| l.starts_with("MemTotal"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|kb| kb.parse::<f64>().ok())
        .map(|kb| kb / 1024.0 / 1024.0)
        .unwrap_or(4.0)
}

/// Heads-up equity samples per second on one thread: 40,000 samples after a 2,000-sample warm-up
/// (about 3 ms on the i7-4770K; `probe --bench` reports the whole detection, 2 ms on the cloud box).
fn benchmark() -> f64 {
    let full = Range::full();
    let hole = [Card::parse("Ah").unwrap(), Card::parse("Kd").unwrap()];
    let board = [Card::parse("7s").unwrap(), Card::parse("8s").unwrap(), Card::parse("2c").unwrap()];
    let mut rng = SmallRng::seed_from_u64(42);
    let _ = equity_vs_ranges(hole, &board, &[&full], 2_000, &mut rng);
    let n = 40_000;
    let t = Instant::now();
    let _ = equity_vs_ranges(hole, &board, &[&full], n, &mut rng);
    n as f64 / t.elapsed().as_secs_f64().max(1e-6)
}

/// Inspect the machine, run a short equity benchmark (~2 ms) and derive [`Tuning`].
pub fn detect() -> HardwareProfile {
    let logical = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let (cpu_model, physical, avx2) = cpu_info();
    let physical = if physical == 0 { logical } else { physical };
    let memory = memory_gb();
    let sps = benchmark();
    // Keep the main equity estimate under ~2ms of one core; never above the tuned default.
    let decision_samples = ((sps * 0.002) as usize).clamp(600, 2_500);
    let learner_threads = logical;
    // 640x the simulation budget on a reference-speed machine, dealt in parallel (0161).
    let live_samples = live_budget(decision_samples, sps);
    let live_deal_chunks = logical;
    // Reference: ~13M heads-up samples/s on an i7-4770K (the original target box) = 1.0.
    let speed = (sps / REFERENCE_SPS).clamp(0.2, 2.0);
    // Evaluation size (and so the promotion gate's power) stays what it was when the learner left two
    // cores free; more threads only finish it sooner.
    let learner_tables = (logical.saturating_sub(2).max(1) * 2).clamp(2, 32);
    // Keep evaluations large enough for meaningful confidence intervals even on a busy machine.
    let learner_hands = ((1_500.0 * speed) as usize).clamp(1_500, 3_000);
    let max_bots = ((memory / 0.25) as usize).clamp(1, 5);
    HardwareProfile {
        cpu_model,
        logical_cores: logical,
        physical_cores: physical,
        memory_gb: (memory * 10.0).round() / 10.0,
        avx2,
        samples_per_sec: sps.round(),
        tuning: Tuning { decision_samples, live_samples, live_deal_chunks, learner_threads, learner_tables, learner_hands, max_bots },
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_live_budget_is_four_times_bigger_on_the_reference_machine_and_scales_down_on_slow_ones() {
        // 0161: p95 was 97 ms at 400k samples with a 45 s turn window; 1.6M on the i7-4770K.
        assert_eq!(super::live_budget(2_500, 43_000_000.0), 1_600_000);
        assert_eq!(super::live_budget(2_500, 13_000_000.0), 1_600_000, "the reference speed gets the full budget");
        // Half the reference speed gets half, keeping decision time roughly constant.
        assert_eq!(super::live_budget(2_500, 6_500_000.0), 800_000);
        // Never below the previous 160x budget, however slow.
        assert_eq!(super::live_budget(600, 1_000.0), 600 * 160);
    }

    #[test]
    fn detects_something_sane() {
        let p = super::detect();
        assert!(p.logical_cores >= 1);
        assert!(p.samples_per_sec > 1000.0);
        assert!(p.tuning.decision_samples >= 600 && p.tuning.decision_samples <= 2500);
        assert!(p.tuning.learner_threads >= 1 && p.tuning.learner_threads <= p.logical_cores);
    }
}

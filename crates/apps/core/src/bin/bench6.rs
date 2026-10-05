//! `bench6 [hands] [stack_bb]` — the six-max benchmark (0279).
//!
//! The repeatable strength measurement the project was missing: one champion and one challenger over
//! identical deals against a **frozen** opponent pool, reported overall and broken down by the
//! position the hero held and by how each hand ended. Heads-up numbers (`sim paired`) cannot see a
//! seat, and a pool that moves with the season cannot settle an argument.
//!
//! ```text
//! bench6 4000                      # the champion against itself: 0.0 by construction
//! SIM_B='{"call_margin":0.005}' bench6 4000
//! SIM_MODELS=/tmp/m.json bench6 4000   # price the frozen pool with live opponent models
//! ```
//!
//! `SIM_A` and `SIM_B` are `Params` JSON (missing fields default), `SIM_STACK_BB` and `SIM_SEED`
//! override the arguments. Exit code 0 always: this is a report, not a gate — the gate is the
//! promotion rule, which has its own fresh-deal confirmation.

use sv10_core::bench::{Bench, FROZEN_POOL, six_max_paired};

fn bb100(v: f64) -> String {
    format!("{:+.1}", v)
}

/// A level with its own 95% interval, so a row is read as an estimate rather than a fact. The
/// interval is the point: at a few thousand hands a position's level is worth tens of bb/100 of
/// standard error, and two seats only differ if their intervals say so.
fn level(v: f64, ci: (f64, f64)) -> String {
    format!("{:+.1} ({:+.0}..{:+.0})", v, ci.0, ci.1)
}

fn report(bench: &Bench) {
    let p = &bench.paired;
    println!(
        "== six-max benchmark: {} hands per arm, {} blind stack, pool {}",
        bench.hands,
        std::env::var("SIM_STACK_BB").ok().and_then(|v| v.parse().ok()).unwrap_or(100),
        bench.pool.join(", ")
    );
    println!(
        "   champion {} bb/100   challenger {} bb/100   paired {:+.3} bb/hand (95% {:+.3}..{:+.3}, {} of {} hands differ)",
        bb100(bench.champion_bb100),
        bb100(bench.challenger_bb100),
        p.mean_bb,
        p.lower_95(),
        p.upper_95(),
        p.differing,
        p.hands
    );
    println!("   the level is this frozen pool with default parameters, not the live fleet: read the difference, not the level");
    println!("\n   by position (our bb/100 in each arm, with the interval on the level)");
    println!("   {:<14} {:>7} {:>22} {:>22} {:>14}", "position", "hands", "champion 95%", "challenger 95%", "paired 95%");
    for (name, row) in &bench.by_position {
        let paired =
            if row.hands == 0 { "—".to_string() } else { format!("{:+.3}..{:+.3}", row.paired.lower_95(), row.paired.upper_95()) };
        println!(
            "   {:<14} {:>7} {:>22} {:>22} {:>14}",
            name,
            row.hands,
            level(row.champion_bb100, row.champion_95),
            level(row.challenger_bb100, row.challenger_95),
            paired
        );
    }
    println!("\n   by outcome (bb/100 per hand *in that class*, so a rare big pot dominates its row)");
    println!("   {:<22} {:>7} {:>22} {:>12}", "outcome", "hands", "champion 95%", "challenger");
    for (name, row) in &bench.by_outcome {
        println!("   {:<22} {:>7} {:>22} {:>12}", name, row.hands, level(row.champion_bb100, row.champion_95), bb100(row.challenger_bb100));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let hands: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(4_000);
    let stack_bb: i64 = std::env::var("SIM_STACK_BB").ok().and_then(|v| v.parse().ok()).unwrap_or(100);
    let seed: u64 = std::env::var("SIM_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(20_260_926);
    // The frozen pool is built from the built-in styles; the policy may still *price* them with
    // live opponent models, which is what SIM_MODELS is for.
    let models = sv10_core::inputs::models("SIM_MODELS");
    eprintln!("frozen pool: {} ({} opponents)", FROZEN_POOL.join(", "), FROZEN_POOL.len());
    if models.players.is_empty() {
        eprintln!("no SIM_MODELS: the pool is priced from its own styles");
    }
    let t0 = std::time::Instant::now();
    let bench = six_max_paired(&sv10_core::inputs::params("SIM_A"), &sv10_core::inputs::params("SIM_B"), &models, hands, stack_bb, seed);
    report(&bench);
    eprintln!("== {:.1}s, seed {seed}", t0.elapsed().as_secs_f64());
}

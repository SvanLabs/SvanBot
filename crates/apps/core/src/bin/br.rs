//! `br [hands] [rounds]` — the champion's exploitability meter (#783).
//!
//! Five villains share one archetype whose six parameters a coordinate descent tunes to minimize the champion's
//! bb/100, every candidate on the same deals; the winner is re-measured on a fresh seed. Prints what a table of
//! exploiters takes from the champion beyond the frozen pool, and writes the same as JSON to `artifacts/br/` keyed by
//! the champion's version. A lower bound on exploitability inside that family, not a true best response.
//!
//! `SIM_A` is the champion's `Params` JSON (default: the defaults); `SIM_MODELS`, `SIM_SEED` (the documented seed set
//! is 20260926, 20260927, 20260928) and `SIM_STACK_BB` as for `bench6`. Reads nothing from the live fleet and never
//! plays a hand on the server.

use sv10_core::bench::exploit;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let hands: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(6_000);
    let rounds: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3);
    let stack_bb: i64 = std::env::var("SIM_STACK_BB").ok().and_then(|v| v.parse().ok()).unwrap_or(100);
    let seed: u64 = std::env::var("SIM_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(20_260_926);
    let champion = sv10_core::inputs::params("SIM_A");
    let models = sv10_core::inputs::models("SIM_MODELS");
    let t0 = std::time::Instant::now();
    let r = exploit(&champion, &models, hands, stack_bb, seed, rounds);
    let a = &r.exploiter;
    println!(
        "== best-response meter: {} hands per evaluation, {} evaluations, {stack_bb} blind stacks, seed {}",
        r.hands, r.evaluations, r.seed
    );
    println!("   champion vs the frozen pool            {:+.1} bb/100", r.pool_bb100);
    println!(
        "   exploiter found: vpip {:.2} pfr {:.2} aggression {:.2} bluff {:.2} call margin {:+.2} sizing {:.2}",
        a.vpip, a.pfr, a.aggression, a.bluff, a.call_margin, a.sizing
    );
    println!("   champion vs five of them (search seed) {:+.1} bb/100  (optimistic: chosen for being low)", r.search_bb100);
    println!(
        "   champion vs five of them (fresh seed {}, {} hands) {:+.1} bb/100 (95% {:+.1} .. {:+.1})",
        r.confirm_seed, r.confirm_hands, r.confirm_bb100, r.confirm_95.0, r.confirm_95.1
    );
    println!("   extraction beyond the pool: {:+.1} bb/100 (a lower bound inside this family of villains)", r.extraction_bb100);
    let version = std::env::var("BR_VERSION").unwrap_or_else(|_| "unversioned".into());
    let dir = std::path::Path::new(&std::env::var("SVANBOT10_ROOT").unwrap_or_else(|_| ".".into())).join("artifacts/br");
    if std::fs::create_dir_all(&dir).is_ok()
        && let Ok(json) = serde_json::to_string_pretty(&r)
    {
        let path = dir.join(format!("{version}-seed{seed}.json"));
        if std::fs::write(&path, json).is_ok() {
            println!("   wrote {}", path.display());
        }
    }
    eprintln!("== {:.0}s", t0.elapsed().as_secs_f64());
}

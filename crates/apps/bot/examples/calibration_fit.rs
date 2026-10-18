//! Offline self-calibration fit (2026-10-18 depth-split design): what the live task installs from a
//! store's rows, pooled as before the split and split by pot depth, printed per category and as the
//! two `Params` fragments `sim paired` reads (`SIM_A` pooled, `SIM_B` split). It fits and prints;
//! it stores nothing.
//!
//!   CARGO_TARGET_DIR=target/dev cargo run --profile release -p sv10-bot --example calibration_fit -- ROOT
use anyhow::{Context, Result};
use sv10_bot::tasks::calibration::fit_from_store;
use sv10_store::store::{DepthSplit, Store};

fn main() -> Result<()> {
    let root = std::path::PathBuf::from(std::env::args().nth(1).context("usage: calibration_fit ROOT")?);
    let store = Store::open(&root.join("artifacts").join("svanbot10.db"))?;
    for (arm, split, flag) in [("A", DepthSplit::NONE, false), ("B", sv10_bot::CALIBRATION_DEPTH_SPLIT, true)] {
        let (bias, caps, table) = fit_from_store(&store, &split)?;
        println!("== SIM_{arm}: {}", if flag { "split by pot depth" } else { "pooled" });
        for (key, row) in &table {
            let num = |k: &str| row[k].as_f64().unwrap_or(0.0);
            println!(
                "{key:20} n {:>7}  residual {:>9.2} ± {:>7.2}  correction {:>7.2} bb  {}",
                row["n"],
                num("residual_bb"),
                num("se_bb"),
                num("bias_bb"),
                row["bound_by"].as_str().unwrap_or("?")
            );
        }
        println!("SIM_{arm}={}", serde_json::json!({"ev_bias": bias, "ev_bias_pot_cap": caps, "calibration_depth_split": flag}));
    }
    Ok(())
}

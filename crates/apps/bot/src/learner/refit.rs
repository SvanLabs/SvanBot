//! The evidence refresh in steps (0334): the daily range-model fit, the live fits, then the
//! per-opponent fits with the population snapshot. Each is well under two minutes; the three
//! together took 35–138 s in one piece.

use serde_json::json;
use std::time::Instant;

use super::run::{RefitRun, STEP_TARGET_SECS};
use super::{Ctx, POPULATION_MODELS_KEY, load_models, now, status};

/// The steps in order.
const STEPS: [&str; 3] = ["range model", "live fits", "per-opponent fits"];

/// Whether the daily range-model fit is due.
fn range_model_due(ctx: &Ctx) -> bool {
    let fitted_at = ctx
        .store
        .get_kv(crate::RANGE_PARAMS_KEY)
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str::<crate::StoredRangeParams>(&j).ok())
        .map(|s| s.fitted_at)
        .unwrap_or(0.0);
    now() - fitted_at >= 24.0 * 3600.0 && ctx.root.join("target").join("release").join("calibrate").exists()
}

/// Re-fit the range model to showdowns (the `calibrate` binary decides, from held-out
/// likelihood, whether the new fit is used). About 100 s.
fn refit_range_model(ctx: &Ctx) {
    let exe = ctx.root.join("target").join("release").join("calibrate");
    match std::process::Command::new(&exe).arg("20000").current_dir(ctx.root).status() {
        Ok(st) if st.success() => tracing::info!("range model re-fit finished"),
        Ok(st) => tracing::warn!("range model re-fit exited with {st}"),
        Err(e) => tracing::warn!("range model re-fit could not start: {e}"),
    }
}

/// Planned seconds of step `i`.
fn planned(ctx: &Ctx, i: u8) -> f64 {
    match i {
        0 if range_model_due(ctx) => 110.0,
        0 => 0.0,
        1 => 20.0,
        _ => 40.0,
    }
}

/// Advance the refresh by one step (several when they fit). `true` once it is complete.
pub fn step(ctx: &Ctx, run: &mut RefitRun) -> anyhow::Result<bool> {
    let t0 = Instant::now();
    let mut did = false;
    while (run.done as usize) < STEPS.len() {
        let i = run.done;
        if did && t0.elapsed().as_secs_f64() + planned(ctx, i) > STEP_TARGET_SECS {
            break;
        }
        status(
            ctx.store,
            // `job` names what is running: a refresh now runs on nearly every hand (#314), and the
            // panel must not report it as challenger validation.
            json!({"status": "training", "job": "refit", "phase": format!("refreshing evidence-derived models: {}", STEPS[i as usize]), "automatic": true,
            "lineage": super::load_lineage(ctx.store), "progress": {"hands": i, "target": STEPS.len()}}),
        );
        let t = Instant::now();
        match i {
            0 => {
                if range_model_due(ctx) {
                    status(
                        ctx.store,
                        json!({"status": "training", "job": "refit", "phase": "fitting the range model to showdowns", "automatic": true}),
                    );
                    refit_range_model(ctx);
                }
            }
            1 => crate::livefits::refit_all(ctx.store, now()),
            _ => {
                crate::playerfits::refit(ctx.store, &ctx.root.join("artifacts"));
                let models = load_models(ctx.store);
                ctx.store.put_kv(POPULATION_MODELS_KEY, &serde_json::to_string(&models)?)?;
            }
        }
        if t.elapsed().as_secs_f64() >= 1.0 {
            did = true;
            tracing::info!("evidence refresh: {} in {:.0}s", STEPS[i as usize], t.elapsed().as_secs_f64());
        }
        run.done += 1;
    }
    super::log_step("learner evidence refresh step", t0.elapsed());
    Ok(run.done as usize >= STEPS.len())
}

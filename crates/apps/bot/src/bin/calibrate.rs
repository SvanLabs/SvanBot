//! `calibrate [max_samples]` — fit the range-reconstruction constants to every stored showdown
//! (live hands plus imported past-season hands) and store the result as `range_params.v1`.
//! The fleet and learner install the fitted set only when it beats the defaults on the newest
//! quarter of showdowns, which the fit never sees.

use anyhow::Result;
use sv10_bot::history::HistoryDb;
use sv10_bot::{MODELS_KEY, RANGE_PARAMS_KEY, StoredRangeParams};
use sv10_core::calibrate::{fit_frozen, samples_from_hand};
use sv10_core::model::{HandSummary, ModelStore};
use sv10_core::oprange::RangeParams;
use sv10_store::store::Store;

fn main() -> Result<()> {
    sv10_bot::init_tool_logging();
    let root = std::env::var("SVANBOT10_ROOT").map(std::path::PathBuf::from).unwrap_or(std::env::current_dir()?);
    let artifacts = root.join("artifacts");
    let max_samples: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(20_000);
    let threads = std::env::var("CALIBRATE_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
    rayon::ThreadPoolBuilder::new().num_threads(threads).build_global()?;
    let store = Store::open(&artifacts.join("svanbot10.db"))?;
    let models: ModelStore = store.get_kv(MODELS_KEY)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let fleet =
        sv10_bot::config::Config::from_env(&root).map(|c| c.bots.into_iter().map(|b| b.name).collect::<Vec<_>>()).unwrap_or_default();

    let mut hands: Vec<(String, HandSummary)> = Vec::new();
    if let Ok(db) = HistoryDb::open(&artifacts.join("history.db")) {
        hands.extend(
            db.recent_summaries(400_000, std::env::var_os("CALIBRATE_CORPUS").is_some())?
                .into_iter()
                .filter(|(_, h)| h.board.len() == 5 && !h.shown.is_empty()),
        );
    }
    for bot in &fleet {
        for row in store.recent_fit_hands(bot, 1_000_000)? {
            if let Ok(h) = serde_json::from_str::<HandSummary>(&row.summary)
                && h.board.len() == 5
                && !h.shown.is_empty()
            {
                hands.push((row.ended_at, h));
            }
        }
    }
    hands.sort_by(|a, b| a.0.cmp(&b.0));
    tracing::info!("{} showdown hands available", hands.len());
    // Newest hands first until the sample budget is met, then back to time order.
    let mut samples = Vec::new();
    for (_, h) in hands.iter().rev() {
        samples.extend(samples_from_hand(h, &models, &fleet));
        if samples.len() >= max_samples {
            break;
        }
    }
    samples.reverse();
    if samples.len() < 1000 {
        tracing::warn!("only {} showdown samples; need at least 1000 to fit", samples.len());
        return Ok(());
    }
    // Diagnostic (0122): is the live range model calibrated on each postflop line type?
    if std::env::var("CALIBRATE_LINES").is_ok() {
        let rp = sv10_bot::fitted_range_params(store.get_kv(RANGE_PARAMS_KEY)?.as_deref()).unwrap_or(RangeParams::DEFAULT);
        println!("line            samples  P(model range stronger than shown)  95%");
        for (kind, n, mean, se) in sv10_core::calibrate::line_calibration(&samples, &rp) {
            println!("{kind:<15} {n:>7}  {mean:.3}  [{:.3}, {:.3}]", mean - 1.96 * se, mean + 1.96 * se);
        }
        return Ok(());
    }
    let split = samples.len() * 3 / 4;
    let val = samples.split_off(split);
    tracing::info!("fitting on {} showdowns, validating on the newest {}", samples.len(), val.len());
    let started = std::time::Instant::now();
    // Experiments: CALIBRATE_START=live starts from the stored fitted set instead of the defaults;
    // CALIBRATE_FREEZE=a,b holds those fields at their start values. Both imply a dry run.
    let start = match std::env::var("CALIBRATE_START").as_deref() {
        Ok("live") => sv10_bot::fitted_range_params(store.get_kv(RANGE_PARAMS_KEY)?.as_deref()).unwrap_or(RangeParams::DEFAULT),
        _ => RangeParams::DEFAULT,
    };
    let frozen_env = std::env::var("CALIBRATE_FREEZE").unwrap_or_default();
    let frozen: Vec<&str> = frozen_env.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    let experiment = std::env::var("CALIBRATE_START").is_ok() || !frozen.is_empty();
    let report = fit_frozen(&samples, &val, start, 8, &frozen);
    let active = report.val_ll > report.default_val_ll + 0.005;
    tracing::info!(
        "range fit in {:.0}s ({} evaluations): train {:.4} -> {:.4}, validation {:.4} -> {:.4} (uniform {:.4}) -> {}",
        started.elapsed().as_secs_f64(),
        report.evaluations,
        report.default_train_ll,
        report.train_ll,
        report.default_val_ll,
        report.val_ll,
        report.uniform_ll,
        if active { "ACTIVE" } else { "not better than defaults; keeping defaults" }
    );
    // Held-out likelihood by the shown player's largest postflop bet (0235): a shape term aimed at
    // overbets must help there without hurting the rest.
    for (lo, hi, label) in
        [(0.0, 1.5, "bets under 1.5x pot or none"), (1.5, 4.0, "largest bet 1.5-4x pot"), (4.0, f64::INFINITY, "largest bet 4x+ pot")]
    {
        let set: Vec<&sv10_core::calibrate::ShowdownSample> = val.iter().filter(|s| (lo..hi).contains(&s.max_bet_to_pot())).collect();
        if set.is_empty() {
            continue;
        }
        let mean = |rp: &RangeParams| set.iter().map(|s| s.log_likelihood(rp)).sum::<f64>() / set.len() as f64;
        tracing::info!("  held-out {label}: n {} log-likelihood {:.4} -> {:.4}", set.len(), mean(&start), mean(&report.params));
    }
    let defaults = RangeParams::DEFAULT;
    let mut d = defaults;
    let mut p = report.params;
    for ((name, a, _, _), (_, b, _, _)) in d.fields_mut().into_iter().zip(p.fields_mut()) {
        if (*a - *b).abs() > 1e-6 {
            tracing::info!("  {name}: {:.3} -> {:.3}", *a, *b);
        }
    }
    let stored = StoredRangeParams {
        active,
        params: report.params,
        report: serde_json::to_value(&report)?,
        fitted_at: chrono::Utc::now().timestamp() as f64,
    };
    if std::env::var("CALIBRATE_DRY").is_ok() || experiment {
        tracing::info!("dry run: not stored");
        println!("{}", serde_json::to_string(&stored.params)?);
    } else {
        store.put_kv(RANGE_PARAMS_KEY, &serde_json::to_string(&stored)?)?;
    }
    Ok(())
}

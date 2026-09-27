//! Decision analyst: a separate process that re-solves every live decision with a deep search.
//!
//! The fleet queues each decision's full inputs (`audit_queue`, a `ReplayRecord`). The analyst takes
//! them oldest first, re-runs the recorded strategy with `ANALYST_SAMPLES` Monte Carlo samples (default
//! 10x the live budget: 16M on the i7-4770K since 0161, about two seconds per decision) dealt across every core, and stores whether the live choice
//! matched and how many big blinds the deep search says it gave up (`decision_audit`). The dashboard
//! shows the totals. It never changes live play: it measures whether a bigger live budget would.
//!
//! When the audit queue is empty, idle cores run the drift instrument (0128, 0366) from
//! [`sv10_bot::review_drift`]: if the champion in `params.v1` changed since the last check, recent
//! big-spot replays are re-run under the current champion at the recorded decision's own prices
//! (action-flip rate) plus one deep audit each (gap vs deep search), summarized in `analyst.drift`
//! (`review drift` prints that row). That is the post-promotion drift signal 0120 consumes. The
//! re-solve varies the champion's knobs and nothing else — the record's per-opponent corrections, its
//! prices, and a sample pinned to the current replay version — and the row states the basis, the
//! budget, the population and the version mix it was measured on. That is the same price basis and
//! budget `decision_audit` uses, differing only in the policy it grades: the champion's knobs there,
//! the record's here. The same idle branch re-measures the wiring table daily (0316): what each live
//! component is worth on recent decisions, which the dashboard shows so "this component is active" is
//! a number and not a claim.
//!
//! It runs niced below the fleet and above the learner (`scripts/start.sh`) and exits for a new release.

use anyhow::Result;
use serde_json::json;
use std::time::{Duration, Instant};
use sv10_bot::ANALYST_STATUS_KEY as STATUS_KEY;
use sv10_bot::replay::{ReplayRecord, audit};
use sv10_bot::review_drift::{DRIFT_STEP, DriftRun, NetCache, cached_net, drift_check};
use sv10_store::store::Store;

/// Drift is re-checked at most this often, and only while the audit queue is empty.
const DRIFT_INTERVAL_SECS: f64 = 600.0;
/// The wiring table (0316) is re-measured at most this often, and only while the audit queue is
/// empty: it is 200 records re-run a dozen times each, so fresh audits always win.
const WIRING_INTERVAL_SECS: f64 = 24.0 * 3600.0;
/// Recorded big decisions per wiring measurement: 120 re-run about 60 s at 8 threads, inside the
/// two-minute budget for live-system work (200 took 103 s; 0334). `review wiring N` takes any size.
const WIRING_SAMPLE: usize = 120;
/// In-process retry gap for the wiring refresh. A sample under [`MIN_SAMPLE`] stores nothing, so
/// without this the idle branch would re-measure an empty fleet's table every two seconds.
///
/// [`MIN_SAMPLE`]: sv10_bot::review_wiring::MIN_SAMPLE
const WIRING_RETRY_SECS: f64 = 1800.0;

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64()
}

fn main() -> Result<()> {
    sv10_bot::release::handle_version_flag("analyst");
    // Settings in .env (LEARNER_*, ANALYST_*) apply like they do for the fleet; variables already set
    // win, and this runs before any thread reads the environment.
    let env_root = std::env::var("SVANBOT10_ROOT").map(std::path::PathBuf::from).unwrap_or(std::env::current_dir()?);
    let _ = sv10_rt::load_env_file(&env_root.join(".env"));
    sv10_bot::init_tool_logging();
    let release = sv10_bot::release::ExeWatch::current();
    let root = std::env::var("SVANBOT10_ROOT").map(std::path::PathBuf::from).unwrap_or(std::env::current_dir()?);
    let store = Store::open(&root.join("artifacts").join("svanbot10.db"))?;
    let hw = sv10_core::hardware::detect();
    let env = |k: &str| std::env::var(k).ok().and_then(|v| v.trim().parse::<usize>().ok()).filter(|v| *v > 0);
    // The dashboard's compute profile (0187) wins over ANALYST_THREADS and the whole machine.
    let profile_threads = |store: &Store| {
        sv10_bot::profile::ComputeProfile::stored(store.get_kv(sv10_bot::profile::PROFILE_KEY).ok().flatten().as_deref(), hw.logical_cores)
            .map(|p| p.analyst_threads)
    };
    let threads = profile_threads(&store).unwrap_or_else(|| env("ANALYST_THREADS").unwrap_or(hw.logical_cores));
    // A deep re-solve only measures the live budget if it is much deeper: keep it 10x live (0161).
    let samples = env("ANALYST_SAMPLES").unwrap_or(hw.tuning.live_samples * 10);
    // Only expensive spots get a deep audit (`ANALYST_MIN_POT_BB`, 0 = every decision); the rest are
    // marked skipped, freeing the cores for the learner (2026-09-23).
    let min_pot_bb = std::env::var("ANALYST_MIN_POT_BB")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| *v >= 0.0)
        .unwrap_or(sv10_bot::replay::AUDIT_MIN_POT_BB);
    rayon::ThreadPoolBuilder::new().num_threads(threads).build_global()?;
    tracing::info!("analyst on {} ({threads} threads, {samples} samples per deep re-solve, spots from {min_pot_bb} bb)", hw.cpu_model);
    let mut nets: NetCache = Default::default();
    let (mut done, mut skipped, mut busy_secs) = (0u64, 0u64, 0.0f64);
    let started = now();
    let mut last_status = 0.0;
    let mut last_drift = 0.0;
    let mut drift: Option<DriftRun> = None;
    let mut last_wiring = 0.0;
    loop {
        if release.as_ref().is_some_and(|w| w.replacement_ready(Duration::from_secs(20), "analyst")) {
            tracing::info!("new analyst release installed; exiting for it");
            return Ok(());
        }
        if profile_threads(&store).is_some_and(|t| t != threads) {
            tracing::info!("compute profile changed the analyst's threads ({threads} -> {:?}); restarting", profile_threads(&store));
            return Ok(());
        }
        let jobs = match store.audit_batch(16) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("reading the audit queue failed: {e}");
                std::thread::sleep(Duration::from_secs(10));
                continue;
            }
        };
        if now() - last_status >= 15.0 || jobs.is_empty() {
            let summary = store.audit_summary(&(chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339()).unwrap_or_default();
            // `min_pot_bb` is the filter the analyst is actually applying: the findings scan prints it on
            // every decision-loss row, and a row that named the constant while this process ran on
            // `ANALYST_MIN_POT_BB` would name a filter nobody applied (0355).
            let status = json!({"running": true, "threads": threads, "samples": samples, "audited": done, "skipped": skipped,
                "queued": summary.queued, "busy_share": busy_secs / (now() - started).max(1.0), "updated": now(),
                "min_pot_bb": min_pot_bb, "commit": sv10_bot::BUILD_COMMIT});
            let _ = store.put_kv(STATUS_KEY, &status.to_string());
            last_status = now();
        }
        if jobs.is_empty() {
            // Idle cores work the drift check (a new one at most every 10 minutes, one slice of at most
            // DRIFT_STEP per idle pass, 0334); fresh audits always win.
            if drift.is_some() || now() - last_drift >= DRIFT_INTERVAL_SECS {
                last_drift = now();
                drift_check(&store, &mut nets, threads, samples, &mut drift, DRIFT_STEP);
            }
            // ...and the wiring table (0316), which the dashboard reads: daily, and only when idle.
            // `due` reads the stored row, so a restart does not re-measure a fresh table.
            if now() - last_wiring >= WIRING_RETRY_SECS && sv10_bot::review_wiring::due(&store, now(), WIRING_INTERVAL_SECS) {
                last_wiring = now();
                match sv10_bot::review_wiring::refresh(&store, WIRING_SAMPLE) {
                    Ok(r) if r.sample >= sv10_bot::review_wiring::MIN_SAMPLE => tracing::info!(
                        "wiring table refreshed on {} decisions: {} replay exactly, {} of {} replay-v3 records exact",
                        r.sample,
                        r.exact,
                        r.v3_exact,
                        r.v3
                    ),
                    Ok(r) => tracing::info!(
                        "wiring table measured on {} decisions, under the {} a stored table needs; not stored",
                        r.sample,
                        sv10_bot::review_wiring::MIN_SAMPLE
                    ),
                    Err(e) => tracing::warn!("wiring refresh failed: {e}"),
                }
            }
            std::thread::sleep(Duration::from_secs(2));
            continue;
        }
        for job in jobs {
            let t = Instant::now();
            let result = match serde_json::from_str::<ReplayRecord>(&job.record) {
                Ok(rec) if !sv10_bot::replay::worth_auditing(&rec, min_pot_bb) => {
                    skipped += 1;
                    None
                }
                Ok(rec) => {
                    let nn = job.net_digest.as_ref().and_then(|digest| cached_net(&store, &mut nets, digest));
                    // The audit grades the record's own knobs and prices at the analyst's depth: the
                    // recorded decision re-solved on a bigger budget, nothing else varied.
                    let deep = sv10_core::policy::Params { samples, deal_chunks: threads, ..rec.params.clone() };
                    Some(audit(&rec, &deep, nn.as_deref(), &job.bot, &job.hand_id))
                }
                Err(e) => {
                    tracing::warn!("audit record {} unreadable, skipped: {e}", job.id);
                    skipped += 1;
                    None
                }
            };
            if let Err(e) = store.finish_audit(job.id, result.as_ref()) {
                tracing::warn!("storing audit {} failed: {e}", job.id);
                std::thread::sleep(Duration::from_secs(5));
                break;
            }
            if let Some(r) = &result {
                done += 1;
                if r.gap_bb >= 5.0 {
                    tracing::info!(
                        "{} hand {} {}: live {} vs deep {} gives up {:.1} bb (pot {:.0} bb)",
                        r.bot,
                        r.hand_id,
                        r.street,
                        r.live_action,
                        r.deep_action,
                        r.gap_bb,
                        r.pot_bb
                    );
                }
            }
            busy_secs += t.elapsed().as_secs_f64();
        }
        if nets.len() > 16 {
            nets.clear();
        }
    }
}

//! Autonomous strategy learner: fits simulated clones of the live opponent
//! pool, searches policy parameters with paired evaluation, and promotes a
//! challenger into the live fleet only on a positive 95% lower bound. Every job runs as short
//! stored steps (`sv10_bot::learner`, 0334).

use anyhow::Result;
use serde_json::json;
use std::time::{Duration, Instant};
use sv10_bot::learner::run::{self, RESUME_SLICE_SECS, RefitRun, Run, SLICE_TARGET_SECS};
use sv10_bot::learner::search::Outcome;
use sv10_bot::learner::{self, Ctx, MIN_OPPONENT_HANDS, load_params, now, pool::challengers, status};
use sv10_bot::pacing::{Gate, LearnerJob, LearnerSettings, PACING_KEY, Pacing, PacingState, SETTINGS_KEY};
use sv10_bot::{NN_KEY, StoredNet};
use sv10_core::agents::live_pool;
use sv10_core::model::ModelStore;
use sv10_core::policy::Params;
use sv10_core::sim::paired_eval_many;
use sv10_store::store::Store;

/// Time of the latest operator start request (dashboard "Start next search early").
fn operator_run_at(store: &Store) -> Option<f64> {
    sv10_bot::pacing::operator_start_at(&store.get_kv(sv10_bot::LEARNER_COMMAND_KEY).ok().flatten()?)
}

/// `learner pacing-study [tables] [base rows...]` (0244): rebuild the opponent models at each base
/// row and 500, 2,000, 5,000 and 10,000 hands later, and measure how far the search population and
/// the candidate ranking move, next to the seed noise two cycles on the same models see. Deals and
/// clone draws are shared across one base's snapshots, so their differences are the population's.
fn pacing_study(store: &Store, root: &std::path::Path, args: &[String]) -> Result<()> {
    use sv10_bot::pacing_study::{Replay, population_shift, rank_shift};
    const STEPS: [i64; 5] = [0, 500, 2_000, 5_000, 10_000];
    const PAGE: usize = 5_000;
    let tables: usize = args.first().and_then(|a| a.parse().ok()).unwrap_or(8);
    let max_row = store.max_hand_rowid()?;
    let season_row = store
        .get_kv(sv10_bot::SEASON_KEY)?
        .and_then(|j| serde_json::from_str::<sv10_bot::season::CurrentSeason>(&j).ok())
        .and_then(|s| store.first_hand_since(&chrono::DateTime::from_timestamp(s.started_at as i64, 0)?.to_rfc3339()).ok().flatten());
    let mut bases: Vec<i64> = args.iter().skip(1).filter_map(|a| a.parse().ok()).collect();
    if bases.is_empty() {
        bases = [season_row, Some(30_000), Some(max_row - 10_500)].into_iter().flatten().collect();
    }
    bases.retain(|b| b + STEPS[STEPS.len() - 1] <= max_row && *b > 0);
    bases.sort();
    let mut cuts: Vec<i64> = bases.iter().flat_map(|b| STEPS.iter().map(move |s| b + s)).collect();
    cuts.sort();
    cuts.dedup();
    let hw = sv10_core::hardware::detect();
    let hands = hw.tuning.learner_hands;
    println!("pacing study: bases {bases:?}, {tables} tables x {hands} hands per candidate, rows up to {max_row}");
    // Past-season hands warm the models as the live store's history import did; only hands from
    // before the first stored live hand, so no snapshot sees its future.
    let ours = store.bot_names()?;
    let first_live = store.hands_page(0, i64::MAX, 1)?.first().map(|(r, _, _)| *r).unwrap_or(1);
    let first_time = store.hand_time(first_live)?.unwrap_or_default();
    let mut models = ModelStore { half_life_hands: sv10_bot::OPPONENT_HALF_LIFE_HANDS, ..Default::default() };
    let mut warm = 0usize;
    for (t, h) in sv10_bot::history::HistoryDb::open(&root.join("artifacts").join("history.db"))
        .and_then(|db| db.recent_summaries(60_000, false))
        .unwrap_or_default()
    {
        if t.get(..19).unwrap_or("") < first_time.get(..19).unwrap_or("") {
            let hero = h.players.iter().find_map(|(_, n)| ours.contains(n).then_some(n.as_str()));
            models.observe(&h, hero);
            warm += 1;
        }
    }
    println!("warmed with {warm} past-season hands before {first_time}");
    let mut replay = Replay::new(models, ours);
    let mut snapshots: std::collections::BTreeMap<i64, ModelStore> = Default::default();
    let mut at = 0i64;
    for cut in &cuts {
        loop {
            let page = store.hands_page(at, *cut, PAGE)?;
            if page.is_empty() {
                break;
            }
            for (row, id, json) in page {
                at = row;
                if let Ok(h) = serde_json::from_str::<sv10_core::model::HandSummary>(&json)
                    && !h.stacks.is_empty()
                {
                    replay.observe(&id, &h);
                }
            }
        }
        snapshots.insert(*cut, replay.models.clone());
    }
    println!("replayed to row {at}; {} snapshots", snapshots.len());
    let champion = load_params(store);
    let mut eval_champion = champion.clone();
    eval_champion.samples = hw.tuning.decision_samples;
    eval_champion.deal_chunks = 1;
    let candidates = challengers(&champion, 0);
    let batch: Vec<Params> = candidates
        .iter()
        .map(|(_, _, _, p)| {
            let mut e = p.clone();
            e.samples = hw.tuning.decision_samples;
            e.deal_chunks = 1;
            e
        })
        .collect();
    let nn = sv10_bot::neural::active_response_net(store.get_kv(NN_KEY)?.and_then(|j| serde_json::from_str::<StoredNet>(&j).ok()));
    let evaluate = |m: &ModelStore, seed: u64| -> Vec<f64> {
        let clones = live_pool(m, MIN_OPPONENT_HANDS, 16, 7_000 + seed);
        paired_eval_many(&eval_champion, &batch, &clones, m, nn.clone(), tables, hands, 100, 900_000 + seed * 10_000)
            .into_iter()
            .map(|r| r.mean_bb)
            .collect()
    };
    let mut report = Vec::new();
    for base in &bases {
        let t = Instant::now();
        let reference = evaluate(&snapshots[base], 1);
        let noise = evaluate(&snapshots[base], 2);
        let n = rank_shift(&reference, &noise);
        println!(
            "base {base}: seed noise (same models, next cycle's seeds): spearman {:.3}, top5 shared {}, same leader {}, mean |change| {:.2} bb/100",
            n.spearman, n.top5_shared, n.same_leader, n.mean_abs_change_bb100
        );
        let mut rows = vec![json!({"step": 0, "noise": n})];
        for step in &STEPS[1..] {
            let m = &snapshots[&(base + step)];
            let pop = population_shift(&snapshots[base], m, MIN_OPPONENT_HANDS, 16);
            let r = rank_shift(&reference, &evaluate(m, 1));
            println!(
                "base {base} +{step}: pool {} of {} new, rates moved {:.2} pp; ranking spearman {:.3}, top5 shared {}, same leader {}, mean |change| {:.2} bb/100",
                pop.members_changed, pop.members, pop.rate_shift_pp, r.spearman, r.top5_shared, r.same_leader, r.mean_abs_change_bb100
            );
            rows.push(json!({"step": step, "population": pop, "ranking": r}));
        }
        println!("base {base} took {:.0}s", t.elapsed().as_secs_f64());
        report.push(json!({"base": base, "time": store.hand_time(*base)?, "rows": rows,
            "candidates": candidates.iter().map(|(k, o, n, _)| format!("{k} {o:.3}->{n:.3}")).collect::<Vec<_>>()}));
    }
    let out = root.join("artifacts").join("pacing-study.json");
    std::fs::write(&out, serde_json::to_string_pretty(&json!({"tables": tables, "hands": hands, "bases": report}))?)?;
    println!("wrote {}", out.display());
    Ok(())
}

/// `learner bench-fixture`: freeze what a search plays (champion, population, live net) and this
/// machine's sample budgets into `artifacts/bench-fixture.json` for the `bench` bin (0335).
fn bench_fixture(store: &Store, root: &std::path::Path) -> Result<()> {
    let (params, models, nn) = learner::search::inputs(store);
    let hw = sv10_core::hardware::detect();
    let out = root.join("artifacts").join("bench-fixture.json");
    let fixture = json!({"created": now(), "params": params, "models": models, "nn": nn.as_deref(),
        "decision_samples": hw.tuning.decision_samples, "live_samples": hw.tuning.live_samples,
        "live_deal_chunks": hw.tuning.live_deal_chunks, "learner_hands": hw.tuning.learner_hands});
    std::fs::write(&out, serde_json::to_string(&fixture)?)?;
    println!("wrote {} ({} opponents, net {})", out.display(), models.players.len(), if nn.is_some() { "on" } else { "off" });
    Ok(())
}

fn main() -> Result<()> {
    sv10_bot::release::handle_version_flag("learner");
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("pacing-study") {
        let _ = sv10_rt::load_env_file(&std::env::current_dir()?.join(".env"));
        sv10_bot::init_tool_logging();
        let root = std::env::var("SVANBOT10_ROOT").map(std::path::PathBuf::from).unwrap_or(std::env::current_dir()?);
        let store = Store::open(&root.join("artifacts").join("svanbot10.db"))?;
        return pacing_study(&store, &root, &args[1..]);
    }
    if args.first().map(String::as_str) == Some("bench-fixture") {
        let root = std::env::var("SVANBOT10_ROOT").map(std::path::PathBuf::from).unwrap_or(std::env::current_dir()?);
        let store = Store::open(&root.join("artifacts").join("svanbot10.db"))?;
        return bench_fixture(&store, &root);
    }
    // Settings in .env (LEARNER_*, ANALYST_*) apply like they do for the fleet; variables already set
    // win, and this runs before any thread reads the environment.
    let env_root = std::env::var("SVANBOT10_ROOT").map(std::path::PathBuf::from).unwrap_or(std::env::current_dir()?);
    let _ = sv10_rt::load_env_file(&env_root.join(".env"));
    sv10_bot::init_tool_logging();
    let release = sv10_bot::release::ExeWatch::current();
    let root = std::env::var("SVANBOT10_ROOT").map(std::path::PathBuf::from).unwrap_or(std::env::current_dir()?);
    let store = Store::open(&root.join("artifacts").join("svanbot10.db"))?;
    // Size the learner to this machine (override with LEARNER_THREADS), leaving cores for live play.
    let hw = sv10_core::hardware::detect();
    // The dashboard's compute profile (0187) wins over LEARNER_THREADS and the hardware default.
    let profile_threads = |store: &Store| {
        sv10_bot::profile::ComputeProfile::stored(store.get_kv(sv10_bot::profile::PROFILE_KEY).ok().flatten().as_deref(), hw.logical_cores)
            .map(|p| p.learner_threads)
    };
    let threads = profile_threads(&store)
        .unwrap_or_else(|| std::env::var("LEARNER_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(hw.tuning.learner_threads));
    rayon::ThreadPoolBuilder::new().num_threads(threads).build_global()?;
    let tables = hw.tuning.learner_tables;
    let hands = hw.tuning.learner_hands;
    tracing::info!("learner on {} ({} threads, {} tables x {} hands per evaluation)", hw.cpu_model, threads, tables, hands);
    let cycle: u64 = store.get_kv(sv10_bot::LEARNER_CYCLE_KEY).ok().flatten().and_then(|s| s.parse().ok()).unwrap_or(0);
    let pacing = Pacing::from_env();
    tracing::info!(
        ".env pacing: {} new hands per cycle, backoff up to x{}, at most {:.0} h idle, {:.0} min cooldown (dashboard settings override)",
        pacing.min_new_hands,
        pacing.max_backoff,
        pacing.max_idle_secs / 3600.0,
        pacing.cooldown_secs / 60.0
    );
    let mut pace: PacingState = match store.get_kv(PACING_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()) {
        Some(p) => p,
        // An existing learner starts paced from now instead of searching again at once.
        None if cycle > 0 => {
            let now = now();
            let row = store.max_hand_rowid().unwrap_or(0);
            PacingState {
                last_rowid: row,
                last_run: now,
                last_end: now,
                streak: 1,
                follow_up: false,
                refit_rowid: row,
                refit_run: now,
                refit_end: now,
            }
        }
        None => PacingState::default(),
    };
    pace.migrate();
    // Persist at once, so a restart or hot swap keeps counting instead of restarting the wait.
    store.put_kv(PACING_KEY, &serde_json::to_string(&pace)?)?;
    let mut wait_logged = 0.0;
    let mut logged_pacing: Option<(i64, f64)> = None;
    // Live fits are cheap; refit at start, and again hourly on quiet tables where the evidence
    // refresh (which waits for new hands) never comes due.
    sv10_bot::livefits::refit_stale(&store, now());
    sv10_bot::playerfits::refit(&store, &root.join("artifacts"));
    let mut last_calls_refit = now();
    // The first slice this process plays is planned conservatively (0343): the rate it would
    // otherwise plan from was measured by whichever process ran before it, under load that may not
    // hold any more — a run stored on a quiet night and resumed onto a busy day, or resumed at a
    // release. Playing one slice re-measures the rate, and the cap widens for the rest of the run.
    let mut step_cap = RESUME_SLICE_SECS;
    let ctx = Ctx { store: &store, root: &root, threads, tables, hands, decision_samples: hw.tuning.decision_samples };
    loop {
        // A new release takes over between steps (the supervisor restarts the learner, which
        // resumes the stored run).
        if release.as_ref().is_some_and(|w| w.replacement_ready(Duration::from_secs(20), "learner")) {
            tracing::info!("new learner release installed; exiting for it");
            return Ok(());
        }
        // Between steps: a new profile thread count restarts the learner (its supervisor brings it
        // back with the new pool; the run and the pacing state are stored).
        if profile_threads(&store).is_some_and(|t| t != threads) {
            tracing::info!("compute profile changed the learner's threads ({threads} -> {:?}); restarting", profile_threads(&store));
            return Ok(());
        }
        // The dashboard's settings (cooldown, new-hands limit) apply from the next poll.
        let pacing = pacing.with_settings(&LearnerSettings::parse(store.get_kv(SETTINGS_KEY).ok().flatten().as_deref()));
        // A run in progress goes first: one step of at most about two minutes, then these checks again.
        if let Some(run) = run::load(&store) {
            let stepped = advance(&ctx, run, &mut pace, &pacing, &mut last_calls_refit, step_cap);
            step_cap = SLICE_TARGET_SECS;
            if let Err(e) = stepped {
                tracing::warn!("learner step failed ({e:#}); retrying in 30 s");
                std::thread::sleep(Duration::from_secs(30));
            }
            continue;
        }
        let lineage = learner::load_lineage(&store);
        let start_rowid = store.max_hand_rowid().unwrap_or(pace.last_rowid);
        let started = now();
        // The effective pacing (dashboard settings over .env), at start and whenever it changes (0246).
        let effective = (pacing.min_new_hands, pacing.cooldown_secs);
        if logged_pacing != Some(effective) {
            tracing::info!(
                "effective pacing: {} new hands per cycle, {:.0} min cooldown (dashboard settings over .env)",
                pacing.min_new_hands,
                pacing.cooldown_secs / 60.0
            );
            logged_pacing = Some(effective);
        }
        let season_started_at = store
            .get_kv(sv10_bot::SEASON_KEY)
            .ok()
            .flatten()
            .and_then(|j| serde_json::from_str::<sv10_bot::season::CurrentSeason>(&j).ok())
            .map(|season| season.started_at);
        let job = match pacing.gate(&pace, start_rowid, started, operator_run_at(&store), season_started_at) {
            Gate::Wait { job, have, needed, cooldown_until, reason } => {
                // Cheap live fits stay fresh even when no new hands arrive.
                if sv10_bot::raisewar::calls_refit_due(last_calls_refit, started) {
                    sv10_bot::livefits::refit_stale(&store, now());
                    last_calls_refit = now();
                }
                let baseline = match job {
                    LearnerJob::Refit => pace.refit_run,
                    LearnerJob::Search => pace.last_run,
                };
                let elapsed = (started - baseline).max(1.0);
                let eta = if needed == 0 {
                    cooldown_until.unwrap_or(started + 60.0)
                } else if have > 0 {
                    baseline + elapsed * needed as f64 / have as f64
                } else {
                    baseline + pacing.max_idle_secs
                };
                let next_run = cooldown_until.map_or(eta, |until| eta.min(until)).max(started + 60.0);
                let season_day = season_started_at.map(|s| ((started - s).max(0.0) / 86_400.0).floor() as u64 + 1);
                status(
                    &store,
                    json!({"status": "idle", "phase": if cooldown_until.is_some() { "cooling down" } else { "waiting for new hands" }, "automatic": true, "lineage": lineage, "next_run": next_run,
                    "cooldown_until": cooldown_until, "cooldown_minutes": pacing.cooldown_secs / 60.0, "follow_up": pace.follow_up,
                    "progress": {"hands": have, "target": needed},
                    "next_job": {"kind": job, "label": job.label(), "hands": have, "target": needed, "remaining": (needed - have).max(0), "reason": reason, "season_day": season_day},
                    "champion": {"version": lineage.last(), "name": "svanbot10 exploitative EV policy"}}),
                );
                if started - wait_logged >= 1800.0 {
                    tracing::info!("waiting for {}: {reason}", job.label());
                    wait_logged = started;
                }
                std::thread::sleep(Duration::from_secs(10));
                continue;
            }
            Gate::Run { job, reason } => {
                tracing::info!(
                    "starting {}: {reason} ({} new hands)",
                    job.label(),
                    match job {
                        LearnerJob::Refit => (start_rowid - pace.refit_rowid).max(0),
                        LearnerJob::Search => (start_rowid - pace.last_rowid).max(0),
                    }
                );
                wait_logged = 0.0;
                job
            }
        };
        let begun = match job {
            LearnerJob::Refit => Some(Run::Refit(RefitRun { start_rowid, started, done: 0 })),
            LearnerJob::Search => learner::search::begin(&ctx, start_rowid, started, pace.refit_rowid)?.map(|s| Run::Search(Box::new(s))),
        };
        match begun {
            Some(r) => run::save(&store, &r)?,
            None => std::thread::sleep(Duration::from_secs(300)),
        }
    }
}

/// Take one step of the stored run and, when it completes, record it in the pacing state.
fn advance(ctx: &Ctx, run: Run, pace: &mut PacingState, pacing: &Pacing, last_calls_refit: &mut f64, step_cap: f64) -> Result<()> {
    let store = ctx.store;
    match run {
        Run::Refit(mut r) => {
            if !learner::refit::step(ctx, &mut r)? {
                return run::save(store, &Run::Refit(r));
            }
            let ended = now();
            pace.finish_refit(r.start_rowid, r.started, ended);
            *last_calls_refit = ended;
            store.put_kv(PACING_KEY, &serde_json::to_string(&pace)?)?;
            run::clear(store);
            tracing::info!("evidence refresh took {:.0}s; next after {} new hands", ended - r.started, pacing.min_new_hands);
        }
        Run::Search(mut s) => match learner::search::step(ctx, &mut s, step_cap)? {
            Outcome::Continue => run::save(store, &Run::Search(s))?,
            Outcome::Finished { promoted } => {
                let ended = now();
                pace.finish_search(s.start_rowid, s.started, ended, promoted);
                let _ = store.put_kv(PACING_KEY, &serde_json::to_string(&pace)?);
                run::clear(store);
                tracing::info!(
                    "cycle {} took {:.0}s in {} steps; next after {} new hands or {:.0} min cooldown{}",
                    s.cycle,
                    ended - s.started,
                    s.steps,
                    pacing.required(pace.streak),
                    pacing.cooldown_secs / 60.0,
                    if pace.follow_up { " (then a follow-up around the new champion)" } else { "" }
                );
            }
            Outcome::Abandoned(why) => {
                tracing::warn!("cycle {}: search abandoned after {} steps: {why}; the next search starts over", s.cycle, s.steps);
                run::clear(store);
            }
        },
    }
    Ok(())
}

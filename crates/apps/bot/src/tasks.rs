//! Background work of the fleet process: bot sessions under a panic supervisor, and the
//! periodic loops that keep models, parameters, analytics and backups current.

use crate::client;
use crate::live::Shared;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Duration;

// One module per concern (0256); the names below are re-exported so every caller keeps its
// path (`sv10_bot::tasks::retry_unstored_hands` and friends are unchanged).
pub mod backup;
pub mod calibration;
pub mod hands;
pub mod models;
pub mod scan;

pub use backup::{backup_database, free_bytes, save_models};
pub use calibration::{update_calibration, update_calibration_with};
pub use hands::{MAX_HAND_RETRIES, MAX_UNSTORED_HANDS, bound_unstored, retry_unstored_hands};
pub use models::{fold_new_hands, recover_models, refresh_models_from_store};
pub use scan::run_findings_scan;

/// Start every bot session and background loop.
pub fn spawn_all(shared: &Arc<Shared>) {
    if !shared.config.head {
        spawn_bot_loops(shared);
        spawn_hand_retry(shared);
    }
    if shared.config.worker {
        // A worker plays its bots and writes nothing the head owns (models, calibration table,
        // checkpoints), but it plays with everything live: promotions and live fits through the
        // params watcher, the season clock, self-calibration applied locally, and the head's
        // opponent models reloaded whenever the head checkpoints them (0167).
        spawn_worker_loops(shared);
        spawn_season_poller(shared);
        spawn_params_watcher(shared);
        spawn_calibration_loop(shared);
        spawn_worker_model_refresh(shared);
        spawn_experiment_refresh(shared);
        return;
    }

    spawn_season_poller(shared);

    spawn_experiment_poller(shared);
    spawn_experiment_refresh(shared);

    spawn_params_watcher(shared);

    spawn_update_checker(shared);

    spawn_ev_fill(shared);

    spawn_h2h_loop(shared);

    spawn_guide_loop(shared);

    spawn_model_saver(shared);

    spawn_calibration_loop(shared);

    spawn_reputation_loop(shared);

    spawn_backup_loop(shared);

    // Packs the cold JSON written before 0229 and returns history.db's freed pages to the disk.
    crate::compaction::spawn(shared);

    spawn_watchdog(shared);

    spawn_findings_scan(shared);

    // Past-season hands from the server's history export feed the opponent models.
    tokio::spawn(crate::history::run(shared.clone()));

    // The head owns the canonical models; workers only write hand rows.
    if shared.config.head {
        spawn_head_tailer(shared);
    }
}

/// Poll `/season/current` and `/season/me`: wind-down flag and per-bot season state.
fn spawn_season_poller(shared: &Arc<Shared>) {
    let season = shared.clone();
    tokio::spawn(async move {
        let http = reqwest::Client::builder().user_agent(crate::USER_AGENT).timeout(Duration::from_secs(20)).build().unwrap();
        loop {
            if let Ok(r) = http.get(format!("{}/season/current", season.config.rest_base)).send().await
                && let Ok(v) = r.json::<serde_json::Value>().await
            {
                if let Some(clock) = crate::season::SeasonClock::from_current(&v, std::time::Instant::now()) {
                    let (was, now) = (season.season_clock.read().winding_down, clock.winding_down);
                    if now && !was {
                        season.log("fleet", "info", "season winding down: no new hands; table moves frozen until the next season");
                    }
                    *season.season_clock.write() = clock;
                }
                // The season's identity, for the panels that present this season's performance.
                // Persisted so a restart knows the boundary at once.
                if let Some(current) = crate::season::CurrentSeason::from_current(&v) {
                    let changed = season.current_season.read().as_ref() != Some(&current);
                    if changed {
                        match serde_json::to_string(&current) {
                            Ok(json) => {
                                if let Err(e) = season.store.put_kv(crate::SEASON_KEY, &json) {
                                    tracing::warn!("season identity not stored (a restart relearns it from the server): {e}");
                                }
                            }
                            Err(e) => tracing::warn!("season identity not serializable: {e}"),
                        }
                        if let Some(number) = current.number {
                            season.log("fleet", "info", format!("season {number} is the current season; season panels scope to it"));
                        }
                        *season.current_season.write() = Some(current);
                    }
                }
            }
            for (slot, bot) in season.config.bots.clone().into_iter().enumerate() {
                if let Some(m) = client::rest_get(&http, &season, &bot, "/season/me").await {
                    season.update(slot, |b| b.season = Some(m));
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            tokio::time::sleep(Duration::from_secs(120)).await;
        }
    });
}

/// Experiment mode (0291): read the official leaderboard every five minutes, advance the mode,
/// store it for every process and check the running target's live gates. A restart starts in
/// champion mode (the stored state is overwritten before the first reading).
fn spawn_experiment_poller(shared: &Arc<Shared>) {
    let poll = shared.clone();
    tokio::spawn(async move {
        let http = reqwest::Client::builder().user_agent(crate::USER_AGENT).timeout(Duration::from_secs(20)).build().unwrap();
        let mut mode = crate::experiment::ModeState::start(now_secs());
        crate::experiment::store_mode(&poll, &mode);
        loop {
            crate::experiment::poll_once(&poll, &http, &mut mode).await;
            tokio::time::sleep(Duration::from_secs(crate::experiment::mode::POLL_SECS)).await;
        }
    });
}

/// Every process re-reads the experiment state (mode, targets, verdicts, the pair's hand counts)
/// every 15 s; a bot's policy changes only at its next hand.
fn spawn_experiment_refresh(shared: &Arc<Shared>) {
    let refresh = shared.clone();
    tokio::spawn(async move {
        loop {
            let s = refresh.clone();
            crate::jobs::blocking("experiment refresh", move || crate::experiment::refresh(&s)).await;
            tokio::time::sleep(Duration::from_secs(15)).await;
        }
    });
}

/// Fill the all-in EV net of stored hands (0213): new hands within 30 s, the backlog in batches.
fn spawn_ev_fill(shared: &Arc<Shared>) {
    const BATCH: usize = 2_000;
    let fill = shared.clone();
    tokio::spawn(async move {
        let mut total = 0usize;
        loop {
            let s = fill.clone();
            // `jobs::blocking` is the only door to the blocking pool: a panicking job is logged by
            // name and the loop carries on (0254).
            let read = crate::jobs::blocking("all-in EV fill", move || crate::luck::fill_ev_nets(&s.store, BATCH)).await;
            let pause = match read {
                // A panicking job logged itself and carried on; there is nothing to add here.
                Some(Ok(n)) if n == BATCH => {
                    total += n;
                    Duration::from_secs(1)
                }
                Some(Ok(n)) => {
                    if total > 0 {
                        tracing::info!("all-in EV nets filled for {} stored hands", total + n);
                        total = 0;
                    }
                    Duration::from_secs(30)
                }
                Some(Err(e)) => {
                    tracing::warn!("all-in EV fill failed: {e}");
                    Duration::from_secs(60)
                }
                None => Duration::from_secs(60),
            };
            tokio::time::sleep(pause).await;
        }
    });
}

/// The fleet reading its own play (0273): every 30 minutes the instruments are run, what they find
/// is stored and shown, and anything new and serious is filed as a ticket — with the finding
/// closing its own ticket when it stops reproducing.
///
/// The head only: the workers hold no root to write the board from, and two processes filing the
/// same finding would produce two tickets for one problem.
fn spawn_findings_scan(shared: &Arc<Shared>) {
    if shared.config.worker {
        return;
    }
    let scan = shared.clone();
    tokio::spawn(async move {
        let root = scan.config.root.clone();
        tokio::time::sleep(Duration::from_secs(300)).await;
        loop {
            let s = scan.clone();
            let root = root.clone();
            crate::jobs::blocking("findings scan", move || run_findings_scan(&s, &root)).await;
            tokio::time::sleep(Duration::from_secs(1_800)).await;
        }
    });
}
/// Pick up what the learner stored for live play (promotions, models, fits) every 30 s; a store
/// read error keeps what is installed ([`crate::installs`]).
fn spawn_params_watcher(shared: &Arc<Shared>) {
    let watcher = shared.clone();
    tokio::spawn(async move {
        let installs = Arc::new(Mutex::new(crate::installs::Installs::after_startup(&watcher.store)));
        loop {
            let (w, i) = (watcher.clone(), installs.clone());
            crate::jobs::blocking("install watcher", move || i.lock().refresh(&w)).await;
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}

/// Head-to-head results per opponent, for table selection.
fn spawn_h2h_loop(shared: &Arc<Shared>) {
    let h2h = shared.clone();
    tokio::spawn(async move {
        let ledger = Arc::new(Mutex::new(crate::headtohead::Ledger::default()));
        loop {
            let (s, ledger) = (h2h.clone(), ledger.clone());
            crate::jobs::blocking("head-to-head", move || {
                let fleet: Vec<String> = s.config.bots.iter().map(|b| b.name.clone()).collect();
                let mut l = ledger.lock();
                l.update(&s.store, &fleet);
                *s.head_to_head.write() = l.table.clone();
            })
            .await;
            tokio::time::sleep(Duration::from_secs(300)).await;
        }
    });
}

/// Keep the dashboard's starting-hand guide in step with the live models and parameters.
fn spawn_guide_loop(shared: &Arc<Shared>) {
    let guide = shared.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(90)).await;
        loop {
            let g = guide.clone();
            crate::jobs::blocking("starting-hand guide", move || {
                let models = g.models.read().clone();
                let params = g.params.read().clone();
                let nn = g.nn.read().clone();
                crate::guide::refresh(&models, &params, nn.as_deref());
            })
            .await;
            tokio::time::sleep(Duration::from_secs(900)).await;
        }
    });
}

/// Retry hands whose insert failed every 30 s (0152).
fn spawn_hand_retry(shared: &Arc<Shared>) {
    let retry = shared.clone();
    tokio::spawn(async move {
        let mut t = tokio::time::interval(Duration::from_secs(30));
        loop {
            t.tick().await;
            if retry.unstored_hands.lock().is_empty() {
                continue;
            }
            let s = retry.clone();
            crate::jobs::blocking("hand retry", move || retry_unstored_hands(&s)).await;
        }
    });
}
/// Checkpoint the models every 5 minutes (and on shutdown).
fn spawn_model_saver(shared: &Arc<Shared>) {
    let saver = shared.clone();
    tokio::spawn(async move {
        // Every 5 minutes (and on shutdown): hands after the checkpoint are replayed from the
        // hand table on startup, so a longer interval loses nothing and writes 10x less.
        let mut t = tokio::time::interval(Duration::from_secs(300));
        t.tick().await;
        loop {
            t.tick().await;
            let s = saver.clone();
            crate::jobs::blocking("model save", move || save_models(&s)).await;
        }
    });
}

/// Self-calibration: learn per-spot EV corrections from predicted vs realized outcomes.
fn spawn_calibration_loop(shared: &Arc<Shared>) {
    let calib = shared.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            let c = calib.clone();
            let persist = !c.config.worker;
            crate::jobs::blocking("self-calibration", move || update_calibration_with(&c, persist)).await;
            tokio::time::sleep(Duration::from_secs(300)).await;
        }
    });
}

/// Reputation from every season's leaderboard, refreshed hourly.
fn spawn_reputation_loop(shared: &Arc<Shared>) {
    // Ended seasons are cached, so a refresh is two requests.
    let rep = shared.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        loop {
            if let Some(book) = crate::reputation::refresh(&rep).await {
                let n = book.by_name.len();
                *rep.reputation.write() = book;
                tracing::info!("reputation book refreshed ({n} name entries across all seasons)");
            }
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}

/// Autonomy watchdog (0222): every 10 minutes, check that the learner, analyst, fold calibration
/// and backups are still reporting, and log once when one stops and once when it recovers.
fn spawn_watchdog(shared: &Arc<Shared>) {
    let watch = shared.clone();
    tokio::spawn(async move {
        // Give every loop a full round after a restart before judging it.
        tokio::time::sleep(Duration::from_secs(15 * 60)).await;
        let loops = crate::watchdog::loops(crate::pacing::Pacing::from_env().max_idle_secs);
        let mut stale: Vec<crate::watchdog::Stale> = Vec::new();
        loop {
            let w = watch.clone();
            let before = std::mem::take(&mut stale);
            let checked = crate::jobs::blocking("autonomy watchdog", move || {
                let now = crate::watchdog::stale_loops(now_secs(), &loops, |k| w.store.get_kv(k));
                for (level, line) in crate::watchdog::transitions(&before, &now) {
                    w.log("fleet", level, line);
                }
                (before, now)
            })
            .await;
            stale = match checked {
                Some((_, now)) => now,
                None => Vec::new(),
            };
            tokio::time::sleep(Duration::from_secs(600)).await;
        }
    });
}

/// Update check (0236): fetch the update branch every 30 minutes so the dashboard shows what an
/// update would bring without a click. A failed fetch is logged once per change of outcome and
/// simply tried again at the next tick (no retry loop, LESSONS 30).
fn spawn_update_checker(shared: &Arc<Shared>) {
    let up = shared.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(90)).await;
        let mut last_error: Option<String> = None;
        loop {
            let u = up.clone();
            let prev = last_error.clone();
            last_error = crate::jobs::blocking("update check", move || {
                let check = crate::api::run_update_check(&u.config.root, &u.config.artifacts);
                if check.error != prev {
                    match &check.error {
                        Some(e) => u.log("fleet", "warn", format!("update check: {e}")),
                        None if prev.is_some() => u.log("fleet", "info", "update check works again"),
                        None => {}
                    }
                }
                check.error
            })
            .await
            .flatten();
            tokio::time::sleep(Duration::from_secs(1800)).await;
        }
    });
}

/// Hourly consistent database backups.
fn spawn_backup_loop(shared: &Arc<Shared>) {
    let backups = shared.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(120)).await;
        loop {
            let b = backups.clone();
            crate::jobs::blocking("database backup", move || backup_database(&b)).await;
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}

/// One supervised session loop per bot; a panic restarts that bot, never the fleet.
fn spawn_bot_loops(shared: &Arc<Shared>) {
    let config = shared.config.clone();
    for (slot, bot) in config.bots.iter().cloned().enumerate() {
        let s = shared.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(slot as u64 * 3)).await;
            // A panic inside a bot's session must never silently retire that bot.
            loop {
                let handle = tokio::spawn(client::run_bot(s.clone(), slot, bot.clone()));
                match handle.await {
                    Err(e) if e.is_panic() => {
                        s.log(&bot.name, "error", format!("bot task panicked; restarting in 5s: {e}"));
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                    _ => break,
                }
            }
        });
    }
}

/// Worker loops of a split fleet (0128): heartbeats for the head dashboard and the operator's
/// desired-mode key. Head processes never run these; the all-in-one fleet never needs them.
/// Reload the head's opponent-model checkpoint every 30 s when it changed (the head saves every
/// 5 minutes after folding every process's hands in, so this is the canonical view).
fn spawn_worker_model_refresh(shared: &Arc<Shared>) {
    let refresh = shared.clone();
    tokio::spawn(async move {
        let mut last = None;
        loop {
            let s = refresh.clone();
            let prev = last.take();
            last = crate::jobs::blocking("worker model refresh", move || {
                let mut last = prev;
                refresh_models_from_store(&s, &mut last);
                last
            })
            .await
            .flatten();
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}

fn spawn_worker_loops(shared: &Arc<Shared>) {
    let beats = shared.clone();
    tokio::spawn(async move {
        loop {
            for (slot, bot) in beats.config.bots.iter().enumerate() {
                let value = {
                    let b = beats.bots[slot].read();
                    crate::live::wrap_heartbeat(&b, now_secs())
                };
                if let Err(e) = beats.store.put_kv(&crate::live::heartbeat_key(&bot.name), &value.to_string()) {
                    tracing::warn!("heartbeat for {} not stored: {e}", bot.name);
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    let wants = shared.clone();
    tokio::spawn(async move {
        let mut last: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        loop {
            for (slot, bot) in wants.config.bots.iter().enumerate() {
                let key = crate::live::want_key(&bot.name);
                let want = wants.store.get_kv(&key).ok().flatten().unwrap_or_default();
                if !want.is_empty() && last.get(&bot.name).map(|l| l.as_str()) != Some(want.as_str()) {
                    let cmd = want.clone();
                    wants.update(slot, |b| crate::live::apply_desired(b, &cmd));
                    wants.log(&bot.name, "info", format!("operator command from the head: {cmd}"));
                    last.insert(bot.name.clone(), want);
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}
/// Head loop of a split fleet (0128): fold every newly stored hand into the canonical models.
/// Workers write hand rows but never save; the regular 5-minute saver persists what the tailer
/// observed, so no observation is lost whichever process saw the table.
fn spawn_head_tailer(shared: &Arc<Shared>) {
    let tail = shared.clone();
    tokio::spawn(async move {
        loop {
            let t = tail.clone();
            crate::jobs::blocking("head tailer", move || match fold_new_hands(&t.store, &t.models) {
                Ok(folded) if folded > 0 => tracing::info!("head tailer folded {folded} stored hands into the models"),
                Err(e) => tracing::warn!("head model tail failed: {e}"),
                _ => {}
            })
            .await;
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });
}

/// Unix seconds, for heartbeat and dashboard timestamps.
fn now_secs() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64()
}

pub const INTEGRITY_STATUS_KEY: &str = "integrity.status";

#[cfg(test)]
mod tests;

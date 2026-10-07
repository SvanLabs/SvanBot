use anyhow::Result;
use parking_lot::{Mutex, RwLock};
use std::collections::VecDeque;
use std::sync::Arc;
use sv10_bot::live::{BotLive, Shared};
use sv10_bot::{api, config};
use sv10_core::model::ModelStore;
use sv10_core::policy::Params;
use sv10_store::store;

use sv10_bot::{MODELS_KEY, PARAMS_KEY};

#[tokio::main]
async fn main() -> Result<()> {
    sv10_bot::release::handle_version_flag("sv10-bot");
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_timer(sv10_bot::LocalTime)
        .with_target(false)
        .init();
    let root = std::env::var("SVANBOT10_ROOT").map(std::path::PathBuf::from).unwrap_or(std::env::current_dir()?);
    let config = config::Config::from_env(&root)?;
    for db in ["svanbot10.db", "history.db"] {
        let health = sv10_store::integrity::ensure_healthy(&config.artifacts.join(db), &config.artifacts.join("backups"))?;
        if health != sv10_store::integrity::Health::Healthy && health != sv10_store::integrity::Health::Missing {
            tracing::error!("{db}: {health:?}");
        }
    }
    let store = store::Store::open(&config.artifacts.join("svanbot10.db"))?;
    let mut models: ModelStore = store.get_kv(MODELS_KEY)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    // Recency-weighted opponent tallies (0168), set before replaying hands after the checkpoint.
    models.half_life_hands = sv10_bot::OPPONENT_HALF_LIFE_HANDS;
    sv10_bot::tasks::recover_models(&store, &mut models)?;
    let hardware = tokio::task::spawn_blocking(sv10_core::hardware::detect).await?;
    tracing::info!(
        "hardware: {} | {} logical / {} physical cores | {:.1} GB | avx2={} | {:.0} samples/s -> live decisions {} samples in {} parallel chunks, learner {} threads",
        hardware.cpu_model,
        hardware.logical_cores,
        hardware.physical_cores,
        hardware.memory_gb,
        hardware.avx2,
        hardware.samples_per_sec,
        hardware.tuning.live_samples,
        hardware.tuning.live_deal_chunks,
        hardware.tuning.learner_threads
    );
    store.put_kv(sv10_bot::HARDWARE_PROFILE_KEY, &serde_json::to_string(&hardware)?)?;
    let mut params: Params = store.get_kv(PARAMS_KEY)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    params.samples = hardware.tuning.live_samples;
    params.deal_chunks = hardware.tuning.live_deal_chunks;
    if let Some(range) = sv10_bot::fitted_range_params(store.get_kv(sv10_bot::RANGE_PARAMS_KEY)?.as_deref()) {
        params.range = range;
        tracing::info!("range model: showdown-fitted parameters in use");
    }
    let fits = sv10_bot::livefits::LiveFits::load(&store)?;
    for line in fits.changes_from(&sv10_bot::livefits::LiveFits::NONE) {
        tracing::info!("{line}");
    }
    fits.apply(&mut params);
    let player_fits = sv10_bot::playerfits::PlayerFits::load(&store)?;
    for line in player_fits.changes_from(&sv10_bot::playerfits::PlayerFits::none()) {
        tracing::info!("{line}");
    }
    player_fits.apply(&mut models);
    tracing::info!("svanbot10 starting: {} bots, {} known opponents, dry_run={}", config.bots.len(), models.players.len(), config.dry_run);
    // Warm the preflop tables before the first decision needs them.
    tokio::task::spawn_blocking(|| {
        sv10_core::preflop::table();
        sv10_core::preflop::top_range_equity();
    })
    .await?;
    let store_reputation = store.get_kv(sv10_bot::reputation::KEY)?;
    // The last season the poller saw, so the dashboard scopes correctly from the first request
    // instead of merging seasons until the first `/season/current` reply lands.
    let stored_season = store.get_kv(sv10_bot::SEASON_KEY)?.and_then(|s| serde_json::from_str::<sv10_bot::season::CurrentSeason>(&s).ok());
    let store_clock = store.get_kv(sv10_bot::season::CLOCK_KEY)?;
    let now_unix = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
    let (events, _) = tokio::sync::broadcast::channel(1024);
    let bots: Vec<RwLock<BotLive>> = config
        .bots
        .iter()
        .enumerate()
        .map(|(slot, b)| {
            RwLock::new(BotLive {
                slot,
                name: b.name.clone(),
                mode: "offline".into(),
                desired: if config.dry_run { "stop".into() } else { "run".into() },
                ..Default::default()
            })
        })
        .collect();
    // Lifetime state-hash tallies, so the Runtime health panel keeps its rate across a hot swap (0301).
    sv10_bot::live::load_state_hash_totals(&store, &bots);
    let champion_version = sv10_bot::live::lineage_head(&store).unwrap_or_else(|| api::POLICY_VERSION.to_string());
    // A renamed bot keeps one season record: every name its key played under (identity, 2026-09-23).
    let aliases = sv10_bot::identity::resolve(&store, &config.bots, &std::env::var("SVANBOT_ALIASES").unwrap_or_default());
    for names in aliases.values().filter(|n| n.len() > 1) {
        tracing::info!("bot {} also played as {}", names[0], names[1..].join(", "));
    }
    // Hands the previous process left in progress, finished from the resync replay (0315). A head
    // process plays no bot of its own, so the hands it must not take are its workers' (0128).
    let plays: Vec<String> = if config.head { Vec::new() } else { config.bots.iter().map(|b| b.name.clone()).collect() };
    let resumable = sv10_bot::live::take_open_hands(&store, &plays);
    let shared = Arc::new(Shared {
        config: config.clone(),
        bots,
        models: RwLock::new(models),
        params: RwLock::new(params),
        nn: RwLock::new(None),
        reputation: RwLock::new(store_reputation.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()),
        head_to_head: RwLock::new(Default::default()),
        avatars: RwLock::new(Default::default()),
        store,
        log: Mutex::new(VecDeque::new()),
        events,
        started_at: chrono::Utc::now().to_rfc3339(),
        // The last reading, aged by the wall clock: a restart inside the end window keeps the freeze (#744).
        season_clock: RwLock::new(
            store_clock
                .and_then(|j| sv10_bot::season::SeasonClock::from_stored(&j, std::time::Instant::now(), now_unix))
                .unwrap_or_default(),
        ),
        current_season: RwLock::new(stored_season),
        champion_version: RwLock::new(champion_version),
        restart_requested: Default::default(),
        restore_requested: Default::default(),
        unstored_hands: Default::default(),
        aliases: RwLock::new(aliases),
        experiment: RwLock::new(Default::default()),
        resumable: Mutex::new(resumable),
        tv_cache: Default::default(),
        decision_gate: Arc::new(tokio::sync::Semaphore::new(sv10_bot::live::decision_permits(hardware.logical_cores))),
    });

    // Every numeric setting and the value in force after clamping, once (0250).
    tracing::info!("settings: {}", shared.config.describe());
    drop(sv10_bot::tasks::warm_tables());
    sv10_bot::tasks::spawn_all(&shared);
    spawn_release_watch(&shared);

    let api_shared = shared.clone();
    // Workers never serve the dashboard: every process binding the port would clash, and the
    // head merges worker heartbeats into its own fleet view (0128).
    if !config.worker {
        tokio::spawn(async move {
            if let Err(e) = api::serve(api_shared).await {
                tracing::error!("api server stopped: {e:#}");
            }
        });
    }

    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = term.recv() => {},
    }
    tracing::info!("shutting down; saving models");
    // Workers never save: the head owns the canonical models (0128); a worker overwrite would
    // drop the other processes' observations.
    if !shared.config.worker {
        sv10_bot::tasks::save_models_bounded(&shared);
    }
    // A stop lands mid-hand like a swap does: keep those hands for the next process (0315).
    shared.save_open_hands();
    sv10_bot::tasks::flush_unstored_hands(&shared);
    // Exit here instead of returning: dropping the runtime waits for every blocking task, and the
    // release watch below never returns, so a returned `main` left the process alive after SIGTERM
    // until a KILL (2026-10-04: four workers sat in shutdown for 3 minutes after `restart-bot.sh`).
    // Any exit code restarts the process under its supervisor.
    std::process::exit(0)
}

/// Swap to a newly installed release without stopping play: once `scripts/release.sh` has put a
/// working build at our path, wait until no bot is mid-turn, save models and exit with
/// `SWAP_EXIT_CODE`; the supervisor starts the new build and bots resync their seats.
fn spawn_release_watch(shared: &Arc<Shared>) {
    let watch = sv10_bot::release::ExeWatch::current();
    if watch.is_none() {
        tracing::warn!("release watch unavailable (executable path unknown); setup restarts still work");
    }
    let shared = shared.clone();
    tokio::task::spawn_blocking(move || {
        use std::sync::atomic::Ordering;
        use std::time::{Duration, Instant};
        loop {
            std::thread::sleep(Duration::from_secs(15));
            // The watchdog reads this: a watch that died would stop hot swaps without a word.
            let at = chrono::Utc::now().timestamp_millis() as f64 / 1000.0;
            let _ = shared.store.put_kv(sv10_bot::release::WATCH_KEY, &serde_json::json!({ "at": at }).to_string());
            let release = watch.as_ref().is_some_and(|w| w.replacement_ready(Duration::from_secs(20), "sv10-bot"));
            let setup = shared.restart_requested.load(Ordering::Relaxed);
            let restore = shared.restore_requested.load(Ordering::Relaxed);
            let Some((code, save)) = sv10_bot::release::exit_plan(release, setup, restore) else { continue };
            let why = match (&watch, release, restore) {
                (_, _, true) => "the live database failed its integrity check".to_string(),
                (Some(w), true, _) => format!("new release installed at {}", w.path().display()),
                _ => "bot setup saved from the dashboard".to_string(),
            };
            shared.log("fleet", "info", format!("{why}; restarting at the next moment no bot is mid-turn"));
            let deadline = Instant::now() + Duration::from_secs(90);
            while Instant::now() < deadline && shared.bots.iter().any(|b| in_turn(&b.read())) {
                std::thread::sleep(Duration::from_millis(100));
            }
            if save {
                if !shared.config.worker {
                    sv10_bot::tasks::save_models_bounded(&shared);
                }
                shared.save_open_hands();
                sv10_bot::tasks::flush_unstored_hands(&shared);
            }
            shared.log("fleet", "info", if restore { "exiting to restore the database" } else { "hot swap: exiting to restart" });
            std::process::exit(code);
        }
    });
}

/// A turn is in progress until our action is echoed; a marker older than a minute is stale
/// (table closed or resynced mid-turn) and does not block a swap.
fn in_turn(b: &BotLive) -> bool {
    let now = chrono::Utc::now().timestamp_millis() as f64 / 1000.0;
    b.turn_started.is_some_and(|t| now - t < 60.0)
}

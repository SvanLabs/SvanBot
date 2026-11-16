//! `svanbot`: one bot at the venue. It connects with the bot's key, plays every turn by the
//! policy (a re-solve heads up after the flop, the pricer otherwise), leaves any table a stablemate sits at, and reconnects with a backoff when the venue
//! closes the connection, resyncing the table it sat at if the venue still holds the seat.
//!
//! It reads its setup from the environment, as the venue's docs advise for keys:
//! - `SVANBOT_API_KEY`: the bot's key. It is never printed.
//! - `SVANBOT_NAME`: the bot's name at the venue.
//! - `SVANBOT_ROSTER`: every name of every bot the owner has registered, aliases included, comma
//!   separated. Fair play needs it, so the bot will not start without it.
//! - `SVANBOT_URL`: the venue's WebSocket URL, `wss://openpoker.ai/ws` unless set.
//! - `SVANBOT_STORE`: the bot's store, `~/.local/share/svanbot/<name>.db` unless set. Every hand
//!   and every fair-play incident is kept there, so the bot will not start without it.
//! - `SVANBOT_HEALTH`: where the bot writes what its updater reads, the `health` directory beside
//!   the store unless set (V11-10). The bot records each start there, so a crash loop is visible
//!   from outside the process, and reads the updater's request to stop between hands.
//!
//! `svanbot download` instead downloads the bot's season hand history into its store, and exits
//! ([`download`]). `svanbot paired` runs a paired sim of the policy against the pricer ([`paired`]).
//!
//! An app does I/O, so it turns off the `clippy.toml` bans here, once (ADR 0003).
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

mod download;
mod paired;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use svanbot_runtime::client::{
    Bot, ClientError, Config, Event, PlayOutcome, VENUE, fresh_seed, play_with_stop,
};
use svanbot_runtime::health::{self, Health};
use svanbot_runtime::store::{Source, Store};
use svanbot_strategy::params::Params;
use svanbot_strategy::policy::decide;
use svanbot_strategy::pricer::Budget;
use svanbot_strategy::resolve::iterations_for;

/// Deals sampled per decision: a small cost next to the 45 s action clock. `/measure` will size it.
const SAMPLES: u64 = 20_000;

/// The longest wait between reconnects. The venue allows ten connections a minute per IP.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

fn main() -> ExitCode {
    // Before the environment is read: a start is a start even when the setup is missing, and the
    // updater's crash loop must see a build that cannot read its env as a build that came up.
    let me = std::env::var("SVANBOT_NAME").unwrap_or_else(|_| "svanbot".into());
    if let Err(error) = open_health(&me).note_start(now_ms(), &running_version()) {
        eprintln!("svanbot: could not record the start: {error}");
    }
    if std::env::args().nth(1).as_deref() == Some("download") {
        return download::run();
    }
    if std::env::args().nth(1).as_deref() == Some("paired") {
        let args: Vec<String> = std::env::args().skip(2).collect();
        return paired::run(&args);
    }
    let config = match config_from_env() {
        Ok(config) => config,
        Err(why) => {
            eprintln!("svanbot: {why}");
            return ExitCode::from(2);
        }
    };
    let store = match open_store(&config.me) {
        Ok(store) => store,
        Err(why) => {
            eprintln!("svanbot: {why}");
            return ExitCode::from(2);
        }
    };
    // The updater's request to stop between hands, and the answer this bot gives it.
    let health = open_health(&config.me);
    eprintln!("svanbot: {} connects to {}", config.me, config.url);
    let mut bot = Bot::new(&config);
    let stop = || health.update_requested();
    let mut backoff = Duration::from_secs(1);
    loop {
        let ended = play_with_stop(
            &config,
            &mut bot,
            |spot| {
                let budget = Budget {
                    samples: SAMPLES,
                    seed: fresh_seed(),
                };
                // The compute budget: a bigger pot buys the re-solve more iterations.
                let iterations = iterations_for(spot.pot, spot.big_blind, spot.board.len());
                // No fitted action model yet: one is used only after it passes the gate.
                decide(spot, None, &Params::DEFAULT, budget, iterations)
            },
            |event| keep(&store, event),
            &stop,
        );
        match ended {
            Ok(PlayOutcome::Closed) => {
                eprintln!("svanbot: the venue closed the connection");
                backoff = Duration::from_secs(1);
            }
            Ok(PlayOutcome::Stopped) => {
                // Between hands, and the updater has asked for the build: say so, so the updater
                // knows the table is left, and end. Systemd starts the new build.
                eprintln!("svanbot: stopped between hands for an update");
                if let Err(error) = health.note_stop() {
                    eprintln!("svanbot: could not say it stopped: {error}");
                }
                return ExitCode::SUCCESS;
            }
            Err(ClientError::AuthFailed) => {
                eprintln!("svanbot: the venue refused the key; not retrying");
                return ExitCode::from(1);
            }
            Err(error) => eprintln!("svanbot: {error}"),
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}

/// Keeps a finished hand or an incident. A store failure is reported and play goes on: the next
/// decision does not wait on the disk.
fn keep(store: &Store, event: Event<'_>) {
    let kept = match event {
        Event::Incident(incident) => {
            // `<2>` is journald's priority prefix for crit, so `journalctl -p crit` lists every
            // incident and nothing else.
            eprintln!("<2>svanbot: FAIR-PLAY INCIDENT: {incident:?}");
            store.append_incident(incident, now_ms())
        }
        Event::Hand(hand) => store.append_hand(&hand, Source::Live, now_ms()).map(|_| ()),
    };
    if let Err(error) = kept {
        eprintln!("svanbot: NOT KEPT: {error}");
    }
}

/// The store's directory is also where the bot's health files live, one `health` subdirectory
/// beside the stores, so the updater finds them by the same rule and neither is told a path.
fn open_health(me: &str) -> Health {
    let dir = match std::env::var_os("SVANBOT_HEALTH") {
        Some(path) => PathBuf::from(path),
        None => store_path(me)
            .map(health::beside_store)
            .unwrap_or_else(|_| PathBuf::from("health")),
    };
    Health::in_dir(dir, me)
}

/// The build this bot is running: the name of the directory `current` points at, which is the
/// version the updater installed. Empty when the bot runs from a checkout, where a start with no
/// version is still a start.
fn running_version() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|exe| std::fs::canonicalize(exe).ok())
        .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
        .and_then(|dir| {
            dir.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_default()
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

/// The bot's store: `SVANBOT_STORE`, or `~/.local/share/svanbot/<name>.db`.
pub(crate) fn store_path(me: &str) -> Result<PathBuf, String> {
    match std::env::var_os("SVANBOT_STORE") {
        Some(path) => Ok(PathBuf::from(path)),
        None => {
            let home = std::env::var_os("HOME").ok_or("set SVANBOT_STORE or HOME for the store")?;
            Ok(PathBuf::from(home).join(format!(".local/share/svanbot/{me}.db")))
        }
    }
}

pub(crate) fn open_store(me: &str) -> Result<Store, String> {
    let path = store_path(me)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("store: cannot make {}: {e}", dir.display()))?;
    }
    Store::open(&path, me).map_err(|e| format!("{e} at {}", path.display()))
}

fn config_from_env() -> Result<Config, String> {
    let need = |name: &str| std::env::var(name).map_err(|_| format!("set {name}"));
    let roster: Vec<String> = need("SVANBOT_ROSTER")?
        .split(',')
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .collect();
    if roster.is_empty() {
        return Err("SVANBOT_ROSTER names no bot: fair play needs the whole roster".into());
    }
    Ok(Config {
        url: std::env::var("SVANBOT_URL").unwrap_or_else(|_| VENUE.to_owned()),
        key: need("SVANBOT_API_KEY")?,
        me: need("SVANBOT_NAME")?,
        roster,
    })
}

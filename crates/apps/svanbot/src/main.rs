//! `svanbot`: one bot at the venue. It connects with the bot's key, plays every turn by the
//! pricer, leaves any table a stablemate sits at, and reconnects with a backoff when the venue
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
//!
//! An app does I/O, so it turns off the `clippy.toml` bans here, once (ADR 0003).
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use svanbot_runtime::client::{Bot, ClientError, Config, Event, VENUE, fresh_seed, play};
use svanbot_runtime::store::{Source, Store};
use svanbot_strategy::pricer::{Budget, decide};

/// Deals sampled per decision: a small cost next to the 45 s action clock. `/measure` will size it.
const SAMPLES: u64 = 20_000;

/// The longest wait between reconnects. The venue allows ten connections a minute per IP.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

fn main() -> ExitCode {
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
    eprintln!("svanbot: {} connects to {}", config.me, config.url);
    let mut bot = Bot::new(&config);
    let mut backoff = Duration::from_secs(1);
    loop {
        let ended = play(
            &config,
            &mut bot,
            |spot| {
                let budget = Budget {
                    samples: SAMPLES,
                    seed: fresh_seed(),
                };
                decide(spot, budget).action
            },
            |event| keep(&store, event),
        );
        match ended {
            Ok(()) => {
                eprintln!("svanbot: the venue closed the connection");
                backoff = Duration::from_secs(1);
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

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

fn open_store(me: &str) -> Result<Store, String> {
    let path = match std::env::var_os("SVANBOT_STORE") {
        Some(path) => PathBuf::from(path),
        None => {
            let home = std::env::var_os("HOME").ok_or("set SVANBOT_STORE or HOME for the store")?;
            PathBuf::from(home).join(format!(".local/share/svanbot/{me}.db"))
        }
    };
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

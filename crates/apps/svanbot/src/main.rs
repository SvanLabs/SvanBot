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
//!
//! An app does I/O, so it turns off the `clippy.toml` bans here, once (ADR 0003).
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

use std::process::ExitCode;
use std::time::Duration;

use svanbot_runtime::client::{ClientError, Config, VENUE, fresh_seed, play};
use svanbot_runtime::session::Session;
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
    eprintln!("svanbot: {} connects to {}", config.me, config.url);
    let mut session = Session::new(&config.me, config.roster.iter().cloned());
    let mut backoff = Duration::from_secs(1);
    loop {
        let ended = play(
            &config,
            &mut session,
            |spot| {
                let budget = Budget {
                    samples: SAMPLES,
                    seed: fresh_seed(),
                };
                decide(spot, budget).action
            },
            |incident| eprintln!("svanbot: FAIR-PLAY INCIDENT: {incident:?}"),
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

//! The venue client: one WebSocket connection, driven through a [`Session`].
//!
//! It connects to `wss://openpoker.ai/ws` with the bot's key as a bearer token, reads frames,
//! hands each to the session, and sends what the session answers. [`play`] runs one connection
//! until it closes. Reconnecting, with a backoff, is the caller's choice: it passes the same
//! session back in, so the session resyncs the table it sits at.

use std::fmt;

use svanbot_core::rules::Action;
use svanbot_core::spot::Spot;
use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderValue;
use tungstenite::http::header::AUTHORIZATION;
use tungstenite::{Message, WebSocket};

use crate::limit::Limiter;
use crate::session::{Incident, Session};
use crate::wire::{self, Inbound, Outbound};

/// The venue's WebSocket endpoint.
pub const VENUE: &str = "wss://openpoker.ai/ws";

/// Who connects, and where.
#[derive(Clone)]
pub struct Config {
    pub url: String,
    /// The bot's API key. Never printed: `Debug` leaves it out.
    pub key: String,
    /// The bot's name at the venue.
    pub me: String,
    /// Every name of every bot the owner has registered, aliases included.
    pub roster: Vec<String>,
}

impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("url", &self.url)
            .field("key", &"<hidden>")
            .field("me", &self.me)
            .field("roster", &self.roster)
            .finish()
    }
}

/// Why a connection ended badly.
#[derive(Debug)]
pub enum ClientError {
    /// The venue refused the key. Retrying with the same key cannot help.
    AuthFailed,
    /// The URL or the key cannot make a request.
    BadConfig(String),
    /// The socket failed.
    Socket(tungstenite::Error),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthFailed => write!(f, "the venue refused the key"),
            Self::BadConfig(why) => write!(f, "bad config: {why}"),
            Self::Socket(error) => write!(f, "socket: {error}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<tungstenite::Error> for ClientError {
    fn from(error: tungstenite::Error) -> Self {
        Self::Socket(error)
    }
}

/// Plays one connection until the venue closes it. `decide` is asked at each of our turns;
/// `on_incident` hears every fair-play incident the session raises.
pub fn play(
    config: &Config,
    session: &mut Session,
    mut decide: impl FnMut(&Spot) -> Action,
    mut on_incident: impl FnMut(&Incident),
) -> Result<(), ClientError> {
    let mut request = config
        .url
        .as_str()
        .into_client_request()
        .map_err(|e| ClientError::BadConfig(e.to_string()))?;
    let bearer = HeaderValue::from_str(&format!("Bearer {}", config.key))
        .map_err(|_| ClientError::BadConfig("the key is not a valid header value".into()))?;
    request.headers_mut().insert(AUTHORIZATION, bearer);
    let (mut socket, _) = tungstenite::connect(request)?;
    let mut limiter = Limiter::venue();
    loop {
        let text = match socket.read() {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => continue,
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        let frame = match wire::parse_frame(text.as_str()) {
            Ok(frame) => frame,
            Err(error) => {
                eprintln!("svanbot: skipped a frame: {error}");
                continue;
            }
        };
        let outbound = match &frame.inbound {
            Inbound::Connected { .. } => session.on_connect(),
            Inbound::Error { code, .. } if code == "auth_failed" => {
                return Err(ClientError::AuthFailed);
            }
            inbound => {
                if let Inbound::Error { code, .. } = inbound {
                    eprintln!("svanbot: the venue reported {code}");
                }
                let step = session.on_frame(frame, |spot| decide(spot));
                step.incidents.iter().for_each(&mut on_incident);
                step.send
            }
        };
        send_all(&mut socket, &mut limiter, &outbound)?;
    }
}

/// Sends each message in turn, waiting where the venue's rate limit asks for it.
fn send_all<S: std::io::Read + std::io::Write>(
    socket: &mut WebSocket<S>,
    limiter: &mut Limiter,
    messages: &[Outbound],
) -> Result<(), ClientError> {
    for message in messages {
        let wait = limiter.admit(std::time::Instant::now());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        socket.send(Message::text(message.to_json()))?;
    }
    Ok(())
}

/// A seed from fresh entropy, for a decision's budget. The pure layers never draw entropy: this
/// is where it comes from (ADR 0003).
pub fn fresh_seed() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::hash::RandomState::new().build_hasher();
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos()),
    );
    hasher.finish()
}

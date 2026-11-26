//! The venue client: one WebSocket connection, driven through a [`Session`].
//!
//! It connects to `wss://openpoker.ai/ws` with the bot's key as a bearer token, reads frames,
//! hands each to the session, and sends what the session answers. [`play`] runs one connection
//! until it closes. Reconnecting, with a backoff, is the caller's choice: it passes the same
//! [`Bot`] back in, so the session resyncs the table it sits at and the recorder finishes the
//! hand it was gathering.
//!
//! [`play_with_stop`] adds the seam an update needs ([`crate::slot`]): a stop check that ends the
//! connection between hands, so the updater can swap the slot while no hand is in flight.

use std::fmt;
use std::net::TcpStream;
use std::time::Duration;

use svanbot_core::spot::Spot;
use svanbot_strategy::policy::Decision;
use tungstenite::client::IntoClientRequest;
use tungstenite::http::HeaderValue;
use tungstenite::http::Uri;
use tungstenite::http::header::AUTHORIZATION;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::limit::Limiter;
use crate::record::{Hand, Recorder};
use crate::rest::{API, Rest, RestError};
use crate::session::{Incident, Session};
use crate::status::Table;
use crate::wire::{self, Inbound, Outbound, Player};

/// The venue's WebSocket endpoint.
pub const VENUE: &str = "wss://openpoker.ai/ws";

fn public_venue(uri: &Uri) -> bool {
    uri.scheme_str() == Some("wss")
        && uri
            .host()
            .is_some_and(|host| host.eq_ignore_ascii_case("openpoker.ai"))
        && uri.port_u16().unwrap_or(443) == 443
        && uri.path() == "/ws"
        && uri.query().is_none_or(str::is_empty)
}

/// Puts `wait` on the socket under `socket`, so a read the venue has nothing to answer returns
/// instead of blocking. Both stream kinds reach the same [`std::net::TcpStream`]: plain directly,
/// TLS through rustls's `StreamOwned::sock`.
fn set_read_wait(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>, wait: Duration) {
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => {
            let _ = stream.set_read_timeout(Some(wait));
        }
        MaybeTlsStream::Rustls(stream) => {
            let _ = stream.sock.set_read_timeout(Some(wait));
        }
        _ => {}
    }
}

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
    /// Cold-start seat discovery failed before the bot could connect.
    SeatDiscovery(RestError),
    /// The socket failed.
    Socket(tungstenite::Error),
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AuthFailed => write!(f, "the venue refused the key"),
            Self::BadConfig(why) => write!(f, "bad config: {why}"),
            Self::SeatDiscovery(error) => write!(f, "active game: {error}"),
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

/// What a connection tells its caller, besides the moves it plays.
#[derive(Debug)]
pub enum Event<'a> {
    /// The venue connection opened.
    Connected,
    /// The table the bot sits at changed, or it left one. `None` means it sits at none.
    Table(Option<Table>),
    /// A fair-play incident the session raised.
    Incident(&'a Incident),
    /// A hand the recorder finished.
    Hand(Hand),
}

/// What a bot carries from one connection to the next.
#[derive(Clone, Debug)]
pub struct Bot {
    pub session: Session,
    pub recorder: Recorder,
    seat_discovered: bool,
}

impl Bot {
    /// A bot with nothing seen yet.
    pub fn new(config: &Config) -> Self {
        Self {
            session: Session::new(&config.me, config.roster.iter().cloned()),
            recorder: Recorder::default(),
            seat_discovered: false,
        }
    }

    /// Reads the public active game once before a cold connection can join the lobby.
    pub fn discover_seat(&mut self, rest: &mut Rest) -> Result<(), ClientError> {
        if self.seat_discovered {
            return Ok(());
        }
        let body = rest.get("/me/active-game").map_err(|error| match error {
            RestError::AuthFailed => ClientError::AuthFailed,
            error => ClientError::SeatDiscovery(error),
        })?;
        let missing = |field| {
            ClientError::SeatDiscovery(RestError::Body(format!("missing or invalid {field}")))
        };
        if body["playing"]
            .as_bool()
            .ok_or_else(|| missing("playing"))?
        {
            let table_id = body["table_id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| missing("table_id"))?;
            let seat = body["seat"]
                .as_u64()
                .filter(|seat| *seat < 6)
                .ok_or_else(|| missing("seat"))?;
            self.session.resume_seat(table_id, seat as usize);
        }
        self.seat_discovered = true;
        Ok(())
    }
}

/// How many milliseconds a read waits before it lets the stop check run again. A stop is only
/// obeyed between hands, so the wait costs nothing while a hand is being played.
const READ_WAIT_MS: u64 = 100;

/// Why [`play_with_stop`] ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayOutcome {
    /// The venue closed the connection.
    Closed,
    /// The stop check said so, between hands: the caller may now replace the build.
    Stopped,
}

/// Plays one connection until the venue closes it. `decide` is asked at each of our turns;
/// `on_event` hears every incident and every finished hand.
pub fn play<D: Into<Decision>>(
    config: &Config,
    bot: &mut Bot,
    decide: impl FnMut(&Spot) -> D,
    on_event: impl FnMut(Event<'_>),
) -> Result<PlayOutcome, ClientError> {
    play_with_stop(config, bot, decide, on_event, &|| false)
}

/// Plays one connection, stopped between hands when `stop` says so. Between hands the venue has
/// sent the last frame of the hand it dealt and is waiting for the next, so a stop here never
/// cuts a hand: it is exactly when an update may replace the build. `stop` is asked before a
/// blocking read, so it lands within [`READ_WAIT_MS`] of being set.
pub fn play_with_stop<D: Into<Decision>>(
    config: &Config,
    bot: &mut Bot,
    mut decide: impl FnMut(&Spot) -> D,
    on_event: impl FnMut(Event<'_>),
    stop: &dyn Fn() -> bool,
) -> Result<PlayOutcome, ClientError> {
    play_with_players(config, bot, |spot, _| decide(spot), on_event, stop)
}

/// Plays a connection with the current turn's player identities available to each decision.
pub fn play_with_players<D: Into<Decision>>(
    config: &Config,
    bot: &mut Bot,
    mut decide: impl FnMut(&Spot, &[Player]) -> D,
    mut on_event: impl FnMut(Event<'_>),
    stop: &dyn Fn() -> bool,
) -> Result<PlayOutcome, ClientError> {
    let mut request = config
        .url
        .as_str()
        .into_client_request()
        .map_err(|e| ClientError::BadConfig(e.to_string()))?;
    let bearer = HeaderValue::from_str(&format!("Bearer {}", config.key))
        .map_err(|_| ClientError::BadConfig("the key is not a valid header value".into()))?;
    request.headers_mut().insert(AUTHORIZATION, bearer);
    if public_venue(request.uri()) && !bot.seat_discovered {
        let mut rest = Rest::new(API, &config.key, Limiter::new(60, Duration::from_secs(60)));
        bot.discover_seat(&mut rest)?;
    }
    let (mut socket, _) = tungstenite::connect(request)?;
    // The read timeout is what lets a stop interrupt a read the venue has left idle. A timeout is
    // not an error here: it is an empty read, and the loop asks `stop` again.
    set_read_wait(&mut socket, Duration::from_millis(READ_WAIT_MS));
    on_event(Event::Connected);
    // A session outlives its connection, so a reconnect may already know the table.
    let mut last_table = bot.session.table();
    if let Some(table) = last_table.clone() {
        on_event(Event::Table(Some(table)));
    }
    let mut limiter = Limiter::venue();
    loop {
        if stop() {
            return Ok(PlayOutcome::Stopped);
        }
        let text = match socket.read() {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) => return Ok(PlayOutcome::Closed),
            Ok(_) => continue,
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Ok(PlayOutcome::Closed);
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
        if let Some(hand) = bot.recorder.record(text.as_str(), &frame) {
            on_event(Event::Hand(hand));
        }
        let outbound = match &frame.inbound {
            Inbound::Connected { .. } => bot.session.on_connect(),
            Inbound::Error { code, .. } if code == "auth_failed" => {
                return Err(ClientError::AuthFailed);
            }
            inbound => {
                if let Inbound::Error { code, .. } = inbound {
                    eprintln!("svanbot: the venue reported {code}");
                }
                let step = bot.session.on_frame_with_players(frame, &mut decide);
                for incident in &step.incidents {
                    on_event(Event::Incident(incident));
                }
                bot.recorder.trace(step.traced);
                step.send
            }
        };
        let table = bot.session.table();
        if table != last_table {
            on_event(Event::Table(table.clone()));
            last_table = table;
        }
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
    use std::hash::{BuildHasher, RandomState};

    RandomState::new().hash_one("svanbot decision")
}

#[cfg(test)]
mod tests {
    use super::public_venue;

    #[test]
    fn only_the_public_venue_connection_uses_public_seat_discovery() {
        for url in [
            "wss://openpoker.ai/ws",
            "wss://OPENPOKER.AI:443/ws",
            "wss://openpoker.ai/ws?",
        ] {
            assert!(public_venue(&url.parse().unwrap()), "{url}");
        }
        for url in [
            "wss://openpoker.ai/ws?competition_id=private",
            "ws://openpoker.ai/ws",
            "wss://openpoker.ai:8443/ws",
            "ws://127.0.0.1/ws",
        ] {
            assert!(!public_venue(&url.parse().unwrap()), "{url}");
        }
    }
}

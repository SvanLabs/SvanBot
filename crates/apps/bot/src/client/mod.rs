//! One bot's connection to openpoker.ai: cold/warm start, lobby, table play,
//! resync, rebuys, and reconnection with backoff.
//!
//! `mod.rs` holds the connection lifecycle (`run_bot`, `session`); `handler` dispatches server
//! messages, `seat` owns the seat lifecycle and the between-hands table moves, `decide` turns a turn into a legal action, `rest` covers REST helpers, `quality`
//! rates tables.

use crate::config::BotConfig;
use crate::live::{DecisionView, Shared};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::{Duration, Instant};
use sv10_core::engine::Action;
use sv10_core::policy::decide_with;
use sv10_rng::SeedableRng;
use sv10_rng::rngs::SmallRng;
use sv10_store::store::HandRow;
use sv10_venue::tracker::{LegalActions, TableTracker};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// Resyncs asked for after action rejections per hand (0163): enough to recover a stale token or
/// a corrected action, few enough that a persistent rejection cannot flood (spec: 10+ rejections
/// in 5 s is a flood warning).
pub const MAX_REJECT_RESYNCS: u32 = 3;

use crate::USER_AGENT;

mod authpark;
mod decide;
mod handler;
mod quality;
mod recover;
mod rest;
mod seat;
mod turns;

// The calibration round (a blocking-pool caller) shares the 0322 locked-write retry (#744).
pub(crate) use decide::retry_locked_write_blocking;

use decide::act;
pub use decide::legalize;
use handler::handle;
pub use quality::TableQuality;
use quality::table_quality;
use rest::balance_from;
use rest::prepare_buy_in;
pub use rest::rest_get;
use seat::{HandEnd, Leave, Move, Seat, between_hands, stablemate_at_table, top_up_funded};

pub(super) struct Conn {
    out: mpsc::UnboundedSender<Value>,
}

impl Conn {
    /// Queue a frame for the writer. False when the writer task has died: the frame is lost and
    /// the session is about to end (the loop watches the writer), so callers must not treat it as sent.
    fn send(&self, v: Value) -> bool {
        self.out.send(v).is_ok()
    }
}

pub async fn run_bot(shared: Arc<Shared>, slot: usize, bot: BotConfig) {
    let http = reqwest::Client::builder().user_agent(USER_AGENT).timeout(Duration::from_secs(20)).build().unwrap();
    let mut tracker = TableTracker::default();
    tracker.reset_table();
    let mut backoff = Duration::from_secs(1);
    let mut auth_park = authpark::AuthPark::default();
    let mut rng = SmallRng::from_os();
    loop {
        let desired = shared.bots[slot].read().desired.clone();
        if desired == "stop" {
            shared.update(slot, |b| {
                b.mode = "stopped".into();
                b.connected = false;
            });
            tokio::time::sleep(Duration::from_secs(2)).await;
            continue;
        }
        shared.update(slot, |b| b.mode = "connecting".into());
        let active = rest_get(&http, &shared, &bot, "/me/active-game").await;
        if let Some(a) = &active
            && a["playing"].as_bool() == Some(true)
            && tracker.table_id.is_none()
        {
            tracker.table_id = a["table_id"].as_str().map(String::from);
            // A cold start that finds us already seated: the server kept (or gave) us a seat while no
            // client was connected. Logged with the stack so an outage's chip cost is measurable
            // (2026-09-22: bots came back seated at new tables after 12.5 h offline, some busted; 0145).
            // The spec's documented cold-restart path (active-game playing:true, then resync): INFO,
            // so real warnings stand out (0246).
            shared.log(
                &bot.name,
                "info",
                format!(
                    "cold start found us seated at table {} seat {} with stack {}",
                    a["table_id"].as_str().unwrap_or("?"),
                    a["seat"],
                    a["stack_chips"]
                ),
            );
        }
        let began = Instant::now();
        let ended = session(&shared, slot, &bot, &http, &mut tracker, &mut rng).await;
        let lasted = began.elapsed();
        let ended = match ended {
            // One refused login must not cost the seat: retry on the backoff path until the third in a row (#744).
            Ok(SessionEnd::Fatal(reason)) if reason.starts_with("auth_failed") => {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
                let rotated = shared.store.get_kv(authpark::ROTATION_KEY).ok().flatten().and_then(|v| v.trim().parse::<f64>().ok());
                match auth_park.failed(now, now - lasted.as_secs_f64(), rotated) {
                    authpark::Verdict::Park => Ok(SessionEnd::Fatal(reason)),
                    authpark::Verdict::Retry(n) => {
                        shared.log(&bot.name, "warn", format!("{reason}; retrying ({n} of {} before parking)", authpark::LIMIT));
                        Ok(SessionEnd::Throttled)
                    }
                }
            }
            other => {
                auth_park.reset();
                other
            }
        };
        match ended {
            Ok(SessionEnd::Fatal(reason)) => {
                shared.log(&bot.name, "error", format!("stopping: {reason}"));
                shared.update(slot, |b| {
                    b.mode = "error".into();
                    b.connected = false;
                    b.last_error = Some(reason.clone());
                    b.desired = "stop".into();
                });
            }
            Ok(end @ (SessionEnd::Closed | SessionEnd::Throttled)) => {
                backoff = next_backoff(Some(&end), backoff, lasted);
                shared.update(slot, |b| {
                    b.connected = false;
                    if b.mode != "stopped" {
                        b.mode = "offline".into();
                    }
                });
            }
            Err(e) => {
                shared.log(&bot.name, "warn", format!("connection error: {e:#}"));
                shared.update(slot, |b| {
                    b.connected = false;
                    b.mode = "offline".into();
                    b.last_error = Some(format!("{e:#}"));
                });
                backoff = next_backoff(None, backoff, lasted);
            }
        }
        let jitter = Duration::from_millis(sv10_rng::RngExt::random_range(&mut rng, 0..1000));
        tokio::time::sleep(backoff + jitter).await;
    }
}

pub(super) enum SessionEnd {
    Closed,
    /// The server refused the connection for too many connection attempts in the shared play
    /// pool; reconnecting at the normal 1 s cadence keeps every bot throttled (2026-09-22 start:
    /// 40 refusals over 60 s).
    Throttled,
    Fatal(String),
}

/// First wait after a connection-attempt refusal; doubles per refusal up to the normal 60 s cap.
const THROTTLE_BACKOFF: Duration = Duration::from_secs(5);

/// How long a bot may sit unseated before the loop asks the lobby for a table again. A timer, not a
/// silence detector: the spec promises no lobby heartbeat, but a bot that depends on the server
/// saying nothing is one frame away from never re-seating (0251).
pub(super) const UNSEATED_REJOIN: Duration = Duration::from_secs(120);

/// Whether a bot that has been off its seat for `unseated_for` should ask the lobby again (0251).
/// A timer, not a silence detector: the old rule waited for two minutes of *total* silence, so any
/// lobby frame more often than that left the bot unseated indefinitely.
pub(super) fn should_rejoin_unseated(unseated_for: Duration, pending_join: bool) -> bool {
    !pending_join && unseated_for >= UNSEATED_REJOIN
}

/// How long an unseated bot has waited since it lost its seat or last asked the lobby, whichever
/// is later. Timing only from the lost seat re-sent `join_lobby` on every backoff step once the
/// first two minutes had passed, while the bot was still queued (the server answers
/// `already_in_lobby`; 2026-09-27 sweep).
pub(super) fn unseated_wait(unseated_since: Option<Instant>, last_join: Option<Instant>, now: Instant) -> Duration {
    let since = match (unseated_since, last_join) {
        (Some(u), Some(j)) => Some(u.max(j)),
        (u, j) => u.or(j),
    };
    since.map(|t| now.saturating_duration_since(t)).unwrap_or_default()
}

/// A session that lasted this long was healthy: its end starts a fresh backoff ladder.
pub(super) const HEALTHY_SESSION: Duration = Duration::from_secs(60);

/// Reconnect wait after a session that `lasted` ends (`None` is a transport error). The ladder
/// restarts after a healthy session: a mid-play reset has a turn deadline (45 s) and a seat grace
/// (120 s) running, so it must not inherit the cap earlier failures reached (2026-09-26, 0246).
pub(super) fn next_backoff(end: Option<&SessionEnd>, prev: Duration, lasted: Duration) -> Duration {
    let prev = if lasted >= HEALTHY_SESSION { Duration::from_secs(1) } else { prev };
    match end {
        Some(SessionEnd::Closed) => Duration::from_secs(1),
        Some(SessionEnd::Throttled) => (prev * 2).clamp(THROTTLE_BACKOFF, Duration::from_secs(60)),
        Some(SessionEnd::Fatal(_)) => prev,
        None => (prev * 2).min(Duration::from_secs(60)),
    }
}

async fn session(
    shared: &Arc<Shared>,
    slot: usize,
    bot: &BotConfig,
    http: &reqwest::Client,
    tracker: &mut TableTracker,
    rng: &mut SmallRng,
) -> anyhow::Result<SessionEnd> {
    let mut req = shared.config.ws_url.as_str().into_client_request()?;
    req.headers_mut().insert("Authorization", format!("Bearer {}", bot.api_key).parse()?);
    req.headers_mut().insert("User-Agent", USER_AGENT.parse()?);
    let (ws, _) = tokio::time::timeout(Duration::from_secs(20), tokio_tungstenite::connect_async(req)).await??;
    let (mut write, mut read) = ws.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Value>();
    let mut writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(Duration::from_secs(20));
        loop {
            tokio::select! {
                msg = out_rx.recv() => match msg {
                    Some(v) => if write.send(Message::Text(v.to_string().into())).await.is_err() { break },
                    None => break,
                },
                _ = ping.tick() => if write.send(Message::Ping(Vec::new().into())).await.is_err() { break },
            }
        }
    });
    let conn = Conn { out: out_tx };
    shared.update(slot, |b| {
        b.connected = true;
        b.connected_since = Some(chrono::Utc::now().to_rfc3339());
        b.last_error = None;
    });
    let mut last_join: Option<Instant> = None;
    let mut join_attempts: u32 = 0;
    // When we last had a seat (None: never seated this session). The unseated rejoin is a *timer*,
    // not a silence detector: a lobby frame every minute must not keep a bot off its seat forever
    // (0261's rejoin depended on total silence, which the server does not promise).
    let mut unseated_since: Option<Instant> = Some(Instant::now());
    let mut seat = Seat::default();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut last_frame = Instant::now();
    // Outcome watchdog: traffic without completed hands (stuck table, empty table) also re-queues.
    let mut hands_seen = shared.bots[slot].read().session_hands;
    let mut hands_changed = Instant::now();
    let result = loop {
        let recovery_deadline = seat.turn.as_ref().map(|turn| turn.deadline);
        tokio::select! {
            _ = turns::wait(recovery_deadline) => {
                turns::finish(shared, slot, bot, tracker, &conn, rng, &mut seat.turn, Instant::now()).await;
            }
            // A dead writer silently swallows every later action: end the session at once so the
            // reconnect resyncs and re-answers any pending turn (0090).
            _ = &mut writer => {
                shared.log(&bot.name, "warn", "websocket writer stopped; reconnecting");
                break Ok(SessionEnd::Closed);
            }
            frame = read.next() => {
                let Some(frame) = frame else { break Ok(SessionEnd::Closed) };
                let frame = match frame {
                    Ok(f) => f,
                    Err(e) => break Err(e.into()),
                };
                let text = match frame {
                    Message::Text(t) => t.to_string(),
                    Message::Close(c) => {
                        if let Some(c) = &c
                            && u16::from(c.code) == 4001 {
                                break Ok(SessionEnd::Fatal("auth_failed (close 4001)".into()));
                            }
                        break Ok(SessionEnd::Closed);
                    }
                    _ => continue,
                };
                last_frame = Instant::now();
                let Ok(msg) = serde_json::from_str::<Value>(&text) else { continue };
                if let Some(end) = handle(shared, slot, bot, http, tracker, &conn, &msg, rng, &mut seat).await { break Ok(end) }
            }
            _ = tick.tick() => {
                let desired = shared.bots[slot].read().desired.clone();
                let seated = tracker.table_id.is_some();
                if desired != "run" && seated && seat.leave(Leave::Pause) {
                    conn.send(json!({"type": "leave_table"}));
                    shared.log(&bot.name, "info", "operator requested leave; leaving after this hand");
                }
                if desired == "stop" && !seated {
                    break Ok(SessionEnd::Closed);
                }
                // Watchdog: a seated bot hears table traffic every few seconds; silence means a
                // dead or half-open connection. Unseated, re-issue the join periodically.
                // Wind-down tables are silent by design; reconnecting then only churns (0143).
                if desired == "run" && seated && last_frame.elapsed() > Duration::from_secs(180) && !shared.season_clock.read().silence_expected(Instant::now()) {
                    shared.log(&bot.name, "warn", "watchdog: no table traffic for 3 minutes; reconnecting");
                    break Ok(SessionEnd::Closed);
                }
                let hands_now = shared.bots[slot].read().session_hands;
                if hands_now != hands_seen || !seated {
                    hands_seen = hands_now;
                    hands_changed = Instant::now();
                }
                // A re-queue, not a pause: the leave completes into a rejoin (0206).
                if desired == "run" && seated && !seat.leaving() && hands_changed.elapsed() > Duration::from_secs(600) && shared.season_clock.read().table_moves_allowed(Instant::now()) {
                    shared.log(&bot.name, "warn", "watchdog: seated with no completed hand for 10 minutes; leaving to re-queue");
                    seat.leave(Leave::Rejoin);
                    conn.send(json!({"type": "leave_table"}));
                    hands_changed = Instant::now();
                }
                let unseated_for = if seated { Duration::ZERO } else { unseated_wait(unseated_since, last_join, Instant::now()) };
                if should_rejoin_unseated(unseated_for, seat.pending_join) {
                    if !seat.pending_join {
                        shared.log(&bot.name, "warn", format!("watchdog: no seat for {} minutes; rejoining lobby", unseated_for.as_secs() / 60));
                    }
                    seat.pending_join = true;
                }
                if seated {
                    join_attempts = 0;
                    unseated_since = None;
                } else if unseated_since.is_none() {
                    unseated_since = Some(Instant::now());
                }
                // Back off repeated joins (e.g. during season wind-down) instead of hammering the lobby.
                let join_gap = Duration::from_secs((8u64 << (join_attempts / 3).min(4)).min(120));
                if desired == "run" && seat.pending_join && last_join.map(|t| t.elapsed() > join_gap).unwrap_or(true) {
                    last_join = Some(Instant::now());
                    join_attempts += 1;
                    seat.pending_join = false;
                    if let Some(buy_in) = prepare_buy_in(http, shared, slot, bot).await {
                        conn.send(json!({"type": "join_lobby", "buy_in": buy_in}));
                        conn.send(json!({"type": "set_auto_rebuy", "enabled": true}));
                        shared.log(&bot.name, "info", format!("joining lobby with buy-in {buy_in}"));
                    } else {
                        seat.pending_join = true;
                        last_join = Some(Instant::now() + Duration::from_secs(52));
                    }
                }
            }
        }
    };
    writer.abort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0251: the unseated rejoin is a timer. Before, it waited for two minutes with no frame at
    /// all, so a lobby that says anything — a notice, a keepalive, a heartbeat added later — would
    /// keep the bot off its seat forever with nothing in the log.
    #[test]
    fn an_unseated_bot_rejoins_on_a_timer_not_on_silence() {
        assert!(!should_rejoin_unseated(Duration::from_secs(119), false), "not yet two minutes");
        assert!(should_rejoin_unseated(Duration::from_secs(120), false), "two minutes is the bar");
        assert!(should_rejoin_unseated(Duration::from_secs(3_600), false), "however long it takes");
        assert!(!should_rejoin_unseated(Duration::from_secs(3_600), true), "and only once: a join is already queued");
    }

    #[test]
    fn a_bot_waiting_in_the_lobby_is_not_re_queued_every_backoff_step() {
        let now = Instant::now();
        let lost = now - Duration::from_secs(600);
        // Ten minutes unseated but queued 10 s ago: wait for that join, do not send another.
        assert_eq!(unseated_wait(Some(lost), Some(now - Duration::from_secs(10)), now), Duration::from_secs(10));
        assert!(!should_rejoin_unseated(unseated_wait(Some(lost), Some(now - Duration::from_secs(10)), now), false));
        // A join older than the lost seat does not shorten the wait; no join yet counts from the seat.
        assert_eq!(unseated_wait(Some(lost), Some(lost - Duration::from_secs(5)), now), Duration::from_secs(600));
        assert_eq!(unseated_wait(Some(lost), None, now), Duration::from_secs(600));
        assert_eq!(unseated_wait(None, None, now), Duration::ZERO);
    }
}

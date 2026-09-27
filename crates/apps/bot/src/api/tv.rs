//! The public TV: a second listener that serves the table view and nothing else.
//!
//! Everything on it is unauthenticated — that is the point of it — so the surface itself is the
//! security boundary, and it is drawn twice. The router carries the table view, a health answer and
//! the built page's files, and returns 404 to every other `/api/` path: a dashboard route mounted
//! here by accident would be served to the public with no token, which [`tests`] refuses. The payload
//! carries only the keys [`PUBLIC_BOT_KEYS`] and [`PUBLIC_SEAT_KEYS`] name, so a field added to the
//! dashboard's table payload is absent here until somebody adds it on purpose.

use super::{off_runtime, state};
use crate::live::Shared;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::{Json, Router};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::convert::Infallible;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tower_http::services::{ServeDir, ServeFile};

/// The keys of a bot's table payload a spectator may see. What is deliberately absent:
///
/// - `hole` — the bot's own cards. A live stream of them is a hand shown to the opponents it is
///   playing against, and there is no flag in this payload that says a hand has ended, so the rule is
///   the simple one: the board is public, a hand in it is not.
/// - `decision` — the candidates, their EVs, the equity and the reason it chose, which is the policy's
///   own working. `version` goes with it: which policy is playing is not a spectator's business.
/// - `turn`, `turn_started` — when it is thinking, and for how long. Think time is a tell the project's
///   own response model is fitted on, and a public clock is that tell given away live.
/// - `last_error`, `season` — process internals and the operator's figures.
const PUBLIC_BOT_KEYS: [&str; 14] = [
    "slot",
    "name",
    "mode",
    "status",
    "connected",
    "table_id",
    "hand_id",
    "board",
    "seats",
    "hero_seat",
    "dealer_seat",
    "actor_seat",
    "pot",
    "big_blind",
];

/// The keys of one seat a spectator may see. `read` — the fitted opponent profile, its style and
/// tendencies — is the one that matters: it is the model's opinion of a named person.
const PUBLIC_SEAT_KEYS: [&str; 8] = ["seat", "name", "stack", "bet", "folded", "status", "last_action", "avatar_url"];

/// One table payload reduced to the public keys. Unknown keys are dropped rather than passed through,
/// so this stays true when the dashboard's payload grows.
pub(super) fn public_table(v: &Value) -> Value {
    let Some(obj) = v.as_object() else { return Value::Null };
    let mut out = serde_json::Map::new();
    for key in PUBLIC_BOT_KEYS {
        if let Some(value) = obj.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    if let Some(seats) = out.get_mut("seats").and_then(Value::as_array_mut) {
        for seat in seats.iter_mut() {
            if let Some(o) = seat.as_object_mut() {
                o.retain(|k, _| PUBLIC_SEAT_KEYS.contains(&k.as_str()));
            }
        }
    }
    Value::Object(out)
}

/// Every bot's table as the public gets it.
fn bots(s: &Shared) -> Value {
    Value::Array(s.bots.iter().map(|b| public_table(&state::fleet_table_json(s, &b.read()))).collect())
}

async fn tv_state(State(s): State<Arc<Shared>>) -> Response {
    off_runtime(move || Json(json!({"public": true, "bots": bots(&s)}))).await.into_response()
}

/// The public stream: the same per-table events the dashboard gets, reduced to the public keys. No
/// `state` event — the dashboard's snapshot carries the metrics, the logs and the training state, and
/// none of it belongs on this listener.
async fn tv_events(State(s): State<Arc<Shared>>) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    struct St {
        s: Arc<Shared>,
        rx: broadcast::Receiver<String>,
        dirty: BTreeSet<usize>,
        sent: HashMap<usize, Instant>,
    }
    let rx = s.events.subscribe();
    let init = St { s, rx, dirty: BTreeSet::new(), sent: HashMap::new() };
    let stream = futures_util::stream::unfold(init, |mut st| async move {
        loop {
            let now = Instant::now();
            let ready =
                st.dirty.iter().copied().find(|slot| st.sent.get(slot).is_none_or(|t| now.duration_since(*t) >= state::TABLE_EVERY));
            if let Some(slot) = ready {
                st.dirty.remove(&slot);
                st.sent.insert(slot, now);
                let shared = st.s.clone();
                let data = off_runtime(move || {
                    shared
                        .bots
                        .get(slot)
                        .map(|b| json!({"slot": slot, "bot": public_table(&state::fleet_table_json(&shared, &b.read()))}).to_string())
                })
                .await;
                match data {
                    Ok(Some(d)) => return Some((Ok(Event::default().event("table").data(d)), st)),
                    _ => continue,
                }
            }
            // A dirty table is due within TABLE_EVERY; an idle one can wait for the keep-alive.
            let wait = if st.dirty.is_empty() { Duration::from_secs(15) } else { state::TABLE_EVERY };
            match tokio::time::timeout(wait, st.rx.recv()).await {
                Ok(Ok(msg)) => {
                    if let Some(slot) = serde_json::from_str::<Value>(&msg).ok().and_then(|v| v["slot"].as_u64()) {
                        st.dirty.insert(slot as usize);
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => st.dirty.extend(0..st.s.bots.len()),
                Ok(Err(_)) => return None,
                Err(_) => {}
            }
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

async fn not_found() -> Response {
    (StatusCode::NOT_FOUND, Json(json!({"detail": "not on the public TV; the dashboard is elsewhere"}))).into_response()
}

/// Headers for the TV listener. The dashboard refuses framing (`X-Frame-Options: DENY`); a public
/// table view is meant to be embedded, so this one allows it and says so in the modern header. The
/// cache and sniffing rules are the dashboard's, unchanged.
async fn headers_layer(req: Request, next: Next) -> Response {
    let immutable = req.uri().path().starts_with("/assets/");
    let value = if immutable { "public, max-age=31536000, immutable" } else { "no-cache" };
    let mut res = next.run(req).await;
    let headers = res.headers_mut();
    headers.entry(header::CACHE_CONTROL).or_insert(HeaderValue::from_static(value));
    headers.entry(header::X_CONTENT_TYPE_OPTIONS).or_insert(HeaderValue::from_static("nosniff"));
    headers.entry(header::CONTENT_SECURITY_POLICY).or_insert(HeaderValue::from_static("frame-ancestors *"));
    headers.entry(header::REFERRER_POLICY).or_insert(HeaderValue::from_static("same-origin"));
    res
}

/// The public listener's whole surface: the table view, its stream, a health answer, the built page
/// and its assets. The catch-all is what makes "public only" a property of the router rather than a
/// promise about the routes above it.
pub(super) fn router(shared: Arc<Shared>, dist: &Path) -> Router {
    Router::new()
        .route(
            "/api/health",
            get(|| async { Json(json!({"ok": true, "public": true, "version": crate::VERSION, "commit": crate::BUILD_COMMIT})) }),
        )
        .route("/api/tv", get(tv_state))
        .route("/api/tv/events", get(tv_events))
        .route("/api/{*rest}", any(not_found))
        .fallback_service(ServeDir::new(dist).not_found_service(ServeFile::new(dist.join("index.html"))))
        .layer(middleware::from_fn(headers_layer))
        .with_state(shared)
}

#[cfg(test)]
mod tests;

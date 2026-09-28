//! Dashboard API: implements the contract the React control room consumes
//! (`web/src/types.ts`), backed by live bot state and the SQLite store.

use crate::live::{BotLive, Shared};
use anyhow::Result;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use sv10_core::cards::{Card, mask_of};
use sv10_core::eval::{CATEGORY_NAMES, category, eval};
use sv10_core::model::{Counter, HandSummary, PlayerStats};
use tower_http::services::{ServeDir, ServeFile};

mod control;
mod games;
mod hands;
mod insights;
mod intel;
mod monitor;
mod opponents;
mod players;
mod releases;
mod setup;
mod state;
mod timeline;
mod tv;
mod wiring;

use control::*;
use games::*;
use hands::*;
use insights::*;
use intel::*;
use monitor::*;
use opponents::*;
use players::*;
use releases::checkout::*;
pub use releases::checkout::{UpdateCheck, run_update_check};
use releases::*;
use setup::*;
use state::*;
use timeline::*;
use wiring::*;

pub const POLICY_VERSION: &str = "sv10-ev-1";

/// Run a handler that reads the store or builds a large snapshot on the blocking pool, so dashboard
/// requests never stall the async workers that carry the bots' table connections. A panicking
/// handler answers 500 instead of dropping the connection.
async fn off_runtime<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f).await.map_err(|e| server_error("dashboard handler failed", e))
}

/// The host check for the System view (`GET /api/host`, 0241): read fresh on each request (a few
/// small `/proc` and `/sys` files and two free-space calls), judged by [`crate::hostcheck::checks`].
async fn host(State(s): State<Arc<Shared>>) -> Result<Json<Value>, ApiError> {
    let (artifacts, archive) = (s.config.artifacts.clone(), s.config.archive_dir.clone());
    let checks = off_runtime(move || crate::hostcheck::checks(&crate::hostcheck::read(&artifacts, &archive))).await?;
    Ok(Json(json!({"checks": checks, "checked_at": chrono::Utc::now().timestamp()})))
}

/// A request the server answers with an error response (boxed: a `Response` is large).
pub(super) struct ApiError(Box<Response>);

impl From<Response> for ApiError {
    fn from(r: Response) -> Self {
        ApiError(Box::new(r))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        *self.0
    }
}

/// A request the server could not answer: logged, and answered 500 with the reason, so a panel
/// shows "unavailable" instead of an empty result (LESSONS 24).
fn server_error(what: &str, e: impl std::fmt::Display) -> ApiError {
    tracing::warn!("{what}: {e}");
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": format!("{what}: {e}")}))).into_response().into()
}

/// A store read a handler needs: its error answers 500 (0222).
fn store_read<T>(what: &str, r: Result<T>) -> Result<T, ApiError> {
    r.map_err(|e| server_error(&format!("store unreadable ({what})"), e))
}

/// A store read that feeds one figure of a larger snapshot: the snapshot still goes out, and the
/// failure is logged at most once a minute so a store outage shows in the log, not as silence.
fn snapshot_read<T: Default>(what: &str, r: Result<T>) -> T {
    r.unwrap_or_else(|e| {
        snapshot_warn(&format!("store unreadable ({what}), dashboard figure left empty: {e}"));
        T::default()
    })
}

/// Report a store failure behind a dashboard figure, at most once a minute process-wide: a broken
/// store answers every panel on every poll, and the log must show the outage without drowning in it.
fn snapshot_warn(message: &str) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST_WARN: AtomicU64 = AtomicU64::new(0);
    let now = now_secs() as u64;
    if now.saturating_sub(LAST_WARN.load(Ordering::Relaxed)) >= 60 {
        LAST_WARN.store(now, Ordering::Relaxed);
        tracing::warn!("{message}");
    }
}

fn now_secs() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs_f64()
}

fn parse_ts(s: &str) -> f64 {
    crate::season::ended_at_secs(s)
}

/// The scope a season panel actually used, so the dashboard can label it instead of implying a
/// season it did not read. `scoped: false` means the season boundary was unknown and the figures
/// cover every stored hand.
fn season_scope(season: Option<&crate::season::CurrentSeason>) -> Value {
    match season {
        Some(s) => json!({"scoped": true, "number": s.number, "id": s.id, "started_at": s.started_at}),
        None => json!({"scoped": false, "number": null, "id": null, "started_at": null}),
    }
}

/// Keep only the hands of the current season. With no known boundary nothing is dropped; the
/// panel reports `scoped: false` rather than passing all-time figures off as this season's.
fn this_season<T>(season: Option<&crate::season::CurrentSeason>, rows: Vec<T>, ended_at: impl Fn(&T) -> f64) -> Vec<T> {
    match season {
        Some(season) => rows.into_iter().filter(|r| season.contains(ended_at(r))).collect(),
        None => rows,
    }
}

fn split_cards(s: &str) -> Vec<String> {
    s.as_bytes().chunks(2).filter(|c| c.len() == 2).map(|c| String::from_utf8_lossy(c).to_string()).collect()
}

fn session_cookie(token: &str) -> String {
    sv10_digest::hex(sv10_digest::Sha256::digest(format!("svanbot10:{token}").as_bytes()))
}

fn authorized(s: &Shared, headers: &HeaderMap) -> bool {
    let Some(token) = &s.config.operator_token else { return true };
    let want = format!("svan_session={}", session_cookie(token));
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|c| c.split(';').any(|part| sv10_digest::constant_time_eq(part.trim().as_bytes(), want.as_bytes())))
}

/// Hashed build assets never change; everything else (the page, the API) is revalidated so a new
/// dashboard build shows up on a plain reload. Every response also refuses framing and MIME
/// sniffing and keeps the referrer on this origin.
async fn cache_layer(req: Request, next: Next) -> Response {
    let immutable = req.uri().path().starts_with("/assets/");
    let mut res = next.run(req).await;
    let value = if immutable { "public, max-age=31536000, immutable" } else { "no-cache" };
    let headers = res.headers_mut();
    headers.entry(header::CACHE_CONTROL).or_insert(HeaderValue::from_static(value));
    headers.entry(header::X_CONTENT_TYPE_OPTIONS).or_insert(HeaderValue::from_static("nosniff"));
    headers.entry(header::X_FRAME_OPTIONS).or_insert(HeaderValue::from_static("DENY"));
    headers.entry(header::REFERRER_POLICY).or_insert(HeaderValue::from_static("same-origin"));
    res
}

/// The host name a request was addressed to (the `Host` header without its port).
fn request_host(headers: &HeaderMap) -> &str {
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(""),
        None => host.split(':').next().unwrap_or(""),
    }
}

/// Without an operator token, a request that changes anything (every POST: bot commands, training,
/// releases, compute profile, setup) must be addressed to a loopback host, so a hostile page that
/// rebinds its own domain to 127.0.0.1 cannot drive the fleet (0222; setup had this since 0085).
fn rebinding_refused(s: &Shared, method: &axum::http::Method, headers: &HeaderMap) -> bool {
    s.config.operator_token.is_none() && method != axum::http::Method::GET && !crate::setup::loopback(request_host(headers))
}

async fn auth_layer(State(s): State<Arc<Shared>>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if path.starts_with("/api/") && path != "/api/session" && path != "/api/health" && !authorized(&s, req.headers()) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"detail": "Operator token required"}))).into_response();
    }
    if path.starts_with("/api/") && rebinding_refused(&s, req.method(), req.headers()) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"detail": "Without an operator token, changes are accepted only at http://127.0.0.1; set SVANBOT_WEB__OPERATOR_TOKEN to allow other hosts"})),
        )
            .into_response();
    }
    next.run(req).await
}

pub async fn serve(shared: Arc<Shared>) -> Result<()> {
    let dist = shared.config.web_dist.clone();
    let app = Router::new()
        // `public` says which listener answered: false here, true on the TV (`tv::router`). The
        // dashboard branches on it before it asks for a session, so a spectator on the public
        // listener never sees a login form, and an operator never gets the table view by accident.
        .route(
            "/api/health",
            get(|| async { Json(json!({"ok": true, "public": false, "version": crate::VERSION, "commit": crate::BUILD_COMMIT})) }),
        )
        .route("/api/session", post(session))
        .route("/api/state", get(state))
        .route("/api/events", get(events))
        .route("/api/raw", get(raw_state))
        .route("/api/bots/{slot}/hands", get(hands))
        .route("/api/bots/{slot}/hands/{hand_id}", get(replay))
        .route("/api/bots/{slot}/opponents", get(opponents))
        .route("/api/bots/{slot}/opponents/{name}", get(opponent_detail))
        .route("/api/players/{name}/card", get(player_card))
        .route("/api/accuracy", get(decision_accuracy))
        .route("/api/quiz", get(quiz))
        .route("/api/bots/{slot}/command", post(bot_command))
        .route("/api/training/command", post(training_command))
        .route("/api/training/settings", post(training_settings))
        .route("/api/starting-hands", get(starting_hands))
        .route("/api/leaderboard", get(leaderboard))
        .route("/api/experiment", get(|State(s): State<Arc<Shared>>| async move { Json(crate::experiment::view(&s, now_secs())) }))
        .route("/api/fleet", get(fleet))
        .route("/api/highlights", get(highlights))
        .route("/api/stories", get(stories))
        .route("/api/timeline", get(timeline))
        .route("/api/timeline/hour/{hour}", get(timeline_hour))
        .route("/api/calibration", get(calibration))
        .route("/api/compute", get(compute))
        .route("/api/rivals", get(rivals))
        .route("/api/intel", get(intel))
        .route("/api/badges", get(badges))
        .route("/api/compute/profile", post(compute_profile))
        .route("/api/analysis", get(analysis))
        .route("/api/hand-classes", get(|| async { Json(json!((0..169).map(sv10_core::range::class_name).collect::<Vec<_>>())) }))
        .route("/api/bots/{slot}/ranges", get(ranges))
        .route("/api/monitor", get(monitor))
        .route("/api/host", get(host))
        .route("/api/wiring", get(wiring))
        .route("/api/releases", get(releases))
        .route("/api/releases/log", get(release_log))
        .route("/api/releases/update", post(trigger_update))
        .route("/api/releases/check", post(check_updates))
        .route("/api/releases/progress", get(release_progress))
        .route("/api/releases/snapshots", get(release_snapshots))
        .route("/api/releases/rollback", post(trigger_rollback))
        .route("/api/setup", get(get_setup).post(save_setup))
        .route("/api/setup/verify-key", post(verify_key))
        .fallback_service(ServeDir::new(&dist).not_found_service(ServeFile::new(dist.join("index.html"))))
        .layer(middleware::from_fn(cache_layer))
        .layer(middleware::from_fn_with_state(shared.clone(), auth_layer))
        .with_state(shared.clone());
    let addr = format!("{}:{}", shared.config.web_host, shared.config.web_port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("dashboard on http://{addr}");
    // The public TV (`SVANBOT_TV_PORT`, 0 = off): a second listener carrying the table view and
    // nothing else, for an audience with no operator token (`tv`). Binding it beyond loopback is
    // what publishes it. A failure to bind is logged, not fatal: the optional public surface must
    // never take the control room down with it, and the log line is where an operator sees why.
    if shared.config.tv_port != 0 {
        let tv_addr = format!("{}:{}", shared.config.tv_host, shared.config.tv_port);
        match tokio::net::TcpListener::bind(&tv_addr).await {
            Ok(tv_listener) => {
                tracing::info!("public TV on http://{tv_addr} (unauthenticated: table view only)");
                let tv = tv::router(shared.clone(), &dist);
                tokio::spawn(async move {
                    if let Err(e) = axum::serve(tv_listener, tv).await {
                        tracing::error!("public TV stopped: {e:#}");
                    }
                });
            }
            Err(e) => tracing::error!("public TV not started on {tv_addr}: {e}"),
        }
    }
    axum::serve(listener, app).await?;
    Ok(())
}

async fn session(State(s): State<Arc<Shared>>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let Some(token) = &s.config.operator_token else {
        return Json(json!({"ok": true})).into_response();
    };
    let given = body["token"].as_str().unwrap_or("");
    let same = sv10_digest::constant_time_eq(given.as_bytes(), token.as_bytes());
    if authorized(&s, &headers) || (!given.is_empty() && same) {
        let cookie = format!("svan_session={}; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000", session_cookie(token));
        let mut resp = Json(json!({"ok": true})).into_response();
        // The cookie is built from a hex digest, so it is always a valid header value.
        if let Ok(v) = HeaderValue::from_str(&cookie) {
            resp.headers_mut().insert(header::SET_COOKIE, v);
        }
        return resp;
    }
    (StatusCode::UNAUTHORIZED, Json(json!({"detail": "Operator token required"}))).into_response()
}

#[cfg(test)]
mod security_tests {
    use super::*;
    use axum::http::Method;

    fn headers(host: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, HeaderValue::from_str(host).unwrap());
        h
    }

    #[test]
    fn changes_without_a_token_need_a_loopback_host() {
        let s = Shared::for_test("api-rebinding", &["A"]);
        assert!(s.config.operator_token.is_none());
        for host in ["127.0.0.1:8787", "localhost:8787", "[::1]:8787"] {
            assert!(!rebinding_refused(&s, &Method::POST, &headers(host)), "{host}");
        }
        assert!(rebinding_refused(&s, &Method::POST, &headers("evil.example:8787")), "rebound domain");
        assert!(rebinding_refused(&s, &Method::POST, &HeaderMap::new()), "no Host header");
        assert!(!rebinding_refused(&s, &Method::GET, &headers("evil.example:8787")), "reads stay open, as before");
    }
}

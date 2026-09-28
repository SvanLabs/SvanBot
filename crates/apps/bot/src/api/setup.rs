//! Bot setup endpoints (0085): read the configured bots without their keys, verify a key against
//! openpoker.ai, and save a new setup to `.env` (see `crate::setup`).

use super::*;
use crate::setup::{SetupInput, SetupView, current, lookup, plan, slot_views};

fn env_text(s: &Shared) -> String {
    std::fs::read_to_string(s.config.root.join(".env")).unwrap_or_default()
}

fn write_blocked(s: &Shared) -> Option<String> {
    (s.config.operator_token.is_none() && !crate::setup::loopback(&s.config.web_host))
        .then(|| "Set SVANBOT_WEB__OPERATOR_TOKEN in .env first: the dashboard is reachable from other machines.".to_string())
}

/// Without an operator token, key-handling requests must name a loopback host, so a hostile page
/// that rebinds its own domain to 127.0.0.1 cannot read or replace keys.
fn host_refused(s: &Shared, headers: &HeaderMap) -> Option<Response> {
    if s.config.operator_token.is_some() {
        return None;
    }
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    let name =
        if let Some(rest) = host.strip_prefix('[') { rest.split(']').next().unwrap_or("") } else { host.split(':').next().unwrap_or("") };
    (!crate::setup::loopback(name)).then(|| {
        (StatusCode::FORBIDDEN, Json(json!({"detail": "Bot setup without an operator token only works from http://127.0.0.1"})))
            .into_response()
    })
}

pub(super) async fn get_setup(State(s): State<Arc<Shared>>) -> Json<SetupView> {
    let text = env_text(&s);
    let (bots, buy_in, seek_top_rank) = current(&lookup(&text, |k| std::env::var(k).ok()));
    let blocked = write_blocked(&s);
    Json(SetupView {
        bots: slot_views(&bots),
        buy_in,
        seek_top_rank,
        max_bots: crate::setup::MAX_BOTS,
        can_write: blocked.is_none(),
        write_blocked: blocked,
        supervised: s.config.supervised,
        restart_pending: s.restart_requested.load(std::sync::atomic::Ordering::Relaxed),
    })
}

/// Check a key (pasted, or the stored key of `slot`) with `GET /me` and `GET /season/me`; returns the
/// bot's registered name and Pro status only.
pub(super) async fn verify_key(State(s): State<Arc<Shared>>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    if let Some(refused) = host_refused(&s, &headers) {
        return refused;
    }
    let key = match (body["key"].as_str().map(str::trim).filter(|k| !k.is_empty()), body["slot"].as_u64()) {
        (Some(k), _) => k.to_string(),
        (None, Some(slot)) => {
            let text = env_text(&s);
            let (bots, _, _) = current(&lookup(&text, |k| std::env::var(k).ok()));
            match bots.get(slot as usize) {
                Some(b) => b.key.clone(),
                None => return (StatusCode::NOT_FOUND, Json(json!({"detail": "Unknown bot slot"}))).into_response(),
            }
        }
        _ => return (StatusCode::BAD_REQUEST, Json(json!({"detail": "Paste a key to check"}))).into_response(),
    };
    if !sv10_rt::env_value_is_safe(&key) {
        return (StatusCode::BAD_REQUEST, Json(json!({"detail": "That does not look like an API key"}))).into_response();
    }
    let Ok(http) = reqwest::Client::builder().timeout(Duration::from_secs(10)).build() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": "HTTP client unavailable"}))).into_response();
    };
    let get = |path: &str| http.get(format!("{}{path}", s.config.rest_base)).bearer_auth(&key).send();
    match get("/me").await {
        Ok(r) if r.status().is_success() => {
            let me: Value = r.json().await.unwrap_or(Value::Null);
            let pro = match get("/season/me").await {
                Ok(r) if r.status().is_success() => r.json::<Value>().await.ok().and_then(|v| v["pro_tier"].as_bool()),
                _ => None,
            };
            // Never pass on the account email or other profile fields.
            Json(json!({"ok": true, "name": me["name"].as_str(), "pro_tier": pro})).into_response()
        }
        Ok(r) if r.status() == StatusCode::UNAUTHORIZED || r.status() == StatusCode::FORBIDDEN => {
            (StatusCode::BAD_REQUEST, Json(json!({"detail": "openpoker.ai rejected this key"}))).into_response()
        }
        Ok(r) => (StatusCode::BAD_GATEWAY, Json(json!({"detail": format!("openpoker.ai answered {}", r.status())}))).into_response(),
        Err(_) => {
            (StatusCode::BAD_GATEWAY, Json(json!({"detail": "openpoker.ai is unreachable; the key was not checked"}))).into_response()
        }
    }
}

pub(super) async fn save_setup(State(s): State<Arc<Shared>>, headers: HeaderMap, Json(input): Json<SetupInput>) -> Response {
    if let Some(refused) = host_refused(&s, &headers) {
        return refused;
    }
    if let Some(reason) = write_blocked(&s) {
        return (StatusCode::FORBIDDEN, Json(json!({"detail": reason}))).into_response();
    }
    let path = s.config.root.join(".env");
    let text = env_text(&s);
    let (existing, _, _) = current(&lookup(&text, |k| std::env::var(k).ok()));
    let updates = match plan(&input, &existing) {
        Ok(u) => u,
        Err(reason) => return (StatusCode::BAD_REQUEST, Json(json!({"detail": reason}))).into_response(),
    };
    let refs: Vec<(&str, Option<&str>)> = updates.iter().map(|(k, v)| (k.as_str(), v.as_deref())).collect();
    let updated = sv10_rt::update_env_text(&text, &refs);
    if !text.is_empty() {
        // Unchecked on purpose: if the directory cannot be made, the write below fails and reports it (issue #326).
        let _ = std::fs::create_dir_all(&s.config.artifacts);
        if let Err(e) = sv10_rt::write_private_atomic(&s.config.artifacts.join(".env.previous"), &text) {
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": format!("Could not keep the previous .env: {e}")})))
                .into_response();
        }
    }
    if let Err(e) = sv10_rt::write_private_atomic(&path, &updated) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": format!("Could not write .env: {e}")}))).into_response();
    }
    let names: Vec<String> = input.bots.iter().map(|b| format!("{}{}", b.name.trim(), if b.enabled { "" } else { " (off)" })).collect();
    s.log(
        "fleet",
        "info",
        format!("bot setup saved from the dashboard: {}; buy-in {}, seek top {}", names.join(", "), input.buy_in, input.seek_top_rank),
    );
    let restart = if s.config.supervised {
        s.restart_requested.store(true, std::sync::atomic::Ordering::Relaxed);
        "scheduled"
    } else {
        "manual"
    };
    Json(json!({"ok": true, "restart": restart})).into_response()
}

//! Operator controls: bot and learner commands, the starting-hand guide.

use super::*;

#[cfg(test)]
mod tests;

pub(super) async fn bot_command(State(s): State<Arc<Shared>>, Path(slot): Path<usize>, Json(body): Json<Value>) -> Response {
    let Some(b) = s.bots.get(slot) else {
        return (StatusCode::NOT_FOUND, Json(json!({"detail": "Unknown bot slot"}))).into_response();
    };
    let desired = match body["command"].as_str().unwrap_or("") {
        "start" => "run",
        "pause" => "pause",
        "stop" => "stop",
        other => return (StatusCode::BAD_REQUEST, Json(json!({"detail": format!("Unknown command {other}")}))).into_response(),
    };
    let name = b.read().name.clone();
    // A head controls workers only through this key. A single-process fleet can still apply the
    // command locally if persistence fails, but a head must not acknowledge an undelivered command.
    if let Err(e) = s.store.put_kv(&crate::live::want_key(&name), desired) {
        if s.config.head {
            s.log(&name, "error", format!("operator command not delivered to worker: {e}"));
            return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"detail": format!("operator command not delivered to worker: {e}")})))
                .into_response();
        }
        s.log(&name, "warn", format!("operator command not persisted for split-fleet workers: {e}"));
    }
    crate::live::apply_desired(&mut b.write(), desired);
    s.log(&name, "info", format!("operator command: {}", body["command"].as_str().unwrap_or("")));
    Json(json!({"ok": true})).into_response()
}

pub(super) async fn training_command(State(s): State<Arc<Shared>>, Json(body): Json<Value>) -> Response {
    let command = body["command"].as_str().unwrap_or("").to_string();
    if !crate::pacing::learner_acts_on(&command) {
        let detail = format!("the learner does not act on `{command}`; it accepts `start`");
        return (StatusCode::BAD_REQUEST, Json(json!({"detail": detail}))).into_response();
    }
    // The learner is a separate process that only sees the command through the store, so a failed
    // write must fail the request instead of reporting a queued command that never arrives.
    if let Err(e) = s.store.put_kv(crate::LEARNER_COMMAND_KEY, &json!({"command": command, "body": body, "ts": now_secs()}).to_string()) {
        s.log("learner", "error", format!("training command {command} not queued: {e}"));
        return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"detail": format!("training command not queued: {e}")}))).into_response();
    }
    s.log("learner", "info", format!("training command queued: {command}"));
    Json(json!({"ok": true})).into_response()
}

/// Operator learner settings: `{"cooldown_minutes": 0..=1440, "min_new_hands": 0..=20000}`, either or both
/// (minutes to let a search cool down; live hands after which a search starts at once). Evidence
/// refreshes are not set here: they run on the hands as they arrive (#314).
pub(super) async fn training_settings(State(s): State<Arc<Shared>>, Json(body): Json<Value>) -> Response {
    let stored = crate::pacing::LearnerSettings::parse(s.store.get_kv(crate::pacing::SETTINGS_KEY).ok().flatten().as_deref());
    let settings = match stored.update(&body) {
        Ok(v) => v,
        Err(detail) => return (StatusCode::BAD_REQUEST, Json(json!({ "detail": detail }))).into_response(),
    };
    if let Err(e) = s.store.put_kv(crate::pacing::SETTINGS_KEY, &serde_json::to_string(&settings).unwrap_or_default()) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": format!("could not save the setting: {e}")}))).into_response();
    }
    let effective = crate::pacing::Pacing::from_env().with_settings(&settings);
    let (minutes, hands) = (effective.cooldown_secs / 60.0, effective.min_new_hands);
    s.log("learner", "info", format!("learner settings: cooldown {minutes} min, {hands} new hands start a champion search"));
    Json(json!({"ok": true, "cooldown_minutes": minutes, "min_new_hands": hands})).into_response()
}

/// Operator compute profile (0187): `{"name": "quiet" | "balanced" | "max"}` or a checked custom profile.
/// The live bots apply it on the watcher's next poll, the learner and analyst between runs.
pub(super) async fn compute_profile(State(s): State<Arc<Shared>>, Json(body): Json<Value>) -> Response {
    let hardware = s.store.get_kv(crate::HARDWARE_PROFILE_KEY).ok().flatten();
    let logical =
        hardware.as_deref().and_then(|h| serde_json::from_str::<Value>(h).ok()).and_then(|h| h["logical_cores"].as_u64()).unwrap_or(1)
            as usize;
    let profile = match crate::profile::ComputeProfile::from_request(&body, logical) {
        Ok(p) => p,
        Err(detail) => return (StatusCode::BAD_REQUEST, Json(json!({ "detail": detail }))).into_response(),
    };
    if let Err(e) = s.store.put_kv(crate::profile::PROFILE_KEY, &serde_json::to_string(&profile).unwrap_or_default()) {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"detail": format!("could not save the profile: {e}")}))).into_response();
    }
    s.log(
        "fleet",
        "info",
        format!(
            "compute profile {}: live budget x{}, learner {} threads, analyst {} threads",
            profile.name, profile.live_scale, profile.learner_threads, profile.analyst_threads
        ),
    );
    Json(compute_profile_json(&s, logical)).into_response()
}

pub(super) async fn starting_hands(State(s): State<Arc<Shared>>) -> Json<Value> {
    if let Some(v) = crate::guide::cached() {
        return Json(v);
    }
    let v = tokio::task::spawn_blocking(move || {
        let models = s.models.read().clone();
        let params = s.params.read().clone();
        let nn = s.nn.read().clone();
        crate::guide::refresh(&models, &params, nn.as_deref());
        crate::guide::cached().unwrap_or_else(|| json!([]))
    })
    .await
    .unwrap_or_else(|_| json!([]));
    Json(v)
}

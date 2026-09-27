//! The split-fleet seam (0128): the keys one process publishes its bots' state under, the keys it
//! reads the operator's desired mode back from, and the lineage head the dashboard shows (0320: split
//! out of `live.rs`; the unit tests stayed there, in `live/tests.rs`).

use super::*;

/// Latest entry of the learner's promotion lineage, if recorded.
pub fn lineage_head(store: &Store) -> Option<String> {
    let lineage: Vec<String> = serde_json::from_str(&store.get_kv(crate::LEARNER_LINEAGE_KEY).ok()??).ok()?;
    lineage.last().cloned()
}

/// kv key carrying one worker's [`BotLive`] snapshot in split-fleet mode (0128).
pub fn heartbeat_key(name: &str) -> String {
    format!("bot.live.{name}")
}

/// kv key carrying the operator's desired mode for a worker bot (`run`/`pause`/`stop`).
pub fn want_key(name: &str) -> String {
    format!("bot.want.{name}")
}

/// A heartbeat is fresh enough to stand in for the worker on the dashboard.
pub const HEARTBEAT_STALE_SECS: f64 = 30.0;

/// Wrap a bot's live state with its send time; identical payloads are skipped by the store,
/// so an idle bot costs no SSD writes.
pub fn wrap_heartbeat(b: &BotLive, now_secs: f64) -> serde_json::Value {
    // `commit`: the worker's build, so the dashboard's update progress can confirm its swap (0236).
    serde_json::json!({"hb": now_secs, "state": b, "commit": crate::BUILD_COMMIT})
}

/// The build commit a worker's heartbeat reports (heartbeats before 0236 carry none).
pub fn heartbeat_commit(v: &serde_json::Value) -> Option<String> {
    v.get("commit")?.as_str().map(String::from)
}

/// Split a heartbeat back into live state and its age; `None` on any shape or clock problem.
/// Staleness is judged by the reader, not here.
pub fn unwrap_heartbeat(v: &serde_json::Value, now_secs: f64) -> Option<(BotLive, f64)> {
    let hb = v.get("hb")?.as_f64()?;
    if !hb.is_finite() || hb > now_secs + 5.0 {
        return None;
    }
    let state = serde_json::from_value(v.get("state")?.clone()).ok()?;
    Some((state, now_secs - hb))
}

/// Head side of the split-fleet liveness seam: a worker's freshest heartbeat, when it may stand
/// in for an offline local slot. One home for the whole read contract — key format, payload
/// shape, clock rules and the staleness threshold — so the dashboard never re-implements it.
pub fn read_remote_bot(store: &Store, name: &str, now_secs: f64) -> Option<(BotLive, f64)> {
    let v: serde_json::Value = serde_json::from_str(&store.get_kv(&heartbeat_key(name)).ok()??).ok()?;
    let (b, age) = unwrap_heartbeat(&v, now_secs)?;
    (age <= HEARTBEAT_STALE_SECS).then_some((b, age))
}

/// Apply an operator desired-mode command to one bot (dashboard and worker want-poll share this).
pub fn apply_desired(b: &mut BotLive, desired: &str) {
    b.desired = desired.to_string();
    if desired == "run" && (b.mode == "error" || b.mode == "stopped" || b.mode == "paused") {
        b.mode = "connecting".into();
    }
}

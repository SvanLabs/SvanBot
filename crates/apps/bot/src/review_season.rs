//! `review season-snapshot` / `season-compare`: season-carryover checks (0257).

use anyhow::Result;
use sv10_core::model::ModelStore;
use sv10_store::store::Store;

pub fn season_snapshot(store: &Store, root: &std::path::Path) -> Result<serde_json::Value> {
    use serde_json::json;
    let models: ModelStore = store.get_kv(crate::MODELS_KEY)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let mut top: Vec<(&String, f32)> = models.players.iter().map(|(n, p)| (n, p.hands)).collect();
    top.sort_by(|a, b| b.1.total_cmp(&a.1));
    let top: serde_json::Map<String, serde_json::Value> = top.into_iter().take(50).map(|(n, h)| (n.clone(), json!(h))).collect();
    let lineage: Vec<String> = store.get_kv(crate::LEARNER_LINEAGE_KEY)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let kv_field = |key: &str, field: &str| -> serde_json::Value {
        store
            .get_kv(key)
            .ok()
            .flatten()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .map(|v| v[field].clone())
            .unwrap_or(serde_json::Value::Null)
    };
    let mut hands = serde_json::Map::new();
    for bot in store.bot_names()? {
        hands.insert(bot.clone(), json!(store.recent_hands_light(&bot, 10_000_000)?.len()));
    }
    let history = crate::history::HistoryDb::open(&root.join("artifacts").join("history.db")).map(|h| h.count()).unwrap_or(-1);
    Ok(json!({
        "taken_at": chrono::Utc::now().to_rfc3339(),
        "models": {"players": models.players.len(), "total_hands": models.players.values().map(|p| p.hands as f64).sum::<f64>(), "top": top},
        "lineage": lineage,
        "params_digest": store.get_kv(crate::PARAMS_KEY)?.map(|p| { sv10_digest::hex(sv10_digest::Sha256::digest(p.as_bytes())) }),
        "nn": {"active": kv_field(crate::NN_KEY, "active"), "trained_at": kv_field(crate::NN_KEY, "trained_at")},
        "range": {"active": kv_field(crate::RANGE_PARAMS_KEY, "active"), "fitted_at": kv_field(crate::RANGE_PARAMS_KEY, "fitted_at")},
        "hands": hands,
        "history_hands": history,
    }))
}

/// Print the carry-over checklist; returns the number of failures.
pub fn season_compare(a: &serde_json::Value, b: &serde_json::Value) -> usize {
    let mut failures = 0;
    let mut check = |ok: bool, what: String| {
        println!("{} {what}", if ok { "PASS" } else { "FAIL" });
        failures += usize::from(!ok);
    };
    let n = |v: &serde_json::Value| v.as_f64().unwrap_or(0.0);
    check(
        n(&b["models"]["players"]) >= n(&a["models"]["players"]),
        format!("opponent models kept: {} -> {} players", a["models"]["players"], b["models"]["players"]),
    );
    // Imports re-weight by age, so allow a small drift; a reset would lose nearly everything.
    let (ta, tb) = (n(&a["models"]["total_hands"]), n(&b["models"]["total_hands"]));
    check(tb >= ta * 0.8, format!("observed hands kept (age weighting allows -20%): {ta:.0} -> {tb:.0}"));
    let lost: Vec<&String> = a["models"]["top"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(name, h)| n(&b["models"]["top"][name.as_str()]) < n(h) * 0.8 && b["models"]["top"].get(name.as_str()).is_some())
        .map(|(name, _)| name)
        .collect();
    check(lost.is_empty(), format!("most-observed opponents keep their hands (lost: {lost:?})"));
    let (la, lb) = (a["lineage"].as_array().cloned().unwrap_or_default(), b["lineage"].as_array().cloned().unwrap_or_default());
    check(
        lb.len() >= la.len() && lb[..la.len()] == la[..],
        format!("champion lineage unchanged or extended: {:?} -> {:?}", la.last(), lb.last()),
    );
    if lb.len() == la.len() {
        check(a["params_digest"] == b["params_digest"], "live parameters unchanged without a promotion".into());
    }
    check(
        b["nn"]["active"] == a["nn"]["active"] || n(&b["nn"]["trained_at"]) > n(&a["nn"]["trained_at"]),
        format!("response model state kept: active {} -> {}", a["nn"]["active"], b["nn"]["active"]),
    );
    check(
        b["range"]["active"] == a["range"]["active"] || n(&b["range"]["fitted_at"]) > n(&a["range"]["fitted_at"]),
        format!("range model state kept: active {} -> {}", a["range"]["active"], b["range"]["active"]),
    );
    for (bot, h) in a["hands"].as_object().into_iter().flatten() {
        check(n(&b["hands"][bot.as_str()]) >= n(h), format!("{bot}: stored hands not lost ({} -> {})", h, b["hands"][bot.as_str()]));
    }
    check(
        n(&b["history_hands"]) >= n(&a["history_hands"]),
        format!("history.db hands not lost ({} -> {})", a["history_hands"], b["history_hands"]),
    );
    failures
}

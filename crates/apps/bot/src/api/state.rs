//! Live state: per-bot snapshots, training status, the realtime event stream.

use super::*;

pub(super) struct MetricsCache {
    at: Instant,
    /// The figures the last successful store read produced. A failed read never overwrites them:
    /// zeros served in their place made a bot at a table look like one that had never played (0326).
    value: Value,
    /// The newest failed read behind those figures, kept until a read succeeds.
    error: Option<String>,
}

pub(super) fn metrics_cache() -> &'static parking_lot::Mutex<HashMap<usize, MetricsCache>> {
    static C: OnceLock<parking_lot::Mutex<HashMap<usize, MetricsCache>>> = OnceLock::new();
    C.get_or_init(|| parking_lot::Mutex::new(HashMap::new()))
}

/// The learner settings in effect (`.env` defaults with the dashboard's overrides) and their limits.
fn learner_settings(store: &sv10_store::store::Store) -> Value {
    let p = crate::pacing::Pacing::from_env()
        .with_settings(&crate::pacing::LearnerSettings::parse(store.get_kv(crate::pacing::SETTINGS_KEY).ok().flatten().as_deref()));
    json!({"cooldown_minutes": p.cooldown_secs / 60.0, "max_cooldown_minutes": crate::pacing::MAX_COOLDOWN_MINUTES,
        "min_new_hands": p.min_new_hands, "max_min_new_hands": crate::pacing::MAX_MIN_NEW_HANDS})
}

/// The knobs the learner searches, as the Champion profile needs them (#322): the bounds its bar is
/// drawn against, the shipped default the bar marks as its reference, and the sentence under it.
/// `crate::knobs` is the one definition — the panel used to carry a hand-typed copy that no change
/// to the search ever reached.
fn knob_catalogue() -> Vec<Value> {
    let shipped = sv10_core::policy::Params::default();
    crate::knobs::KNOBS
        .iter()
        .map(|k| {
            json!({"key": k.key, "label": k.label, "min": k.min, "max": k.max, "decimals": k.decimals,
                "default": k.get(&shipped), "description": k.description})
        })
        .collect()
}

fn response_model_labels(stored: &crate::StoredNet) -> (&'static str, &'static str, &'static str) {
    if crate::neural::response_net_is_eligible(stored) {
        ("available", "active: opponent response model", "in use")
    } else if stored.active {
        ("rejected", "stored artifact rejected by current live gates", "rejected by live gates")
    } else {
        ("validating", "not yet better than the stat model", "not better yet")
    }
}

pub(super) fn metrics(s: &Shared, b: &BotLive) -> Value {
    if let Some(c) = metrics_cache().lock().get(&b.slot)
        && c.at.elapsed() < Duration::from_secs(5)
    {
        // A cached snapshot is up to five seconds old; the counters are cheap to refresh and are
        // the panel's evidence, so they come from the bot as it is now.
        return served(c.value.clone(), b, c.error.as_deref());
    }
    let bb = if b.big_blind > 0 { b.big_blind as f64 } else { s.big_blind() };
    // A bot's winnings are a season standing, so the headline figures and the curve read this
    // season; the cumulative run stays available as `all_time`.
    let season = s.season();
    let mut all: Vec<ResultRow> = Vec::new();
    let mut error: Option<String> = None;
    for name in s.names_of(&b.name) {
        match s.store.bot_ev_results(&name) {
            Ok(rows) => all.extend(rows),
            // Reported, not defaulted (0326): one failed name read used to publish
            // `{"hands": 0, "net_chips": 0}` for a bot that was at a table playing.
            Err(e) => error = Some(format!("store unreadable (bot results): {e}")),
        }
    }
    let last_good = metrics_cache().lock().get(&b.slot).map(|c| c.value.clone());
    let (value, error) = figures(all, error, &season, bb, last_good);
    if let Some(e) = &error {
        snapshot_warn(&format!("{e}, serving the last good figures (stale)"));
    }
    let value = served(value, b, error.as_deref());
    // The figures are cached; a failed read keeps the previous ones and only advances the clock, so
    // the next request retries the store in five seconds rather than hammering it.
    metrics_cache().lock().insert(b.slot, MetricsCache { at: Instant::now(), value: value.clone(), error });
    value
}

/// The figures one bot's snapshot carries, and the failure behind them (0326). A clean read builds
/// them from `rows`; a failed name read keeps the previous good figures (`last_good` — the
/// leaderboard panel's pattern, so a playing bot never reads as an idle one), or the flagged empty
/// shape when there is no previous read to keep.
fn figures(
    rows: Vec<ResultRow>,
    error: Option<String>,
    season: &Option<crate::season::CurrentSeason>,
    bb: f64,
    last_good: Option<Value>,
) -> (Value, Option<String>) {
    if error.is_some() {
        return (last_good.unwrap_or_else(|| empty_figures(season, bb)), error);
    }
    let mut rows = rows;
    rows.sort_by(|a, b| a.3.cmp(&b.3));
    let all_time = summarize(&rows, bb).0;
    let season_rows = this_season(season.as_ref(), rows, |(_, _, _, ended): &ResultRow| parse_ts(ended));
    let (summary, series) = summarize(&season_rows, bb);
    let mut v = summary;
    v["season"] = season_scope(season.as_ref());
    v["all_time"] = all_time;
    v["series"] = json!(series);
    (v, None)
}

/// The figures a bot's snapshot carries: the stored ones, plus the live counters from the bot as it
/// is now, plus whether the stored figures are the last good reading rather than this second's
/// (0326). `stale` and `error` are always present, so the dashboard can tell "0 hands" from
/// "unreadable" without guessing.
fn served(mut value: Value, b: &BotLive, error: Option<&str>) -> Value {
    value["stale"] = json!(error.is_some());
    value["error"] = json!(error);
    value["p95_ms"] = p95(b);
    value["rejected"] = json!(b.rejections);
    value["state_hash"] = state_hash_json(b);
    value["decisions"] = json!(b.decisions);
    value
}

/// The shape of a snapshot whose store could not be read at all: the fields a panel reads, zeroed
/// and flagged, never presented as a measurement.
fn empty_figures(season: &Option<crate::season::CurrentSeason>, bb: f64) -> Value {
    let mut v = summarize(&[], bb).0;
    v["season"] = season_scope(season.as_ref());
    v["all_time"] = summarize(&[], bb).0;
    v["series"] = json!([]);
    v
}

/// A bot's `state_hash` row (0301): this session's checks, the lifetime tallies that survive a
/// restart, and the newest mismatch — the one to open `review state-hash` for.
pub(super) fn state_hash_json(b: &BotLive) -> Value {
    let totals = &b.state_hash_lifetime;
    json!({
        "ok": b.state_hash_ok, "bad": b.state_hash_bad,
        "lifetime_ok": totals.ok, "lifetime_bad": totals.bad,
        "last_mismatch": totals.last_mismatch.as_ref().map(mismatch_json),
    })
}

fn mismatch_json(m: &crate::live::StateHashMismatch) -> Value {
    json!({"at": m.at, "table": m.table, "verdict": m.verdict, "summary": m.summary})
}

/// Fleet-wide `state_hash` totals (0301): every bot's session and lifetime counters added up, and
/// the newest mismatch in the fleet, tagged with the bot that saw it. A head whose workers hold the
/// bots shows what they last reported in their heartbeats.
fn fleet_state_hash(s: &Shared) -> Value {
    let (mut ok, mut bad, mut lifetime_ok, mut lifetime_bad) = (0u64, 0u64, 0u64, 0u64);
    let mut newest: Option<Value> = None;
    let mut newest_at = f64::NEG_INFINITY;
    for bot in &s.bots {
        let b = bot.read();
        ok += b.state_hash_ok;
        bad += b.state_hash_bad;
        lifetime_ok += b.state_hash_lifetime.ok;
        lifetime_bad += b.state_hash_lifetime.bad;
        if let Some(m) = &b.state_hash_lifetime.last_mismatch
            && m.at > newest_at
        {
            newest_at = m.at;
            let mut row = mismatch_json(m);
            row["bot"] = json!(b.name);
            newest = Some(row);
        }
    }
    json!({"ok": ok, "bad": bad, "lifetime_ok": lifetime_ok, "lifetime_bad": lifetime_bad,
        "last_mismatch": newest.unwrap_or(Value::Null)})
}

/// A stored result: net, all-in EV net (0213), showdown, end time.
type ResultRow = (Option<i64>, Option<f64>, bool, String);

/// Net chips, win rate and the running curve over a set of stored results, with the all-in EV
/// (luck-adjusted) curve, win rate and the luck between the two.
fn summarize(rows: &[ResultRow], bb: f64) -> (Value, Vec<Value>) {
    let (mut n, mut sum, mut sq) = (0f64, 0f64, 0f64);
    let (mut ev_sum, mut ev_sq) = (0f64, 0f64);
    let (mut total, mut showdown, mut other) = (0i64, 0i64, 0i64);
    let mut series = Vec::new();
    let step = (rows.len() / 150).max(1);
    for (i, (net, ev, went, _)) in rows.iter().enumerate() {
        let Some(net) = *net else { continue };
        let ev = ev.unwrap_or(net as f64);
        n += 1.0;
        sum += net as f64;
        sq += (net * net) as f64;
        ev_sum += ev;
        ev_sq += ev * ev;
        total += net;
        if *went {
            showdown += net;
        } else {
            other += net;
        }
        if i % step == 0 || i + 1 == rows.len() {
            series.push(json!({"hand": i + 1, "total": total, "showdown": showdown, "other": other, "ev": ev_sum.round()}));
        }
    }
    let rate = |sum: f64, sq: f64| {
        if n >= 2.0 {
            let mean = sum / n;
            let var = (sq / n - mean * mean).max(0.0);
            (Some(mean / bb * 100.0), Some(1.96 * (var / n).sqrt() / bb * 100.0))
        } else {
            (None, None)
        }
    };
    let (bb100, conf) = rate(sum, sq);
    let (ev_bb100, ev_conf) = rate(ev_sum, ev_sq);
    (
        json!({"hands": rows.len(), "priced_hands": n as i64, "net_chips": total, "bb100": bb100, "confidence": conf,
            "ev_net_chips": ev_sum.round(), "ev_bb100": ev_bb100, "ev_confidence": ev_conf, "luck_chips": (total as f64 - ev_sum).round()}),
        series,
    )
}

pub(super) fn p95(b: &BotLive) -> Value {
    if b.latencies_ms.is_empty() {
        return Value::Null;
    }
    let mut v: Vec<f64> = b.latencies_ms.iter().copied().collect();
    v.sort_by(f64::total_cmp);
    json!(v[((v.len() as f64 * 0.95) as usize).min(v.len() - 1)])
}

/// Guide column for our seat this hand: 0 UTG, 1 HJ, 2 CO, 3 BTN, 4 SB, 5 BB.
pub(super) fn guide_column(b: &BotLive) -> Option<usize> {
    let hero = b.seat?;
    let mut seats: Vec<usize> = b.seats.iter().filter(|x| x.in_hand || x.folded).map(|x| x.seat).collect();
    if !seats.contains(&hero) {
        seats.push(hero);
    }
    seats.sort_unstable();
    let n = seats.len();
    if n < 2 {
        return None;
    }
    // Clockwise offset from the button among seats dealt into this hand.
    let start = seats.iter().position(|&s| s >= b.dealer).unwrap_or(0);
    let offset = (seats.iter().position(|&s| s == hero)? + n - start) % n;
    Some(match (n, offset) {
        (2, 0) => 4,
        (2, _) => 5,
        (_, 0) => 3,
        (_, 1) => 4,
        (_, 2) => 5,
        (_, o) if o == n - 1 => 2,
        (_, o) if o == n - 2 => 1,
        _ => 0,
    })
}

/// Our hand's guide score and the weakest score the live opening guide still plays
/// from our position (score = 1 − preflop strength percentile, as in the library).
pub(super) fn preflop_guide_view(b: &BotLive, hole: &[Card]) -> (Value, Value, Value) {
    let (Some(col), [h0, h1]) = (guide_column(b), hole) else { return (Value::Null, Value::Null, Value::Null) };
    let class = sv10_core::range::hand_class(*h0, *h1);
    let score = 1.0 - sv10_core::preflop::table().percentile[class] as f64;
    let threshold = crate::guide::cached().and_then(|g| {
        g.as_array()?
            .iter()
            .filter(|r| r["open"].get(col).and_then(Value::as_bool) == Some(true))
            .filter_map(|r| r["score"].as_f64())
            .reduce(f64::min)
    });
    let label = ["UTG", "HJ", "CO", "BTN", "SB", "BB"][col];
    (json!(score), threshold.map(|t| json!(t)).unwrap_or(Value::Null), json!(label))
}

pub(super) fn bot_json(s: &Shared, b: &BotLive) -> Value {
    let mut v = table_json(s, b);
    v["metrics"] = metrics(s, b);
    v
}

/// How the opponent models read the player in a seat (0212): style, evidence, the shrunk rates
/// decisions price them with, and their per-opponent response correction when one is installed.
fn seat_read(models: &sv10_core::model::ModelStore, name: &str) -> Value {
    let stats = models.players.get(name).cloned().unwrap_or_default();
    let p = models.profile(name);
    json!({
        "style": style_of(&stats).0,
        "hands": stats.hands.round(),
        "vpip": p.vpip,
        "pfr": p.pfr,
        "three_bet": p.three_bet,
        "fold_vs_bet": p.fold_vs_bet,
        "response_ratio": (p.response_ratio != sv10_core::residual::UNIT_RATIO).then_some(p.response_ratio),
        "fold_offset": (p.fold_logit_offset != 0.0).then_some(p.fold_logit_offset),
        "size_tell": (p.size_tell != 0.0).then_some(p.size_tell),
    })
}

/// A bot's live table without its metrics: what the realtime `table` event carries (0211).
pub(super) fn table_json(s: &Shared, b: &BotLive) -> Value {
    let models = s.models.read();
    let decision = b.last_decision.as_ref().map(|d| {
        let hole: Vec<Card> = d.hole.iter().filter_map(|c| Card::parse(c)).collect();
        let board: Vec<Card> = d.board.iter().filter_map(|c| Card::parse(c)).collect();
        let cat = best_five(&hole, &board);
        let hero_stack = b.seats.iter().find(|x| Some(x.seat) == b.seat).map(|x| x.stack + x.bet).unwrap_or(0);
        let villains: Vec<Value> = b
            .seats
            .iter()
            .filter(|x| Some(x.seat) != b.seat && x.in_hand && !x.folded)
            .map(|x| {
                let st = models.players.get(&x.name).cloned().unwrap_or_default();
                // The shrunk profile is exactly what the decision priced this opponent with.
                let p = models.profile(&x.name);
                json!({
                    "name": x.name,
                    "archetype": style_of(&st).0,
                    "hands": st.hands,
                    "confidence": p.confidence,
                    "vpip": p.vpip,
                    "pfr": p.pfr,
                    "three_bet": p.three_bet,
                    "fold_to_3bet": p.fold_to_3bet,
                    "cbet": p.cbet,
                    "fold_to_cbet": p.fold_to_cbet,
                    "fold_vs_bet": p.fold_vs_bet,
                    "raise_vs_bet": p.raise_vs_bet,
                    "river_bluff": p.river_bluff,
                })
            })
            .collect();
        let eff = b
            .seats
            .iter()
            .filter(|x| Some(x.seat) != b.seat && x.in_hand && !x.folded)
            .map(|x| x.stack + x.bet)
            .max()
            .map(|m| m.min(hero_stack))
            .unwrap_or(hero_stack);
        let spr = if d.pot > 0 { Some(eff as f64 / d.pot as f64) } else { None };
        let samples = s.params.read().samples.max(1);
        // A refused decision has no equity to put a standard error on (#424); the panel reads the
        // whole estimate as unavailable, so both fields are null together.
        let se = d.equity.map(|e| (e * (1.0 - e) / samples as f64).sqrt());
        let (preflop_score, opening_threshold, opening_position) = preflop_guide_view(b, &hole);
        let best = d
            .candidates
            .iter()
            .map(|c| format!("{} {} ev {:.0}", c.action, c.amount.map(|a| a.to_string()).unwrap_or_default(), c.ev))
            .collect::<Vec<_>>()
            .join(" · ");
        json!({
            "action": d.action,
            "amount": d.amount.unwrap_or(0),
            "reason": format!("{} — {}", d.reason, best),
            "source": "ev-policy",
            "equity": {"value": d.equity, "samples": samples, "standard_error": se, "exact": false},
            "pot_odds": d.pot_odds,
            "version": d.version,
            "latency_ms": d.latency_ms,
            "effective_stack": eff as f64 / if b.big_blind > 0 { b.big_blind as f64 } else { s.big_blind() },
            "spr": spr,
            "hand_category": cat.as_ref().map(|c| c.0.clone()),
            "best_five": cat.map(|c| c.1).unwrap_or_default(),
            "opponent_models": villains,
            "candidates": d.candidates,
            "street": d.street,
            "pot": d.pot,
            "to_call": d.to_call,
            "preflop_score": preflop_score,
            "opening_threshold": opening_threshold,
            "opening_position": opening_position,
        })
    });
    let status = match b.mode.as_str() {
        "playing" => format!("playing at table {}", b.table_id.as_deref().map(|t| &t[..8.min(t.len())]).unwrap_or("?")),
        "lobby" => "waiting in the lobby".into(),
        other => other.to_string(),
    };
    let mode = match b.mode.as_str() {
        "lobby" => "connecting",
        m => m,
    };
    let last_actions = &b.last_actions;
    let seats: Vec<Value> = b
        .seats
        .iter()
        .map(|x| {
            let read = (Some(x.seat) != b.seat && !x.name.is_empty()).then(|| seat_read(&models, &x.name));
            json!({"seat": x.seat, "name": x.name, "stack": x.stack, "bet": x.bet, "folded": x.folded, "status": x.status, "last_action": last_actions.get(&x.seat), "avatar_url": x.avatar_url.as_deref().and_then(|a| crate::avatar_url(&s.config.rest_base, a)), "read": read})
        })
        .collect();
    let season: Value = b.season.clone().unwrap_or_else(|| json!({}));
    json!({
        "slot": b.slot,
        "name": b.name,
        "mode": mode,
        "status": status,
        "connected": b.connected,
        "last_error": b.last_error,
        "table_id": b.table_id,
        "hand_id": b.hand_id,
        "board": b.board,
        "hole": b.hole,
        "seats": seats,
        "hero_seat": b.seat,
        "dealer_seat": if b.table_id.is_some() { json!(b.dealer) } else { Value::Null },
        "actor_seat": b.actor_seat,
        "pot": b.pot,
        "big_blind": b.big_blind,
        "street": b.street.clone().unwrap_or_else(|| "idle".into()),
        "decision": decision,
        // The version that decided this hand, once it has a decision; otherwise the live champion.
        "version": b.last_decision.as_ref().filter(|d| Some(&d.hand_id) == b.hand_id.as_ref()).map(|d| d.version.clone()).unwrap_or_else(|| s.champion_version.read().clone()),
        "turn_started": b.turn_started,
        "turn": b.turn_started.is_some(),
        "season": season,
    })
}

pub fn training_json(s: &Shared) -> Value {
    let kv = |k: &str| s.store.get_kv(k).ok().flatten().and_then(|v| serde_json::from_str::<Value>(&v).ok());
    let learner = kv(crate::LEARNER_STATUS_KEY);
    let experiments = kv(crate::LEARNER_EXPERIMENTS_KEY).unwrap_or_else(|| json!([]));
    let lineage = kv(crate::LEARNER_LINEAGE_KEY).unwrap_or_else(|| json!([POLICY_VERSION]));
    let version = lineage.as_array().and_then(|l| l.last()).and_then(|v| v.as_str()).unwrap_or(POLICY_VERSION).to_string();
    let params = s.params.read().clone();
    let models = s.models.read();
    // The knobs the learner searches, each with the bounds it searches within, the label and the
    // sentence the profile prints (#322). The dashboard drew its own copy of this list, so a knob the
    // search gained never appeared and a bound it widened left the panel measuring against the old
    // one; it now renders whatever this says, and the values below are keyed by the same table.
    let mut champion = json!({"version": version, "name": format!("svanbot10 EV policy · {version}"),
        "hero_image": params.hero_image, "exact_strengths": params.exact_strengths, "bet_sizes": params.bet_sizes.len()});
    for k in crate::knobs::KNOBS {
        champion[k.key] = json!(k.get(&params));
    }
    let mut t = json!({
        "status": "idle",
        "automatic": true,
        "can_rollback": false,
        "champion": champion,
        "knobs": knob_catalogue(),
        "progress": {"hands": 0, "target": 10800},
        "experiments": experiments,
        // Why candidates have been dying, over the last day (#317): the experiment list is capped at
        // 40 and holds only rejections, so it can neither count a full day nor say what the search
        // was even offered. `learner::funnel` counts at the death instead of reading the list back.
        "search_funnel": crate::learner::funnel::dashboard(&s.store),
        "lineage": lineage,
        "opponent_profiles_tracked": models.players.len(),
        "neural_ev_status": "inactive",
        "settings": learner_settings(&s.store),
        "findings": kv(crate::findings::FINDINGS_KEY).unwrap_or(Value::Null),
    });
    if let Some(stored) = s.store.get_kv(crate::NN_KEY).ok().flatten().and_then(|j| serde_json::from_str::<crate::StoredNet>(&j).ok()) {
        let (availability, status, _) = response_model_labels(&stored);
        t["neural_ev_status"] = json!(availability);
        t["neural_ev"] = json!({
            "status": status,
            "fallback": "hand-built stat response model",
            "validation_hands": stored.val_samples,
            "validation_bb100": Value::Null,
            "required_improvement_bb100": Value::Null,
            "val_log_loss": stored.val_loss,
            "baseline_log_loss": stored.baseline_loss,
        });
    }
    // What changed recently, with real write times: the dashboard must not imply idleness or activity.
    let updated = |k: &str| -> Value {
        s.store
            .kv_updated(k)
            .ok()
            .flatten()
            .and_then(|u| chrono::DateTime::parse_from_rfc3339(&u).ok())
            .map(|t| json!(t.timestamp()))
            .unwrap_or(Value::Null)
    };
    let observed: f64 = models.players.values().map(|p| p.hands as f64).sum();
    let mut learning = vec![json!({"what": "Opponent models", "updated": updated(crate::MODELS_KEY),
        "detail": format!("{} players, {:.0} observed hands; updated after every hand", models.players.len(), observed)})];
    if let Some(n) = s.store.get_kv(crate::NN_KEY).ok().flatten().and_then(|j| serde_json::from_str::<crate::StoredNet>(&j).ok()) {
        let (_, _, usage) = response_model_labels(&n);
        learning.push(json!({"what": "Neural response model", "updated": n.trained_at as i64,
            "detail": format!("{} · validation log-loss {:.4} vs {:.4} for the stat model · {} training samples",
                usage, n.val_loss, n.baseline_loss, n.train_samples)}));
    }
    if let Some(r) =
        s.store.get_kv(crate::RANGE_PARAMS_KEY).ok().flatten().and_then(|j| serde_json::from_str::<crate::StoredRangeParams>(&j).ok())
    {
        learning.push(json!({"what": "Range model fit", "updated": r.fitted_at as i64,
            "detail": format!("{} · refitted daily to showdowns", if r.active { "fitted set in use" } else { "defaults kept (fit not better)" })}));
    }
    learning.push(json!({"what": "Self-calibration", "updated": updated(crate::CALIBRATION_KEY),
        "detail": format!("{} active EV corrections", params.ev_bias.len())}));
    let promoted_at = experiments
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| e["status"] == "promoted")
        .filter_map(|e| e["ts"].as_f64())
        .fold(None, |m: Option<f64>, t| Some(m.map_or(t, |m| m.max(t))));
    let pacing = kv(crate::pacing::PACING_KEY);
    learning.push(json!({"what": "Strategy search", "updated": pacing.as_ref().and_then(|p| p["last_run"].as_f64()).map(|t| t as i64),
        "detail": format!("last promotion {} · {} cycles in a row without one",
            promoted_at.and_then(|t| chrono::DateTime::from_timestamp(t as i64, 0)).map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string()).unwrap_or_else(|| "none yet".into()),
            pacing.as_ref().and_then(|p| p["streak"].as_u64()).unwrap_or(0))}));
    // Decision analyst (separate process): deep re-solves of live decisions over the last 24 hours.
    let analyst = kv(crate::ANALYST_STATUS_KEY);
    if let Ok(audit) = s.store.audit_summary(&(chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339()) {
        let running = analyst.as_ref().and_then(|a| a["updated"].as_f64()).is_some_and(|u| now_secs() - u < 120.0);
        if audit.decisions > 0 || running {
            learning.push(json!({"what": "Decision audit", "updated": analyst.as_ref().and_then(|a| a["updated"].as_f64()).map(|t| t as i64),
                "detail": if audit.decisions > 0 {
                    format!("{} decisions re-solved with {} samples in 24 h · {:.1}% same action · live choice gives up {:.2} bb per decision on average · {} queued",
                        audit.decisions, analyst.as_ref().and_then(|a| a["samples"].as_u64()).unwrap_or(0), 100.0 * audit.same_action as f64 / audit.decisions as f64, audit.mean_gap_bb, audit.queued)
                } else {
                    format!("analyst running · {} decisions queued", audit.queued)
                }}));
        }
        t["audit"] = json!({"last_24h": audit, "analyst": analyst, "running": running});
    }
    t["learning"] = json!(learning);
    // The autonomy watchdog's view, same computation as `review autonomy` (0274 Q5): a stale
    // loop is shown, never hidden behind the healthy summary.
    let stale: Vec<Value> =
        crate::watchdog::stale_loops(now_secs(), &crate::watchdog::loops(crate::pacing::Pacing::from_env().max_idle_secs), |k| {
            s.store.get_kv(k)
        })
        .into_iter()
        .map(|l| json!({"name": l.name, "message": l.message}))
        .collect();
    t["stale_loops"] = json!(stale);
    if let Some(h) = kv(crate::history::STATUS_KEY) {
        t["past_hands"] = h;
    }
    let mut storage = kv(crate::tasks::INTEGRITY_STATUS_KEY).unwrap_or_else(|| json!({}));
    storage["tables"] = json!({"flop": sv10_core::tables::loaded(3).is_some(), "turn": sv10_core::tables::loaded(4).is_some()});
    t["storage"] = storage;
    if let Some(r) =
        s.store.get_kv(crate::RANGE_PARAMS_KEY).ok().flatten().and_then(|j| serde_json::from_str::<crate::StoredRangeParams>(&j).ok())
    {
        let f = |k: &str| r.report[k].as_f64();
        t["range_model"] = json!({
            "active": r.active,
            "fitted_at": r.fitted_at,
            "showdowns": r.report["train_samples"].as_u64().unwrap_or(0) + r.report["val_samples"].as_u64().unwrap_or(0),
            // Information about the shown hand gained from the action line, in nats over a uniform guess.
            "gain_fitted": f("val_ll").zip(f("uniform_ll")).map(|(a, b)| a - b),
            "gain_default": f("default_val_ll").zip(f("uniform_ll")).map(|(a, b)| a - b),
        });
    }
    if let Some(Value::Object(l)) = learner {
        for (k, v) in l {
            if k != "champion" {
                t[k] = v;
            }
        }
    }
    let neural_identity = s
        .store
        .get_kv(crate::NN_KEY)
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str::<crate::StoredNet>(&j).ok())
        .filter(crate::neural::response_net_is_eligible)
        .and_then(|n| crate::replay::net_digest(&n.net).map(|(digest, _)| digest));
    let range_identity = t["range_model"]
        .as_object()
        .map(|r| {
            if r.get("active").and_then(Value::as_bool).unwrap_or(false) {
                format!("fitted@{}", r.get("fitted_at").and_then(Value::as_f64).unwrap_or_default() as i64)
            } else {
                "defaults".to_string()
            }
        })
        .unwrap_or_else(|| "defaults".to_string());
    t["leaderboard_evidence"] = json!({
        "strategy": t["champion"]["version"].as_str().unwrap_or(POLICY_VERSION),
        "neural": neural_identity.unwrap_or_else(|| "stat fallback".to_string()),
        "range": range_identity,
        "calibration": updated(crate::CALIBRATION_KEY).as_i64().map(|ts| format!("updated@{ts}"))
            .unwrap_or_else(|| "no active version".to_string()),
        "opportunity": strongest_evidenced_opportunity(s),
        "next_candidate": t["next_candidate"].as_object().map(|candidate| format!("{} · {}",
            candidate.get("family").and_then(Value::as_str).unwrap_or("candidate"),
            candidate.get("reason").and_then(Value::as_str).unwrap_or("evidence pending"))),
    });
    t
}

/// The live correction that is buying the most chips right now: the applied self-calibration
/// entry whose measured gap between predicted and realized value covers the most decisions.
/// Reads the stored calibration table, so it costs nothing on a dashboard poll, and returns
/// `null` when no category has earned a correction (the panel then says so).
fn strongest_evidenced_opportunity(s: &Shared) -> Value {
    let table: Value = match s.store.get_kv(crate::CALIBRATION_KEY).ok().flatten().and_then(|v| serde_json::from_str(&v).ok()) {
        Some(v) => v,
        None => return Value::Null,
    };
    let mut best: Option<(f64, String)> = None;
    for (category, row) in table.as_object().into_iter().flatten() {
        let bias = row["bias_bb"].as_f64().unwrap_or(0.0);
        let (n, resid) = (row["n"].as_f64().unwrap_or(0.0), row["residual_bb"].as_f64().unwrap_or(0.0));
        if bias == 0.0 || n == 0.0 {
            continue;
        }
        let weight = resid.abs() * n;
        let direction = if resid < 0.0 { "below" } else { "above" };
        let text =
            format!("{category}: realized {:.1} bb {direction} prediction over {n:.0} decisions · correcting {bias:+.1} bb", resid.abs());
        if best.as_ref().is_none_or(|(w, _)| weight > *w) {
            best = Some((weight, text));
        }
    }
    best.map(|(_, text)| json!(text)).unwrap_or(Value::Null)
}

/// A worker's freshest heartbeat, when it may stand in for an offline local slot
/// (the read contract lives on `live::read_remote_bot`).
fn remote_bot(s: &Shared, local: &BotLive) -> Option<(BotLive, f64)> {
    // A worker runs one bot, so its own slot is always 0: show it under the head's slot, which the
    // bot tabs, per-bot endpoints and the per-slot metrics cache all key on.
    crate::live::read_remote_bot(&s.store, &local.name, now_secs()).map(|(mut remote, age)| {
        remote.slot = local.slot;
        (remote, age)
    })
}

/// Rich dashboard view: in split-fleet mode the head never spawns bots, so a fresh worker
/// heartbeat renders through the same [`bot_json`] (opponent profiles come from the head's
/// models, which trail the tables by at most the tailer interval).
fn fleet_bot_json(s: &Shared, b: &BotLive) -> Value {
    fleet_view(s, b, bot_json)
}

/// [`table_json`] with the split-fleet substitution of [`fleet_bot_json`].
pub(super) fn fleet_table_json(s: &Shared, b: &BotLive) -> Value {
    fleet_view(s, b, table_json)
}

/// A head shows an offline slot through its worker's heartbeat, marked `remote`.
fn fleet_view(s: &Shared, b: &BotLive, view: fn(&Shared, &BotLive) -> Value) -> Value {
    if b.mode != "offline" || !s.config.head {
        return view(s, b);
    }
    match remote_bot(s, b) {
        Some((remote, age)) => {
            let mut v = view(s, &remote);
            v["remote"] = json!(true);
            v["heartbeat_age_s"] = json!(age.round());
            v
        }
        None => view(s, b),
    }
}

/// Raw bot state with the same substitution: a remote bot looks like a local one plus
/// `remote` / `heartbeat_age_s`, so `/api/raw` consumers (status script, monitor) keep working.
fn fleet_bot_raw(s: &Shared, b: &BotLive) -> Value {
    if b.mode != "offline" || !s.config.head {
        return serde_json::to_value(b).unwrap_or(Value::Null);
    }
    match remote_bot(s, b) {
        Some((remote, age)) => {
            let mut v = serde_json::to_value(&remote).unwrap_or(Value::Null);
            v["remote"] = json!(true);
            v["heartbeat_age_s"] = json!(age.round());
            v
        }
        None => serde_json::to_value(b).unwrap_or(Value::Null),
    }
}

pub(super) fn snapshot(s: &Shared) -> Value {
    let bots: Vec<Value> = s.bots.iter().map(|b| fleet_bot_json(s, &b.read())).collect();
    let names: HashMap<String, usize> = s
        .bots
        .iter()
        .map(|b| {
            let b = b.read();
            (b.name.clone(), b.slot)
        })
        .collect();
    let logs: Vec<Value> = s
        .log
        .lock()
        .iter()
        .enumerate()
        .rev()
        .take(120)
        .map(|(i, l)| json!({"id": i, "slot": names.get(&l.bot), "ts": parse_ts(&l.ts), "level": l.level, "message": format!("{}: {}", l.bot, l.message)}))
        .collect();
    json!({
        "bots": bots,
        "training": training_json(s),
        "logs": logs,
        "updated": now_secs(),
        "state_hash": fleet_state_hash(s),
        "ops": ops_json(s),
        "config": {"buy_in": s.config.max_buy_in, "auto_rebuy": true, "host": s.config.web_host, "port": s.config.web_port, "configured_slots": s.bots.len(),
            "hardware": s.store.get_kv(crate::HARDWARE_PROFILE_KEY).ok().flatten().and_then(|v| serde_json::from_str::<Value>(&v).ok())},
    })
}

/// The server auto-acts 45 s after sending `your_turn` in public play (spec, Timeouts).
const TURN_DEADLINE_MS: f64 = 45_000.0;

/// Decision-time distribution: n, p50, p95, p99 and max in milliseconds.
fn latency_summary(mut ms: Vec<f64>) -> Value {
    if ms.is_empty() {
        return json!({"n": 0, "p50": null, "p95": null, "p99": null, "max": null});
    }
    ms.sort_by(f64::total_cmp);
    let at = |q: f64| ms[((ms.len() as f64 * q) as usize).min(ms.len() - 1)];
    json!({"n": ms.len(), "p50": at(0.5), "p95": at(0.95), "p99": at(0.99), "max": ms[ms.len() - 1]})
}

/// Compute use of the live decision path (0162): the last hour's decision time overall and per
/// street against the 45 s deadline, the live Monte Carlo budget and the machine's load.
pub(super) fn compute_value(s: &Shared) -> Value {
    let since = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    let rows = snapshot_read("decision latencies", s.store.decision_latencies_since(&since));
    let mut by_street: HashMap<String, Vec<f64>> = HashMap::new();
    for (street, ms) in &rows {
        by_street.entry(street.clone()).or_default().push(*ms);
    }
    let all = latency_summary(rows.iter().map(|(_, ms)| *ms).collect());
    let deadline_share = all["max"].as_f64().map(|m| m / TURN_DEADLINE_MS);
    let streets: serde_json::Map<String, Value> = ["preflop", "flop", "turn", "river"]
        .iter()
        .map(|st| (st.to_string(), latency_summary(by_street.remove(*st).unwrap_or_default())))
        .collect();
    let hardware = s.store.get_kv(crate::HARDWARE_PROFILE_KEY).ok().flatten().and_then(|v| serde_json::from_str::<Value>(&v).ok());
    let load: Option<Vec<f64>> =
        std::fs::read_to_string("/proc/loadavg").ok().map(|l| l.split_whitespace().take(3).filter_map(|x| x.parse().ok()).collect());
    let params = s.params.read();
    let timeouts: u64 = s.bots.iter().map(|b| b.read().decision_timeouts).sum();
    json!({
        "window_minutes": 60,
        "decisions": all,
        "timeouts": timeouts,
        "by_street": streets,
        "deadline_ms": TURN_DEADLINE_MS,
        "max_share_of_deadline": deadline_share,
        "live_samples": params.samples,
        "deal_chunks": params.deal_chunks,
        "logical_cores": hardware.as_ref().and_then(|h| h["logical_cores"].as_u64()),
        "cpu_model": hardware.as_ref().and_then(|h| h["cpu_model"].as_str().map(str::to_string)),
        "load_average": load,
        "profile": compute_profile_json(s, hardware.as_ref().and_then(|h| h["logical_cores"].as_u64()).unwrap_or(1) as usize),
    })
}

/// The compute profile in effect (`max` when none is stored) and the presets for this machine (0187).
pub(super) fn compute_profile_json(s: &Shared, logical: usize) -> Value {
    let stored = crate::profile::ComputeProfile::stored(s.store.get_kv(crate::profile::PROFILE_KEY).ok().flatten().as_deref(), logical);
    let presets = crate::profile::ComputeProfile::presets(logical);
    let active = stored.clone().unwrap_or_else(|| presets[presets.len() - 1].clone());
    json!({"active": active, "stored": stored.is_some(), "presets": presets, "logical_cores": logical, "min_live_scale": crate::profile::MIN_LIVE_SCALE})
}

/// Rivalry cards (0180): nemeses and donors from the live head-to-head ledger, 150+ shared hands.
pub(super) async fn rivals(State(s): State<Arc<Shared>>) -> Response {
    off_runtime(move || {
        let bb = s.bots.first().map(|b| b.read().big_blind).filter(|b| *b > 0).unwrap_or(20) as f64;
        let avatars: HashMap<String, String> =
            s.avatars.read().iter().filter_map(|(n, a)| Some((n.clone(), crate::avatar_url(&s.config.rest_base, a)?))).collect();
        Json(crate::headtohead::rivals(&s.head_to_head.read(), &avatars, 150.0, bb, 5))
    })
    .await
    .into_response()
}

pub(super) async fn compute(State(s): State<Arc<Shared>>) -> Response {
    off_runtime(move || Json(compute_value(&s))).await.into_response()
}

/// Operations the fleet cannot see from a table: hands waiting for a store retry (0152) and the
/// keepalive hold `scripts/stop.sh` leaves (0145): "forever", a unix time, or null.
fn ops_json(s: &Shared) -> Value {
    let hold = std::fs::read_to_string(s.config.artifacts.join("hold-until")).ok().map(|h| h.trim().to_string());
    let hold_until = match hold.as_deref() {
        Some("forever") => json!("forever"),
        Some(t) => t.parse::<i64>().map(|t| json!(t)).unwrap_or(Value::Null),
        None => Value::Null,
    };
    // The keepalive logs only decisions to act, so its last line is the last time it had to
    // bring the fleet back (or found a release holding the lock).
    let keepalive_last = std::fs::read_to_string(s.config.artifacts.join("logs").join("keepalive.log"))
        .ok()
        .and_then(|log| log.lines().rev().find(|l| !l.trim().is_empty()).map(str::to_string));
    json!({"unstored_hands": s.unstored_hands.lock().len(), "hold_until": hold_until, "keepalive_last": keepalive_last})
}

pub(super) async fn state(State(s): State<Arc<Shared>>) -> Response {
    off_runtime(move || state_blocking(&s)).await.into_response()
}

fn state_blocking(s: &Shared) -> Json<Value> {
    Json(snapshot(s))
}

/// Realtime stream (0211). Table events (action, board, result, decision, hand) are forwarded the
/// moment they happen, and a bot's whole live table follows every change as a `table` event, at
/// most every [`TABLE_EVERY`] per bot (bursts coalesce into the latest state). The full `state`
/// snapshot (metrics, training, logs; ~100 KB) comes every [`STATE_EVERY`].
pub(super) async fn events(State(s): State<Arc<Shared>>) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let rx = s.events.subscribe();
    struct St {
        s: Arc<Shared>,
        rx: tokio::sync::broadcast::Receiver<String>,
        last_state: Option<Instant>,
        dirty: std::collections::BTreeSet<usize>,
        last_table: HashMap<usize, Instant>,
    }
    let init = St { s, rx, last_state: None, dirty: Default::default(), last_table: HashMap::new() };
    let stream = futures_util::stream::unfold(init, |mut st| async move {
        loop {
            let now = Instant::now();
            if st.last_state.is_none_or(|t| now.duration_since(t) >= STATE_EVERY) {
                st.last_state = Some(now);
                let shared = st.s.clone();
                // A failed snapshot (logged) skips this event; the next one is due in STATE_EVERY.
                if let Ok(data) = off_runtime(move || snapshot(&shared).to_string()).await {
                    return Some((Ok(Event::default().event("state").data(data)), st));
                }
                continue;
            }
            let ready = |slot: &usize| st.last_table.get(slot).is_none_or(|t| now.duration_since(*t) >= TABLE_EVERY);
            if let Some(slot) = st.dirty.iter().copied().find(ready) {
                st.dirty.remove(&slot);
                st.last_table.insert(slot, now);
                let shared = st.s.clone();
                let data = off_runtime(move || {
                    shared.bots.get(slot).map(|b| json!({"slot": slot, "bot": fleet_table_json(&shared, &b.read())}).to_string())
                })
                .await;
                match data {
                    Ok(Some(d)) => return Some((Ok(Event::default().event("table").data(d)), st)),
                    _ => continue,
                }
            }
            let state_due = STATE_EVERY.saturating_sub(st.last_state.map_or(Duration::ZERO, |t| now.duration_since(t)));
            let table_due = st
                .dirty
                .iter()
                .map(|slot| st.last_table.get(slot).map_or(Duration::ZERO, |t| TABLE_EVERY.saturating_sub(now.duration_since(*t))))
                .min()
                .unwrap_or(state_due);
            match tokio::time::timeout(state_due.min(table_due).max(Duration::from_millis(10)), st.rx.recv()).await {
                Ok(Ok(msg)) => {
                    let v = serde_json::from_str::<Value>(&msg).unwrap_or(Value::Null);
                    let kind = v["type"].as_str().unwrap_or_default().to_string();
                    if let Some(slot) = v["slot"].as_u64() {
                        st.dirty.insert(slot as usize);
                    }
                    if matches!(kind.as_str(), "action" | "board" | "result" | "decision" | "hand") {
                        return Some((Ok(Event::default().event(kind).data(msg)), st));
                    }
                }
                // Missed messages: refresh every table.
                Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => st.dirty.extend(0..st.s.bots.len()),
                Ok(Err(_)) => return None,
                Err(_) => {}
            }
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// Shortest gap between two `table` events for one bot. The public TV throttles its own stream at
/// the same rate: a spectator gets the same picture at the same cost as the dashboard.
pub(super) const TABLE_EVERY: Duration = Duration::from_millis(150);
/// Gap between full `state` snapshots on the realtime stream.
const STATE_EVERY: Duration = Duration::from_secs(5);

pub(super) async fn raw_state(State(s): State<Arc<Shared>>) -> Json<Value> {
    let bots: Vec<Value> = s.bots.iter().map(|b| fleet_bot_raw(&s, &b.read())).collect();
    let log: Vec<Value> = s.log.lock().iter().rev().take(50).map(|l| serde_json::to_value(l).unwrap()).collect();
    Json(
        json!({"version": crate::VERSION, "started_at": s.started_at, "bots": bots, "log": log, "known_opponents": s.models.read().players.len()}),
    )
}

#[allow(clippy::result_large_err)] // the error is the ready-to-send HTTP response
pub(super) fn bot_name(s: &Shared, slot: usize) -> Result<String, Response> {
    s.bots
        .get(slot)
        .map(|b| b.read().name.clone())
        .ok_or_else(|| (StatusCode::NOT_FOUND, Json(json!({"detail": "Unknown bot slot"}))).into_response())
}

#[cfg(test)]
mod tests;

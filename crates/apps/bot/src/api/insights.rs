//! Analytics endpoints: leaderboard, fleet race, highlights, calibration, leak finder and
//! range explorer.

use super::*;

pub mod leaderboard;
// Re-exported so the routes keep their paths (0259); the tests import the panel's
// internals from the module directly.
pub(super) use leaderboard::{leaderboard, leaderboard_entry};

pub(super) struct TimedCache {
    at: Option<Instant>,
    value: Value,
}

/// Badge race (0182): our bots on the `score`, `win_rate` and `hands_played` sorts, refreshed at most
/// every 5 minutes (three requests against the 30/minute leaderboard limit). A failed refresh keeps
/// the last good answer and says so.
pub(super) async fn badges(State(s): State<Arc<Shared>>) -> Json<Value> {
    static C: OnceLock<tokio::sync::Mutex<(Option<Instant>, Value)>> = OnceLock::new();
    let mut cache = C.get_or_init(|| tokio::sync::Mutex::new((None, Value::Null))).lock().await;
    if cache.0.is_some_and(|t| t.elapsed() < Duration::from_secs(300)) {
        return Json(cache.1.clone());
    }
    let ours: Vec<String> = s.bots.iter().map(|b| b.read().name.clone()).collect();
    let fetch = async {
        let http = reqwest::Client::builder()
            .user_agent(crate::USER_AGENT)
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| "client setup failed".to_string())?;
        let mut sorts = Vec::new();
        for sort in ["score", "win_rate", "hands_played"] {
            let url = format!("{}/season/leaderboard?sort_by={sort}&min_hands=10&limit=1000", s.config.rest_base);
            let entries = http
                .get(&url)
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|_| format!("{sort} leaderboard request failed"))?
                .json::<Vec<Value>>()
                .await
                .map_err(|_| format!("{sort} leaderboard JSON failed"))?;
            sorts.push(entries);
        }
        Ok::<_, String>(sorts)
    }
    .await;
    match fetch {
        Ok(sorts) => {
            let mut v = crate::badges::badge_race(&sorts[0], &sorts[1], &sorts[2], &ours);
            v["updated_at"] = json!(now_secs());
            v["error"] = Value::Null;
            *cache = (Some(Instant::now()), v);
        }
        Err(e) => {
            if cache.1.is_null() {
                cache.1 = json!({"bots": [], "updated_at": null});
            }
            cache.1["error"] = json!(e);
            // Retry sooner than a success would, but not on every poll.
            cache.0 = Some(Instant::now() - Duration::from_secs(240));
        }
    }
    Json(cache.1.clone())
}

/// Cumulative net chips over this season for every bot, for the fleet race chart.
///
/// The race is a season standing, so hand 1 of each curve is that bot's first hand of the current
/// season; the previous season is a separate contest and is reported only as `all_time`.
pub(super) async fn fleet(State(s): State<Arc<Shared>>) -> Response {
    off_runtime(move || fleet_blocking(&s)).await.into_response()
}

fn fleet_blocking(s: &Shared) -> Result<Json<Value>, ApiError> {
    let season = s.season();
    let names: Vec<String> = s.bots.iter().map(|b| b.read().name.clone()).collect();
    let mut series = Vec::new();
    for name in names {
        let mut all: Vec<(Option<i64>, bool, String)> = s
            .names_of(&name)
            .iter()
            .map(|n| store_read("bot results", s.store.bot_results(n)))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        all.sort_by(|a, b| a.2.cmp(&b.2));
        let all_time = json!({"hands": all.len(), "total": all.iter().map(|(net, _, _)| net.unwrap_or(0)).sum::<i64>()});
        let rows = this_season(season.as_ref(), all, |(_, _, ended): &(Option<i64>, bool, String)| parse_ts(ended));
        let step = (rows.len() / 120).max(1);
        let mut total = 0i64;
        let mut points = Vec::new();
        for (i, (net, _, ended)) in rows.iter().enumerate() {
            total += net.unwrap_or(0);
            if i % step == 0 || i + 1 == rows.len() {
                points.push(json!({"hand": i + 1, "ts": parse_ts(ended), "total": total}));
            }
        }
        series.push(json!({"name": name, "hands": rows.len(), "total": total, "points": points, "all_time": all_time}));
    }
    Ok(Json(json!({"bots": series, "season": season_scope(season.as_ref()), "updated": now_secs()})))
}

/// Memorable hands from this season, with cumulative milestone badges.
///
/// The figures the panel presents as the fleet's current standing (totals, hands, the memorable
/// hands, the best streak) are this season's; the badges are achievements and stay cumulative, so
/// one already earned is never taken away by a season rollover.
/// Stories (0178): the last 24 hours as a recap and this season as a timeline of milestones and big pots.
pub(super) async fn stories(State(s): State<Arc<Shared>>) -> Response {
    off_runtime(move || -> Result<Json<Value>, ApiError> {
        let season = s.season();
        let names: Vec<String> = s.bots.iter().map(|b| b.read().name.clone()).collect();
        let bb = s.bots.first().map(|b| b.read().big_blind).filter(|b| *b > 0).unwrap_or(20) as f64;
        let mut hands = Vec::new();
        for name in names {
            for alias in s.names_of(&name) {
                for r in store_read("story hands", s.store.recent_hands_light(&alias, 100_000))? {
                    let ts = parse_ts(&r.ended_at);
                    if season.as_ref().is_some_and(|se| !se.contains(ts)) { continue; }
                    hands.push(crate::stories::StoryHand { bot: name.clone(), hand_id: r.hand_id, ts, net: r.net.unwrap_or(0), pot: r.pot });
                }
            }
        }
        hands.sort_by(|a, b| a.ts.total_cmp(&b.ts));
        Ok(Json(json!({"recap": crate::stories::recap(&hands, now_secs(), 24.0 * 3600.0, bb), "timeline": crate::stories::timeline(&hands, bb)})))
    })
    .await.into_response()
}

pub(super) async fn highlights(State(s): State<Arc<Shared>>) -> Response {
    off_runtime(move || highlights_blocking(&s)).await.into_response()
}

fn highlights_blocking(s: &Shared) -> Result<Json<Value>, ApiError> {
    let season = s.season();
    let bots: Vec<(usize, String)> = s
        .bots
        .iter()
        .map(|b| {
            let b = b.read();
            (b.slot, b.name.clone())
        })
        .collect();
    let mut all = Vec::new();
    let mut best_streak: (i64, String) = (0, String::new());
    let mut lifetime_streak: (i64, String) = (0, String::new());
    let (mut fleet_total, mut fleet_hands) = (0i64, 0i64);
    let (mut lifetime_total, mut lifetime_hands) = (0i64, 0i64);
    let mut monsters = Vec::new();
    let mut slams = Vec::new();
    // Badge facts, judged over every season the fleet has played.
    let (mut royalty, mut any_monster, mut any_win) = (false, false, false);
    let (mut showdown_wins, mut best_net) = (0i64, 0i64);
    for (slot, name) in &bots {
        // Every name this bot played under, oldest first (a rename keeps one record).
        let mut rows: Vec<_> = s
            .names_of(name)
            .iter()
            .map(|n| store_read("recent hands", s.store.recent_hands_light(n, 100_000)))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        rows.sort_by(|a, b| a.ended_at.cmp(&b.ended_at));
        let (mut streak, mut lifetime) = (0i64, 0i64);
        for r in &rows {
            let net = r.net.unwrap_or(0);
            let this_season = season.as_ref().map(|s| s.contains(parse_ts(&r.ended_at))).unwrap_or(true);
            lifetime_total += net;
            lifetime_hands += 1;
            any_win |= net > 0;
            best_net = best_net.max(net);
            if net > 0 {
                lifetime += 1;
                if lifetime > lifetime_streak.0 {
                    lifetime_streak = (lifetime, name.clone());
                }
            } else if net < 0 {
                lifetime = 0;
            }
            if this_season {
                fleet_total += net;
                fleet_hands += 1;
                if net > 0 {
                    streak += 1;
                    if streak > best_streak.0 {
                        best_streak = (streak, name.clone());
                    }
                } else if net < 0 {
                    streak = 0;
                }
            }
            let hole: Vec<Card> = split_cards(&r.hole).iter().filter_map(|c| Card::parse(c)).collect();
            let board: Vec<Card> = split_cards(&r.board).iter().filter_map(|c| Card::parse(c)).collect();
            if board.len() == 5 && hole.len() == 2 && net > 0 {
                let v = eval(mask_of(&[hole.clone(), board.clone()].concat()));
                if category(v) >= sv10_core::eval::FULL_HOUSE {
                    any_monster = true;
                    if this_season {
                        monsters.push((v, json!({"slot": slot, "bot": name, "hand_id": r.hand_id, "hole": split_cards(&r.hole), "board": split_cards(&r.board), "net": net, "ts": parse_ts(&r.ended_at), "category": best_five(&hole, &board).map(|b| b.0)})));
                    }
                }
                if category(v) >= sv10_core::eval::QUADS {
                    royalty = true;
                }
            }
            // Showdown slams: big pots taken to showdown and won.
            if r.showdown && net > 0 {
                showdown_wins += 1;
                if r.pot >= 5_000 && this_season {
                    slams.push((net, json!({"slot": slot, "bot": name, "hand_id": r.hand_id, "hole": split_cards(&r.hole), "board": split_cards(&r.board), "net": net, "pot": r.pot, "ts": parse_ts(&r.ended_at)})));
                }
            }
            if this_season {
                all.push((net, *slot, name.clone(), r.clone()));
            }
        }
    }
    let hand_json = |(net, slot, name, r): &(i64, usize, String, sv10_store::store::HandRow)| json!({"slot": slot, "bot": name, "hand_id": r.hand_id, "hole": split_cards(&r.hole), "board": split_cards(&r.board), "net": net, "pot": r.pot, "ts": parse_ts(&r.ended_at)});
    let mut by_net = all.clone();
    by_net.sort_by_key(|x| std::cmp::Reverse(x.0));
    let biggest_wins: Vec<Value> = by_net.iter().take(5).map(hand_json).collect();
    let biggest_losses: Vec<Value> = by_net.iter().rev().take(3).map(hand_json).collect();
    monsters.sort_by_key(|m| std::cmp::Reverse(m.0));
    slams.sort_by_key(|s| std::cmp::Reverse(s.0));
    let milestones = [
        ("first-blood", "First blood", "Win the fleet's first pot", any_win),
        ("ten-k", "10K club", "Fleet up 10,000 chips", lifetime_total >= 10_000),
        ("fifty-k", "High roller", "Fleet up 50,000 chips", lifetime_total >= 50_000),
        ("hundred-k", "Six figures", "Fleet up 100,000 chips", lifetime_total >= 100_000),
        ("million-club", "Million club", "Fleet up 1,000,000 chips", lifetime_total >= 1_000_000),
        ("thousand-hands", "Grinder", "1,000 hands played", lifetime_hands >= 1_000),
        ("ten-thousand-hands", "Iron seat", "10,000 hands played", lifetime_hands >= 10_000),
        ("fifty-k-hands", "Everest", "50,000 hands played", lifetime_hands >= 50_000),
        ("big-pot", "Stack them", "Win a single pot worth 5,000+ chips", best_net >= 5_000),
        ("deep-stack", "Deep stack destroyer", "Net 25,000+ chips in one hand", best_net >= 25_000),
        ("monster", "Monster hand", "Win with a full house or better", any_monster),
        ("royalty", "Royalty", "Win with four of a kind or better", royalty),
        ("showdown-pro", "Showdown pro", "Win 100 showdowns", showdown_wins >= 100),
        ("streak", "Hot streak", "Win 6 hands in a row", lifetime_streak.0 >= 6),
        ("unstoppable", "Unstoppable", "Win 12 hands in a row", lifetime_streak.0 >= 12),
    ];
    Ok(Json(json!({
        "fleet_total": fleet_total,
        "fleet_hands": fleet_hands,
        "season": season_scope(season.as_ref()),
        "biggest_wins": biggest_wins,
        "biggest_losses": biggest_losses,
        "monsters": monsters.iter().take(5).map(|m| m.1.clone()).collect::<Vec<_>>(),
        "slams": slams.iter().take(5).map(|s| s.1.clone()).collect::<Vec<_>>(),
        "best_streak": {"length": best_streak.0, "bot": best_streak.1},
        "milestones": milestones.iter().map(|(id, title, detail, done)| json!({"id": id, "title": title, "detail": detail, "unlocked": done})).collect::<Vec<_>>(),
        "updated": now_secs(),
    })))
}

/// Self-calibration table: predicted vs realized chips by spot category and the correction in use,
/// plus the live-fitted fold calibration (0156) and river all-in call fit (0159) with the shifts
/// the policy is using now (so a stored fit that has not reached live play shows as a mismatch).
pub(super) async fn calibration(State(s): State<Arc<Shared>>) -> Response {
    off_runtime(move || calibration_value(&s).map(Json)).await.into_response()
}

pub(super) fn calibration_value(s: &Shared) -> Result<Value, ApiError> {
    let live = crate::livefits::LiveFits::of(&s.params.read());
    let table = stored_json(s, crate::CALIBRATION_KEY)?.unwrap_or_else(|| json!({}));
    let mut rows: Vec<Value> = table
        .as_object()
        .map(|o| {
            o.iter()
                .map(|(k, v)| {
                    let mut v = v.clone();
                    v["category"] = json!(k);
                    v
                })
                .collect()
        })
        .unwrap_or_default();
    rows.sort_by(|a, b| b["n"].as_i64().unwrap_or(0).cmp(&a["n"].as_i64().unwrap_or(0)));
    let active = rows.iter().filter(|r| r["bias_bb"].as_f64().unwrap_or(0.0) != 0.0).count();
    Ok(json!({"rows": rows, "active_corrections": active, "updated": now_secs(),
        "fold": stored_json(s, crate::foldcal::FOLD_CAL_KEY)?, "river_jam": stored_json(s, crate::raisewar::RIVER_JAM_KEY)?,
        "live": live}))
}

pub(super) fn analysis_cache() -> &'static tokio::sync::Mutex<TimedCache> {
    static C: OnceLock<tokio::sync::Mutex<TimedCache>> = OnceLock::new();
    C.get_or_init(|| tokio::sync::Mutex::new(TimedCache { at: None, value: Value::Null }))
}

/// Leak finder report (recomputed at most every five minutes).
pub(super) async fn analysis(State(s): State<Arc<Shared>>) -> Result<Json<Value>, ApiError> {
    let mut cache = analysis_cache().lock().await;
    if cache.at.map(|t| t.elapsed() < Duration::from_secs(300)).unwrap_or(false) && !cache.value.is_null() {
        return Ok(Json(cache.value.clone()));
    }
    let s2 = s.clone();
    let value = off_runtime(move || -> Result<Value, ApiError> {
        let fleet: Vec<String> = s2.config.bots.iter().map(|b| b.name.clone()).collect();
        let h2h = s2.head_to_head.read().clone();
        let calibration = stored_json(&s2, crate::CALIBRATION_KEY)?;
        let bb = s2.big_blind();
        let season = s2.season();
        store_read("leak analysis hands", crate::analysis::report(&s2.store, &fleet, &h2h, calibration.as_ref(), bb, season.as_ref()))
    })
    .await??;
    cache.at = Some(Instant::now());
    cache.value = value.clone();
    Ok(Json(value))
}

/// Monte Carlo samples behind each range-explorer equity (0161): 200k dealt across every core is a
/// few tens of milliseconds and a standard error under 0.12 percentage points (3,000 gave ~0.9).
const RANGE_EXPLORER_SAMPLES: usize = 200_000;

/// Standard error of an equity estimate from `n` samples.
fn equity_se(equity: f64, n: usize) -> f64 {
    (equity * (1.0 - equity) / n.max(1) as f64).sqrt()
}

/// Range explorer for a bot's latest decision: each live opponent's estimated range as a
/// 13x13 class grid (share of the range's weight per class), our equity against each range and
/// against all of them together, and the top classes.
pub(super) async fn ranges(State(s): State<Arc<Shared>>, Path(slot): Path<usize>) -> Response {
    let Some(bot) = s.bots.get(slot) else { return (StatusCode::NOT_FOUND, Json(json!({"detail": "no such bot"}))).into_response() };
    let Some(sit) = bot.read().last_situation.clone() else { return Json(json!({"available": false})).into_response() };
    let s2 = s.clone();
    let v = tokio::task::spawn_blocking(move || {
        use sv10_rng::SeedableRng;
        let models = s2.models.read().clone();
        let params = s2.params.read().clone();
        let ranges = sv10_core::oprange::estimate_ranges(&sit, &models, &params.range);
        let table = sv10_core::range::combos();
        let mut rng = sv10_rng::rngs::SmallRng::seed_from_u64(7);
        let chunks = params.deal_chunks.max(1);
        let mut opponents = Vec::new();
        let mut all: Vec<&sv10_core::range::Range> = Vec::new();
        for p in sit.live_opponents() {
            let Some(r) = ranges.get(&p.seat) else { continue };
            all.push(r);
            let mut grid = vec![0f64; 169];
            for (i, &(a, b)) in table.cards.iter().enumerate() {
                grid[sv10_core::range::hand_class(a, b)] += r.w[i] as f64;
            }
            let total: f64 = grid.iter().sum::<f64>().max(1e-12);
            // Rank by likelihood per combo: offsuit classes hold 12 combos and suited ones 4, so
            // raw class weight would make untouched ranges look like "mostly 32o".
            let density: Vec<f64> = grid.iter().enumerate().map(|(c, w)| w / sv10_core::range::class_combos(c)).collect();
            let max_density = density.iter().copied().fold(0.0, f64::max).max(1e-12);
            let mut top: Vec<(usize, f64)> = density.iter().copied().enumerate().collect();
            top.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            let equity = sv10_core::equity::equity_vs_ranges_parallel(sit.hole, &sit.board, &[r], RANGE_EXPLORER_SAMPLES, chunks, &mut rng);
            let profile = models.profile(&p.name);
            opponents.push(json!({
                "seat": p.seat,
                "name": p.name,
                "position": sit.position_of(p.seat).name(),
                // Per-class likelihood relative to the most likely class (1 = most likely), and each
                // class's share of the whole range.
                "grid": density.iter().map(|d| d / max_density).collect::<Vec<_>>(),
                "share": grid.iter().map(|g| g / total).collect::<Vec<_>>(),
                "top": top.iter().take(8).map(|(c, _)| json!({"hand": sv10_core::range::class_name(*c), "share": grid[*c] / total})).collect::<Vec<_>>(),
                "equity": equity,
                "equity_se": equity.map(|e| equity_se(e, RANGE_EXPLORER_SAMPLES)),
                "profile": {"hands": profile.hands, "vpip": profile.vpip, "pfr": profile.pfr, "confidence": profile.confidence},
            }));
        }
        let combined = if all.is_empty() {
            None
        } else {
            sv10_core::equity::equity_vs_ranges_parallel(sit.hole, &sit.board, &all, RANGE_EXPLORER_SAMPLES, chunks, &mut rng)
        };
        json!({
            "available": true,
            "hole": sit.hole.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
            "board": sit.board.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
            "street": sit.street.name(),
            "pot": sit.pot,
            "to_call": sit.call_amount,
            "position": sit.position_of(sit.hero_seat).name(),
            "equity_vs_all": combined,
            "equity_vs_all_se": combined.map(|e| equity_se(e, RANGE_EXPLORER_SAMPLES)),
            "samples": RANGE_EXPLORER_SAMPLES,
            "opponents": opponents,
            "range_model": if params.range == sv10_core::oprange::RangeParams::DEFAULT { "defaults" } else { "showdown-fitted" },
        })
    })
    .await
    .unwrap_or(Value::Null);
    Json(v).into_response()
}

#[cfg(test)]
mod tests;

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

fn fleet_blocking(s: &Shared) -> Json<Value> {
    let season = s.season();
    let names: Vec<String> = s.bots.iter().map(|b| b.read().name.clone()).collect();
    let mut series = Vec::new();
    for name in names {
        let mut all: Vec<(Option<i64>, bool, String)> =
            s.names_of(&name).iter().flat_map(|n| snapshot_read("bot results", s.store.bot_results(n))).collect();
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
    Json(json!({"bots": series, "season": season_scope(season.as_ref()), "updated": now_secs()}))
}

/// Memorable hands from this season, with cumulative milestone badges.
///
/// The figures the panel presents as the fleet's current standing (totals, hands, the memorable
/// hands, the best streak) are this season's; the badges are achievements and stay cumulative, so
/// one already earned is never taken away by a season rollover.
/// Stories (0178): the last 24 hours as a recap and this season as a timeline of milestones and big pots.
pub(super) async fn stories(State(s): State<Arc<Shared>>) -> Response {
    off_runtime(move || {
        let season = s.season();
        let names: Vec<String> = s.bots.iter().map(|b| b.read().name.clone()).collect();
        let bb = s.bots.first().map(|b| b.read().big_blind).filter(|b| *b > 0).unwrap_or(20) as f64;
        let mut hands: Vec<crate::stories::StoryHand> = names
            .iter()
            .flat_map(|name| {
                s.names_of(name)
                    .iter()
                    .flat_map(|n| snapshot_read("recent hands", s.store.recent_hands_light(n, 100_000)))
                    .map(|r| crate::stories::StoryHand {
                        bot: name.clone(),
                        hand_id: r.hand_id.clone(),
                        ts: parse_ts(&r.ended_at),
                        net: r.net.unwrap_or(0),
                        pot: r.pot,
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|h| season.as_ref().map(|se| se.contains(h.ts)).unwrap_or(true))
            .collect();
        hands.sort_by(|a, b| a.ts.total_cmp(&b.ts));
        Json(json!({"recap": crate::stories::recap(&hands, now_secs(), 24.0 * 3600.0, bb), "timeline": crate::stories::timeline(&hands, bb)}))
    })
    .await.into_response()
}

pub(super) async fn highlights(State(s): State<Arc<Shared>>) -> Response {
    off_runtime(move || highlights_blocking(&s)).await.into_response()
}

fn highlights_blocking(s: &Shared) -> Json<Value> {
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
        let mut rows: Vec<_> =
            s.names_of(name).iter().flat_map(|n| snapshot_read("recent hands", s.store.recent_hands_light(n, 100_000))).collect();
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
    Json(json!({
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
    }))
}

/// Self-calibration table: predicted vs realized chips by spot category and the correction in use,
/// plus the live-fitted fold calibration (0156) and river all-in call fit (0159) with the shifts
/// the policy is using now (so a stored fit that has not reached live play shows as a mismatch).
pub(super) async fn calibration(State(s): State<Arc<Shared>>) -> Response {
    off_runtime(move || Json(calibration_value(&s))).await.into_response()
}

pub(super) fn calibration_value(s: &Shared) -> Value {
    let stored = |key: &str| s.store.get_kv(key).ok().flatten().and_then(|v| serde_json::from_str::<Value>(&v).ok()).unwrap_or(Value::Null);
    let live = crate::livefits::LiveFits::of(&s.params.read());
    let table = s
        .store
        .get_kv(crate::CALIBRATION_KEY)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str::<Value>(&v).ok())
        .unwrap_or_else(|| json!({}));
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
    json!({"rows": rows, "active_corrections": active, "updated": now_secs(),
        "fold": stored(crate::foldcal::FOLD_CAL_KEY), "river_jam": stored(crate::raisewar::RIVER_JAM_KEY),
        "live": live})
}

pub(super) fn analysis_cache() -> &'static tokio::sync::Mutex<TimedCache> {
    static C: OnceLock<tokio::sync::Mutex<TimedCache>> = OnceLock::new();
    C.get_or_init(|| tokio::sync::Mutex::new(TimedCache { at: None, value: Value::Null }))
}

/// Leak finder report (recomputed at most every five minutes).
pub(super) async fn analysis(State(s): State<Arc<Shared>>) -> Json<Value> {
    let mut cache = analysis_cache().lock().await;
    if cache.at.map(|t| t.elapsed() < Duration::from_secs(300)).unwrap_or(false) && !cache.value.is_null() {
        return Json(cache.value.clone());
    }
    let s2 = s.clone();
    let value = tokio::task::spawn_blocking(move || {
        let fleet: Vec<String> = s2.config.bots.iter().map(|b| b.name.clone()).collect();
        let h2h = s2.head_to_head.read().clone();
        let calibration = s2.store.get_kv(crate::CALIBRATION_KEY).ok().flatten().and_then(|v| serde_json::from_str::<Value>(&v).ok());
        let bb = s2.big_blind();
        let season = s2.season();
        crate::analysis::report(&s2.store, &fleet, &h2h, calibration.as_ref(), bb, season.as_ref())
    })
    .await
    .unwrap_or(Value::Null);
    cache.at = Some(Instant::now());
    cache.value = value.clone();
    Json(value)
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
mod tests {
    use super::leaderboard::{HistoricalStanding, LeaderboardHistory, LeaderboardState, enrich_leaderboard, transition_leaderboard};
    use super::*;
    use std::collections::HashMap;

    fn standings() -> Vec<Value> {
        vec![
            json!({"rank":1,"bot_name":"leader","score":2_000,"hands_played":100,"win_rate":3.0,"pro":true}),
            json!({"rank":2,"bot_name":"alpha","score":1_800,"hands_played":100,"win_rate":2.0,"pro":false}),
            json!({"rank":4,"bot_name":"fourth","score":1_500,"hands_played":100,"win_rate":1.0,"pro":false}),
            json!({"rank":7,"bot_name":"ours","score":1_100,"hands_played":100,"win_rate":0.5,"pro":false}),
        ]
    }

    #[test]
    fn leaderboard_derives_identity_deltas_actual_rank_gaps_and_velocity() {
        let previous = LeaderboardHistory {
            updated: 1_000.0,
            season: Some("12".into()),
            standings: HashMap::from([
                ("ours".into(), HistoricalStanding { rank: Some(5), score: Some(1_000), hands: Some(80) }),
                ("alpha".into(), HistoricalStanding { rank: Some(3), score: Some(1_700), hands: Some(90) }),
            ]),
        };
        let entries = standings();
        let value = enrich_leaderboard(&entries, &["ours".into()], json!({"season_number":12}), Some(&previous), 4_600.0);
        let rows = value["entries"].as_array().unwrap();
        let ours = rows.iter().find(|row| row["name"] == "ours").unwrap();
        assert_eq!(ours["rank_delta"], -2);
        assert_eq!(ours["score_delta"], 100);
        assert_eq!(ours["gap_to_first"], 900);
        assert_eq!(ours["gap_to_next"], 400);
        assert_eq!(ours["gap_to_four"], 400);
        assert_eq!(ours["score_velocity_per_hour"], 100.0);
        assert_eq!(ours["hands_delta"], 20);
        assert_eq!(ours["hands_velocity_per_hour"], 20.0);
        assert_eq!(rows[1]["rank_delta"], 1);
        assert_eq!(rows[1]["gap_to_next"], 200);
        assert_eq!(rows[2]["gap_to_four"], 0);
        assert_eq!(value["stale"], false);
        assert!(value["error"].is_null());
    }

    #[test]
    fn leaderboard_keeps_unmeasurable_fields_null() {
        let previous = LeaderboardHistory { updated: 1_000.0, season: Some("12".into()), standings: HashMap::new() };
        let entries = vec![
            json!({"rank":1,"bot_name":"leader","score":2_000}),
            json!({"rank":"four","bot_name":"broken","score":"many","hands_played":"lots"}),
        ];
        let value = enrich_leaderboard(&entries, &[], json!({"season_number":12}), Some(&previous), 1_299.0);
        let broken = &value["entries"][1];
        for field in [
            "rank",
            "score",
            "hands",
            "rank_delta",
            "gap_to_first",
            "gap_to_next",
            "gap_to_four",
            "score_delta",
            "score_velocity_per_hour",
            "hands_delta",
            "hands_velocity_per_hour",
        ] {
            assert!(broken[field].is_null(), "{field} should be null");
        }
        assert!(value["entries"][0]["score_velocity_per_hour"].is_null());
    }

    #[test]
    fn leaderboard_cache_transitions_preserve_last_success_on_failure() {
        let empty = LeaderboardState::empty();
        let failed = transition_leaderboard(
            &empty,
            Err("request contained secret-token and much more detail than operators need".into()),
            &[],
            100.0,
        );
        assert_eq!(failed.value["entries"], json!([]));
        assert!(failed.value["updated"].is_null());
        assert_eq!(failed.value["stale"], true);
        assert!(failed.value["error"].as_str().unwrap().len() <= 96);

        let fresh = transition_leaderboard(&empty, Ok((standings(), json!({"season_number":12}))), &["ours".into()], 200.0);
        assert_eq!(fresh.value["updated"], 200.0);
        assert_eq!(fresh.value["stale"], false);
        let stale = transition_leaderboard(&fresh, Err("upstream unavailable".into()), &["ours".into()], 500.0);
        assert_eq!(stale.value["entries"], fresh.value["entries"]);
        assert_eq!(stale.value["season"], fresh.value["season"]);
        assert_eq!(stale.value["updated"], 200.0);
        assert_eq!(stale.value["stale"], true);
        assert_eq!(stale.history.as_ref().unwrap().updated, 200.0);
    }

    #[test]
    fn leaderboard_rate_frontier_survives_short_polls_and_resets_on_season_change() {
        let empty = LeaderboardState::empty();
        let first = transition_leaderboard(&empty, Ok((standings(), json!({"season_number":12}))), &["ours".into()], 1_000.0);
        let mut changed = standings();
        changed[3]["score"] = json!(1_160);
        changed[3]["hands_played"] = json!(112);

        let short = transition_leaderboard(&first, Ok((changed.clone(), json!({"season_number":12}))), &["ours".into()], 1_060.0);
        assert_eq!(short.history.as_ref().unwrap().updated, 1_000.0, "one-minute polls must retain the rate frontier");
        assert!(short.value["entries"][3]["hands_velocity_per_hour"].is_null());

        let measurable = transition_leaderboard(&short, Ok((changed.clone(), json!({"season_number":12}))), &["ours".into()], 1_300.0);
        assert_eq!(measurable.value["entries"][3]["score_velocity_per_hour"], 720.0);
        assert_eq!(measurable.value["entries"][3]["hands_velocity_per_hour"], 144.0);
        assert_eq!(measurable.history.as_ref().unwrap().updated, 1_300.0);

        let rollover = transition_leaderboard(&measurable, Ok((standings(), json!({"season_number":13}))), &["ours".into()], 1_360.0);
        assert!(rollover.value["entries"][3]["score_delta"].is_null());
        assert!(rollover.value["entries"][3]["hands_velocity_per_hour"].is_null());
        assert_eq!(rollover.history.as_ref().unwrap().season.as_deref(), Some("13"));
        assert_eq!(rollover.history.as_ref().unwrap().updated, 1_360.0);
    }

    fn hand(bot: &str, id: &str, hole: &str, board: &str, pot: i64, net: i64, showdown: bool) -> sv10_store::store::HandRow {
        at("2026-09-17T00:00:00Z", hand_at(bot, id, hole, board, pot, net, showdown))
    }

    fn at(ended_at: &str, mut row: sv10_store::store::HandRow) -> sv10_store::store::HandRow {
        row.ended_at = ended_at.into();
        row
    }

    fn hand_at(bot: &str, id: &str, hole: &str, board: &str, pot: i64, net: i64, showdown: bool) -> sv10_store::store::HandRow {
        sv10_store::store::HandRow {
            bot: bot.into(),
            hand_id: id.into(),
            table_id: "t".into(),
            ended_at: "2026-09-17T00:00:00Z".into(),
            hero_seat: None,
            hole: hole.into(),
            board: board.into(),
            pot,
            net: Some(net),
            winners: bot.into(),
            summary: String::new(),
            showdown,
        }
    }

    #[test]
    fn highlights_reports_slams_royalty_and_locked_milestones() {
        let shared = Shared::for_test("highlights", &["b"]);
        shared.store.insert_hand(&hand("b", "h1", "AhKd", "AcAsAd2s3d", 9_000, 8_000, true)).unwrap();
        shared.store.insert_hand(&hand("b", "h2", "7c2d", "Kh9s4dQc5h", 400, 200, false)).unwrap();
        let v = highlights_blocking(&shared).0;
        assert_eq!(v["fleet_hands"], 2);
        assert_eq!(v["slams"].as_array().unwrap().len(), 1);
        assert_eq!(v["slams"][0]["hand_id"], "h1");
        let ms: HashMap<String, bool> = v["milestones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| (m["id"].as_str().unwrap().to_string(), m["unlocked"].as_bool().unwrap()))
            .collect();
        assert!(ms["royalty"], "quads win unlocks Royalty");
        assert!(ms["monster"]);
        assert!(!ms["million-club"]);
        assert!(!ms["unstoppable"]);
    }

    /// Season 13 opened 2026-09-20 11:46:23 UTC; season 12's run must not be plotted into it.
    fn season_13() -> crate::season::CurrentSeason {
        crate::season::CurrentSeason::from_current(&json!({
            "season_number": 13, "season_id": "ac9f1eb2", "start_date": "2026-09-20T11:46:23.998128+00:00"
        }))
        .unwrap()
    }

    /// One season-12 hand and two season-13 hands for bot `b`.
    fn across_the_boundary(shared: &Shared) {
        for (ts, id, net) in [
            ("2026-09-14T10:14:00+00:00", "s12", 900_000i64),
            ("2026-09-20T13:46:00+00:00", "s13a", 8_000),
            ("2026-09-20T14:58:00+00:00", "s13b", 5_000),
        ] {
            shared.store.insert_hand(&at(ts, hand_at("b", id, "AhKd", "2c3c4c5c7d", 400, net, false))).unwrap();
        }
    }

    #[test]
    fn the_fleet_race_runs_from_this_seasons_first_hand_and_keeps_the_lifetime_total() {
        let shared = Shared::for_test("fleet-season", &["b"]);
        across_the_boundary(&shared);

        let merged = fleet_blocking(&shared).0;
        assert_eq!(merged["season"]["scoped"], false, "with no known boundary nothing is dropped");
        assert_eq!(merged["bots"][0]["hands"], 3);
        assert_eq!(merged["bots"][0]["total"], 913_000);

        *shared.current_season.write() = Some(season_13());
        let v = fleet_blocking(&shared).0;
        assert_eq!(v["season"]["scoped"], true);
        assert_eq!(v["season"]["number"], 13);
        let bot = &v["bots"][0];
        assert_eq!(bot["hands"], 2, "season 12's hands are a different contest");
        assert_eq!(bot["total"], 13_000);
        assert_eq!(bot["all_time"], json!({"hands": 3, "total": 913_000}), "the lifetime run is kept, labelled");
        let points = bot["points"].as_array().unwrap();
        assert_eq!(points.len(), 2);
        assert_eq!(points[0]["hand"], 1, "hand 1 is this season's first hand");
        assert_eq!(points[0]["ts"], parse_ts("2026-09-20T13:46:00+00:00"));
        assert_eq!(points[0]["total"], 8_000);
        assert_eq!(points[1]["total"], 13_000);
    }

    #[test]
    fn range_explorer_equities_carry_a_standard_error_under_an_eighth_of_a_point() {
        assert!(equity_se(0.5, RANGE_EXPLORER_SAMPLES) < 0.00115, "worst case at 50%");
        assert_eq!(equity_se(0.0, RANGE_EXPLORER_SAMPLES), 0.0);
        assert!((equity_se(0.5, 3_000) - 0.00913).abs() < 1e-4, "the old 3,000-sample error");
    }

    #[test]
    fn a_renamed_bot_keeps_one_season_record_in_every_panel() {
        // 2026-09-23: SvanBotV7 was renamed SvanBotV10 (server kept its #1 score); hands before the
        // rename are stored as SvanBotV7.
        let shared = Shared::for_test("renamed", &["SvanBotV10"]);
        for (bot, id, ts, net) in
            [("SvanBotV7", "old1", "2026-09-21T10:00:00+00:00", 250_000i64), ("SvanBotV10", "new1", "2026-09-22T23:00:00+00:00", -2_500)]
        {
            shared.store.insert_hand(&at(ts, hand_at(bot, id, "AhKd", "2c3c4c5c7d", 400, net, false))).unwrap();
        }
        *shared.current_season.write() = Some(season_13());
        let alone = fleet_blocking(&shared).0;
        assert_eq!(alone["bots"][0]["hands"], 1, "without the alias only the new name counts");
        shared.aliases.write().insert("SvanBotV10".into(), vec!["SvanBotV10".into(), "SvanBotV7".into()]);
        let f = fleet_blocking(&shared).0;
        assert_eq!(
            (f["bots"][0]["name"].as_str(), f["bots"][0]["hands"].as_i64(), f["bots"][0]["total"].as_i64()),
            (Some("SvanBotV10"), Some(2), Some(247_500))
        );
        let h = highlights_blocking(&shared).0;
        assert_eq!((h["fleet_total"].as_i64(), h["fleet_hands"].as_i64()), (Some(247_500), Some(2)));
        let mut bot = shared.bots[0].read().clone();
        bot.slot = 90_155;
        let m = metrics(&shared, &bot);
        assert_eq!((m["net_chips"].as_i64(), m["hands"].as_i64()), (Some(247_500), Some(2)));
    }

    #[test]
    fn calibration_shows_the_stored_fits_next_to_the_shifts_in_use() {
        let shared = Shared::for_test("calibration-fits", &["b"]);
        let empty = calibration_value(&shared);
        assert_eq!(empty["fold"], Value::Null, "no fit stored yet");
        assert_eq!(empty["river_jam"], Value::Null);
        assert_eq!(empty["live"]["river_jam_call_shift"], 0.0);
        let fold = crate::foldcal::FoldCalibration { shift: [0.24, 0.0, -0.62], ..Default::default() };
        shared.store.put_kv(crate::foldcal::FOLD_CAL_KEY, &serde_json::to_string(&fold).unwrap()).unwrap();
        let jam =
            crate::raisewar::RiverJamFit { n: 882, train_shift: 0.09, held_out_saved: 39.0, held_out_lower: -75.0, ..Default::default() };
        shared.store.put_kv(crate::raisewar::RIVER_JAM_KEY, &serde_json::to_string(&jam).unwrap()).unwrap();
        shared.params.write().fold_logit_shift = [0.24, 0.0, -0.62];
        let v = calibration_value(&shared);
        assert_eq!(v["fold"]["shift"], json!([0.24, 0.0, -0.62]));
        assert_eq!(v["river_jam"]["n"], 882);
        assert_eq!(v["river_jam"]["active"], false);
        assert_eq!(v["live"]["fold_logit_shift"], json!([0.24, 0.0, -0.62]), "what the policy actually uses");
    }

    #[test]
    fn a_bots_net_winnings_tile_reads_this_season_and_keeps_the_lifetime_total() {
        let shared = Shared::for_test("metrics-season", &["b"]);
        across_the_boundary(&shared);
        *shared.current_season.write() = Some(season_13());
        // A slot no other test uses: the metrics cache is process-wide and keyed by slot.
        let mut bot = shared.bots[0].read().clone();
        bot.slot = 90_154;
        let v = metrics(&shared, &bot);
        assert_eq!(v["season"]["number"], 13);
        assert_eq!(
            (v["net_chips"].as_i64(), v["hands"].as_i64()),
            (Some(13_000), Some(2)),
            "season 12's 900k is not this season's winnings"
        );
        assert_eq!((v["all_time"]["net_chips"].as_i64(), v["all_time"]["hands"].as_i64()), (Some(913_000), Some(3)));
        let series = v["series"].as_array().unwrap();
        assert_eq!(series.last().unwrap()["total"], 13_000, "the curve starts at this season's first hand");
    }

    #[test]
    fn highlights_present_this_season_while_the_badges_stay_lifetime_achievements() {
        let shared = Shared::for_test("highlights-season", &["b"]);
        across_the_boundary(&shared);
        *shared.current_season.write() = Some(season_13());

        let v = highlights_blocking(&shared).0;
        assert_eq!(v["season"]["number"], 13);
        assert_eq!((v["fleet_total"].as_i64(), v["fleet_hands"].as_i64()), (Some(13_000), Some(2)));
        let wins: Vec<&str> = v["biggest_wins"].as_array().unwrap().iter().map(|h| h["hand_id"].as_str().unwrap()).collect();
        assert_eq!(wins, ["s13a", "s13b"], "season 12's 900k pot is not one of this season's wins");
        assert_eq!(v["best_streak"]["length"], 2, "this season's streak");
        let ms: HashMap<String, bool> = v["milestones"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| (m["id"].as_str().unwrap().to_string(), m["unlocked"].as_bool().unwrap()))
            .collect();
        // 913,000 lifetime against 13,000 this season: the badge stays earned across the rollover.
        assert!(ms["hundred-k"], "an achievement already earned survives the season rollover");
        assert!(ms["deep-stack"]);
        assert!(!ms["million-club"], "badges still read the real lifetime total, not an inflated one");
    }

    #[test]
    fn the_leak_finder_reads_every_season_but_reports_this_ones_result_separately() {
        let shared = Shared::for_test("analysis-season", &["b"]);
        let summary = HandSummary {
            players: vec![(0, "b".into())],
            button: 0,
            bb: 20,
            history: Vec::new(),
            board: Vec::new(),
            shown: Vec::new(),
            stacks: Vec::new(),
        };
        for (ts, id, net) in [("2026-09-14T10:14:00+00:00", "s12", 900_000i64), ("2026-09-20T13:46:00+00:00", "s13a", 8_000)] {
            let mut row = hand_at("b", id, "AhKd", "2c3c4c5c7d", 400, net, false);
            row.ended_at = ts.into();
            row.summary = serde_json::to_string(&summary).unwrap();
            shared.store.insert_hand(&row).unwrap();
        }
        let season = season_13();
        let report = crate::analysis::report(&shared.store, &["b".to_string()], &HashMap::new(), None, 20.0, Some(&season));
        assert_eq!(report["hands"], 2, "leaks are learning: they read every season");
        assert_eq!(report["overall"]["chips"], 908_000);
        assert_eq!(report["season"]["scoped"], true);
        assert_eq!(report["season"]["number"], 13);
        assert_eq!(report["season"]["hands"], 1);
        assert_eq!(report["season"]["chips"], 8_000);

        let unscoped = crate::analysis::report(&shared.store, &["b".to_string()], &HashMap::new(), None, 20.0, None);
        assert_eq!(unscoped["season"]["scoped"], false);
        assert_eq!(unscoped["season"]["chips"], 908_000, "no boundary means nothing is dropped, and the panel says so");
    }
}

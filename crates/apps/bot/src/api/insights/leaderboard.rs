//! Leaderboard panel JSON: standings, identity, deltas, velocity (0259).

use axum::Json;
use axum::extract::State;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use super::super::now_secs;
use crate::live::Shared;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct HistoricalStanding {
    pub(super) rank: Option<i64>,
    pub(super) score: Option<i64>,
    pub(super) hands: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct LeaderboardHistory {
    pub(super) updated: f64,
    pub(super) season: Option<String>,
    pub(super) standings: HashMap<String, HistoricalStanding>,
}

#[derive(Clone, Debug)]
pub(super) struct LeaderboardState {
    pub(super) value: Value,
    pub(super) history: Option<LeaderboardHistory>,
}

impl LeaderboardState {
    pub(super) fn empty() -> Self {
        Self {
            value: json!({"entries": [], "season": null, "updated": null, "stale": true, "error": "Leaderboard unavailable"}),
            history: None,
        }
    }
}

struct LeaderboardCache {
    at: Option<Instant>,
    state: LeaderboardState,
}

fn leaderboard_cache() -> &'static tokio::sync::Mutex<LeaderboardCache> {
    static C: OnceLock<tokio::sync::Mutex<LeaderboardCache>> = OnceLock::new();
    C.get_or_init(|| tokio::sync::Mutex::new(LeaderboardCache { at: None, state: LeaderboardState::empty() }))
}

fn season_identity(season: &Value) -> Option<String> {
    season["id"].as_str().map(str::to_owned).or_else(|| season["season_number"].as_i64().map(|number| number.to_string()))
}

pub(super) fn enrich_leaderboard(
    entries: &[Value],
    ours: &[String],
    season: Value,
    previous: Option<&LeaderboardHistory>,
    now: f64,
) -> Value {
    let current_season = season_identity(&season);
    let previous = previous.filter(|old| current_season.is_some() && old.season == current_season);
    let parsed: Vec<(String, Option<i64>, Option<i64>)> = entries
        .iter()
        .map(|entry| (entry["bot_name"].as_str().unwrap_or("").to_string(), entry["rank"].as_i64(), entry["score"].as_i64()))
        .collect();
    let scores_by_rank: HashMap<i64, i64> =
        parsed.iter().filter_map(|(_, rank, score)| Some((rank.as_ref().copied()?, score.as_ref().copied()?))).collect();
    let leader_score = scores_by_rank.get(&1).copied();
    let fourth_score = scores_by_rank.get(&4).copied();
    let elapsed_hours = previous.map(|old| (now - old.updated) / 3_600.0).filter(|hours| *hours >= 5.0 / 60.0);

    let rows = entries
        .iter()
        .zip(parsed.iter())
        .map(|(entry, (name, rank, score))| {
            let old = previous.and_then(|history| history.standings.get(name));
            let rank_delta = old.and_then(|standing| standing.rank.zip(*rank).map(|(before, current)| before - current));
            let score_delta = old.and_then(|standing| score.zip(standing.score).map(|(current, before)| current - before));
            let hands = entry["hands_played"].as_i64();
            let hands_delta = old.and_then(|standing| hands.zip(standing.hands).map(|(current, before)| current - before));
            let gap_to_first = leader_score.zip(*score).map(|(leader, current)| leader - current);
            let next_score = rank.and_then(|current| {
                scores_by_rank
                    .iter()
                    .filter(|(candidate, _)| **candidate < current)
                    .max_by_key(|(candidate, _)| *candidate)
                    .map(|(_, value)| *value)
            });
            let gap_to_next = next_score.zip(*score).map(|(next, current)| next - current);
            let gap_to_four = match rank {
                Some(value) if *value <= 4 => Some(0),
                Some(_) => fourth_score.zip(*score).map(|(fourth, current)| fourth - current),
                None => None,
            };
            let velocity = match (score_delta, elapsed_hours) {
                (Some(delta), Some(hours)) => Some(delta as f64 / hours),
                _ => None,
            };
            let hands_velocity = match (hands_delta, elapsed_hours) {
                (Some(delta), Some(hours)) => Some(delta as f64 / hours),
                _ => None,
            };
            json!({
                "rank": rank, "name": name, "score": score, "hands": hands,
                "win_rate": entry["win_rate"].as_f64(), "pro": entry["pro"].as_bool(), "ours": ours.contains(name),
                "rank_delta": rank_delta, "gap_to_first": gap_to_first, "gap_to_next": gap_to_next,
                "gap_to_four": gap_to_four, "score_delta": score_delta, "score_velocity_per_hour": velocity,
                "hands_delta": hands_delta, "hands_velocity_per_hour": hands_velocity,
            })
        })
        .collect::<Vec<_>>();
    json!({"entries": rows, "season": season, "updated": now, "stale": false, "error": null})
}

fn leaderboard_history(entries: &[Value], season: &Value, now: f64) -> LeaderboardHistory {
    LeaderboardHistory {
        updated: now,
        season: season_identity(season),
        standings: entries
            .iter()
            .filter_map(|entry| {
                let name = entry["bot_name"].as_str()?.to_string();
                Some((
                    name,
                    HistoricalStanding {
                        rank: entry["rank"].as_i64(),
                        score: entry["score"].as_i64(),
                        hands: entry["hands_played"].as_i64(),
                    },
                ))
            })
            .collect(),
    }
}

pub(super) fn transition_leaderboard(
    prior: &LeaderboardState,
    refresh: Result<(Vec<Value>, Value), String>,
    ours: &[String],
    now: f64,
) -> LeaderboardState {
    match refresh {
        Ok((entries, season)) => {
            let value = enrich_leaderboard(&entries, ours, season, prior.history.as_ref(), now);
            let candidate = leaderboard_history(&entries, &value["season"], now);
            // Keep a comparison frontier long enough to measure a useful rate. Refresh it after
            // five minutes, or immediately at a season boundary so rates never cross seasons.
            let history = prior
                .history
                .as_ref()
                .filter(|old| old.season.is_some() && old.season == candidate.season && now - old.updated < 300.0)
                .cloned()
                .unwrap_or(candidate);
            LeaderboardState { value, history: Some(history) }
        }
        Err(_) => {
            let mut value = prior.value.clone();
            value["stale"] = json!(true);
            value["error"] = json!("Leaderboard refresh failed");
            LeaderboardState { value, history: prior.history.clone() }
        }
    }
}

/// Public season leaderboard (cached 60s) with our bots flagged and rank movement.
/// A player's row in the last fetched leaderboard (rank, score, hands, gaps), if listed (0217).
pub(in crate::api) async fn leaderboard_entry(name: &str) -> Option<Value> {
    let cache = leaderboard_cache().lock().await;
    cache.state.value["entries"].as_array()?.iter().find(|e| e["name"].as_str() == Some(name)).cloned()
}

pub(in crate::api) async fn leaderboard(State(s): State<Arc<Shared>>) -> Json<Value> {
    let mut cache = leaderboard_cache().lock().await;
    if cache.at.map(|t| t.elapsed() < Duration::from_secs(60)).unwrap_or(false) {
        return Json(cache.state.value.clone());
    }
    let url = format!("{}/season/leaderboard?min_hands=10&limit=200", s.config.rest_base);
    let ours: Vec<String> = s.bots.iter().map(|b| b.read().name.clone()).collect();
    let refresh = async {
        let http = reqwest::Client::builder()
            .user_agent(crate::USER_AGENT)
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| "client setup failed".to_string())?;
        let season = http
            .get(format!("{}/season/current", s.config.rest_base))
            .send()
            .await
            .map_err(|_| "season request failed".to_string())?
            .error_for_status()
            .map_err(|_| "season status failed".to_string())?
            .json::<Value>()
            .await
            .map_err(|_| "season JSON failed".to_string())?;
        if !season.is_object() {
            return Err("season shape failed".to_string());
        }
        let entries = http
            .get(&url)
            .send()
            .await
            .map_err(|_| "leaderboard request failed".to_string())?
            .error_for_status()
            .map_err(|_| "leaderboard status failed".to_string())?
            .json::<Vec<Value>>()
            .await
            .map_err(|_| "leaderboard JSON failed".to_string())?;
        // Every ranked bot's avatar, for the rivals cards (0180).
        let mut avatars = s.avatars.write();
        for e in &entries {
            if let (Some(name), Some(url)) = (e["bot_name"].as_str(), e["avatar_url"].as_str()) {
                avatars.insert(name.to_string(), url.to_string());
            }
        }
        drop(avatars);
        Ok((entries, season))
    }
    .await;
    cache.state = transition_leaderboard(&cache.state, refresh, &ours, now_secs());
    cache.at = Some(Instant::now());
    Json(cache.state.value.clone())
}

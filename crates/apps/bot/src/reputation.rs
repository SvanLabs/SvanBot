//! Opponent reputation from every past and current season leaderboard, keyed by the
//! stable agent id (bots rename) and indexed by every name the agent has used.

use crate::live::Shared;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SeasonFinish {
    pub season: i64,
    pub rank: i64,
    pub participants: i64,
    pub score: i64,
    pub hands: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Reputation {
    pub agent_id: String,
    pub names: Vec<String>,
    pub finishes: Vec<SeasonFinish>,
    pub seasons: usize,
    pub best_rank: i64,
    pub top10: usize,
    pub lifetime_hands: i64,
    /// Hands-weighted mean of finishing percentile (1.0 = always first).
    pub strength: f64,
    /// Rank on the active season's leaderboard (0 = not on it).
    #[serde(default)]
    pub current_rank: i64,
    /// Hands played in the active season (ranks mean little early in a season).
    #[serde(default)]
    pub current_hands: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReputationBook {
    pub by_name: HashMap<String, Reputation>,
    pub updated: f64,
}

pub const KEY: &str = "reputation.v1";

impl ReputationBook {
    pub fn get(&self, name: &str) -> Option<&Reputation> {
        self.by_name.get(name)
    }
}

async fn get_json(http: &reqwest::Client, url: &str) -> Option<Value> {
    for attempt in 0..3 {
        match http.get(url).send().await {
            Ok(r) if r.status().is_success() => return r.json().await.ok(),
            Ok(r) if r.status().as_u16() == 429 => tokio::time::sleep(Duration::from_secs(20 * (attempt + 1))).await,
            _ => tokio::time::sleep(Duration::from_secs(3)).await,
        }
    }
    None
}

/// Rebuild the reputation book. Ended seasons are frozen, so their leaderboards are fetched
/// once and cached in the database.
pub async fn refresh(shared: &Shared) -> Option<ReputationBook> {
    let http = reqwest::Client::builder().user_agent(crate::USER_AGENT).timeout(Duration::from_secs(30)).build().ok()?;
    let base = &shared.config.rest_base;
    let seasons = get_json(&http, &format!("{base}/season/list")).await?;
    let mut agents: HashMap<String, Reputation> = HashMap::new();
    for season in seasons.as_array().into_iter().flatten() {
        let Some(id) = season["season_id"].as_str() else { continue };
        let number = season["season_number"].as_i64().unwrap_or(0);
        let ended = season["status"].as_str() != Some("active");
        let cache_key = format!("season.lb.{id}");
        let board = match shared.store.get_kv(&cache_key).ok().flatten().filter(|_| ended) {
            Some(cached) => serde_json::from_str::<Value>(&cached).ok(),
            None => {
                let url = if ended {
                    format!("{base}/season/{id}/leaderboard?limit=1000")
                } else {
                    format!("{base}/season/leaderboard?limit=1000")
                };
                let v = get_json(&http, &url).await;
                if let (Some(v), true) = (&v, ended) {
                    // Best-effort, and the one place in this file where that is the right answer: the
                    // season has ended, so the cached leaderboard cannot change again, a miss costs one
                    // re-fetch of the same fixed content, and the refresh holding the fresh value must
                    // not fail over its copy (issue #326).
                    let _ = shared.store.put_kv(&cache_key, &v.to_string());
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
                v
            }
        };
        let Some(entries) = board.as_ref().and_then(|b| b.as_array()) else { continue };
        let participants = entries.len() as i64;
        for e in entries {
            let agent = e["agent_id"].as_str().or(e["bot_name"].as_str()).unwrap_or("").to_string();
            if agent.is_empty() {
                continue;
            }
            let rep = agents.entry(agent.clone()).or_insert_with(|| Reputation { agent_id: agent.clone(), ..Default::default() });
            for n in [e["bot_name"].as_str(), e["public_profile_slug"].as_str()].into_iter().flatten() {
                if !rep.names.iter().any(|x| x.eq_ignore_ascii_case(n)) {
                    rep.names.push(n.to_string());
                }
            }
            if !ended {
                rep.current_rank = e["rank"].as_i64().unwrap_or(0);
                rep.current_hands = e["hands_played"].as_i64().unwrap_or(0);
            }
            rep.finishes.push(SeasonFinish {
                season: number,
                rank: e["rank"].as_i64().unwrap_or(participants),
                participants,
                score: e["score"].as_i64().unwrap_or(0),
                hands: e["hands_played"].as_i64().unwrap_or(0),
            });
        }
    }
    let mut book = ReputationBook { updated: chrono::Utc::now().timestamp() as f64, ..Default::default() };
    for (_, mut rep) in agents {
        rep.finishes.sort_by_key(|f| std::cmp::Reverse(f.season));
        rep.seasons = rep.finishes.len();
        rep.best_rank = rep.finishes.iter().map(|f| f.rank).min().unwrap_or(0);
        rep.top10 = rep.finishes.iter().filter(|f| f.rank <= 10).count();
        rep.lifetime_hands = rep.finishes.iter().map(|f| f.hands).sum();
        let (mut num, mut den) = (0.0, 0.0);
        for f in &rep.finishes {
            let w = (f.hands.max(1) as f64).sqrt();
            num += w * (1.0 - (f.rank - 1) as f64 / f.participants.max(1) as f64);
            den += w;
        }
        rep.strength = if den > 0.0 { num / den } else { 0.5 };
        for n in rep.names.clone() {
            book.by_name.insert(n.clone(), rep.clone());
            book.by_name.insert(n.to_lowercase(), rep.clone());
        }
    }
    let json = serde_json::to_string(&book).ok()?;
    if let Err(e) = shared.store.put_kv(KEY, &json) {
        tracing::warn!("the reputation book was not stored, so the next start re-reads every season's leaderboard ({e})");
    }
    Some(book)
}

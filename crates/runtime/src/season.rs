//! The season board the dashboard shows (V11-44), from the venue's public season endpoints
//! (<https://docs.openpoker.ai/llms-full.txt>, rev 2026-09-02, "Season"): the season's dates from
//! `GET /season/current`, and the leaderboard from `GET /season/leaderboard`.
//!
//! Both are public and need no key, so the dashboard reads them with [`Rest::public`]. The board
//! keeps the top of the leaderboard and every bot of ours wherever it ranks, so the page can show
//! the race at the top and each bot's own place without the whole list.

use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::limit::Limiter;
use crate::rest::{API, Rest, RestError};

/// The season's dates and state.
pub const CURRENT: &str = "/season/current";

/// The leaderboard as the official display counts it: at least 10 hands, best score first, the
/// whole board in one reply (the endpoint's limit is 1,000).
pub const LEADERBOARD: &str = "/season/leaderboard?sort_by=score&min_hands=10&limit=1000";

/// The entries kept from the top of the board.
pub const TOP: usize = 20;

/// The leaderboard's limit is 30 a minute per address; the dashboard reads both endpoints once a
/// minute, far under it, and the limiter holds it there whatever the caller does.
pub fn client() -> Rest {
    Rest::public(API, Limiter::new(10, Duration::from_secs(60)))
}

/// Reads the season and its leaderboard from the venue, and shapes them with [`board`].
pub fn read(rest: &mut Rest, ours: &[String]) -> Result<Value, String> {
    let fetch = |rest: &mut Rest, path: &str| {
        rest.get(path)
            .map_err(|why: RestError| format!("{path}: {why}"))
    };
    let current = fetch(rest, CURRENT)?;
    let leaderboard = fetch(rest, LEADERBOARD)?;
    board(&current, &leaderboard, ours, TOP)
}

/// The board the page reads: the season's number, end and time left, how many are ranked, the
/// leader's score, and the rows: the first `top` of `leaderboard`, then each of `ours` ranked
/// below them. A reply that is not the venue's shape is an error, never an empty board.
pub fn board(
    current: &Value,
    leaderboard: &Value,
    ours: &[String],
    top: usize,
) -> Result<Value, String> {
    let number = current["season_number"]
        .as_u64()
        .ok_or("the current season has no season_number")?;
    let entries = leaderboard
        .as_array()
        .ok_or("the leaderboard is not a list")?;
    let mut rows = Vec::new();
    for (at, entry) in entries.iter().enumerate() {
        let row = standing(entry)?;
        let mine = ours.iter().any(|name| row["name"] == name.as_str());
        if at < top || mine {
            let mut row = row;
            row.insert("ours".to_owned(), Value::Bool(mine));
            rows.push(Value::Object(row));
        }
    }
    let leader = entries
        .first()
        .and_then(|entry| entry["score"].as_i64())
        .unwrap_or(0);
    Ok(json!({
        "number": number,
        "ends": current["end_date"].as_str(),
        "remaining_s": current["time_remaining_seconds"].as_f64(),
        "registered": current["total_registered"].as_u64(),
        "winding_down": current["winding_down"].as_bool().unwrap_or(false),
        "ranked": entries.len(),
        "leader": leader,
        "rows": rows,
    }))
}

/// One leaderboard entry in the page's names, or why it is not one.
fn standing(entry: &Value) -> Result<Map<String, Value>, String> {
    let whole = |field: &str| {
        entry[field]
            .as_u64()
            .ok_or_else(|| format!("a leaderboard entry has no {field}"))
    };
    let name = entry["bot_name"]
        .as_str()
        .ok_or("a leaderboard entry has no bot_name")?;
    let score = entry["score"]
        .as_i64()
        .ok_or("a leaderboard entry has no score")?;
    let mut row = Map::new();
    row.insert("rank".to_owned(), json!(whole("rank")?));
    row.insert("name".to_owned(), json!(name));
    row.insert("score".to_owned(), json!(score));
    row.insert("hands".to_owned(), json!(whole("hands_played")?));
    row.insert("won".to_owned(), json!(whole("hands_won")?));
    row.insert("rebuys".to_owned(), json!(whole("rebuys")?));
    Ok(row)
}

//! Badge race (0182): where each of our bots stands on the three public leaderboard sorts, and what
//! it takes to reach the season-end rewards.
//!
//! Platform rules (llms-full.txt rev 2026-09-02, 0175): the top 3 by score earn permanent
//! Gold/Silver/Bronze badges (10-hand minimum), prizes go to the top 30, and season prizes are
//! limited to one winning bot per owner. `win_rate` is the share of hands won (`hands_won /
//! hands_played`), not chips; ranks are read from the server's own sorts, never recomputed.

use serde_json::{Value, json};

/// Rank that still earns a badge.
pub const BADGE_RANKS: u64 = 3;
/// Last prize rank.
pub const PRIZE_RANKS: u64 = 30;

fn rank_of(entries: &[Value], name: &str) -> Option<u64> {
    entries.iter().find(|e| e["bot_name"].as_str() == Some(name)).and_then(|e| e["rank"].as_u64())
}

fn score_at(entries: &[Value], rank: u64) -> Option<i64> {
    entries.iter().find(|e| e["rank"].as_u64() == Some(rank)).and_then(|e| e["score"].as_i64())
}

/// The race for `ours`, from the `score`, `win_rate` and `hands_played` sorts.
pub fn badge_race(by_score: &[Value], by_win_rate: &[Value], by_hands: &[Value], ours: &[String]) -> Value {
    let bronze = score_at(by_score, BADGE_RANKS);
    let prize = score_at(by_score, PRIZE_RANKS);
    let bots: Vec<Value> = ours
        .iter()
        .map(|name| {
            let entry = by_score.iter().find(|e| e["bot_name"].as_str() == Some(name.as_str()));
            let score = entry.and_then(|e| e["score"].as_i64());
            let rank = rank_of(by_score, name);
            let medal = match rank {
                Some(1) => Some("gold"),
                Some(2) => Some("silver"),
                Some(3) => Some("bronze"),
                _ => None,
            };
            // Points still needed to pass the current holder of a line (0 once on or above it).
            let gap = |line: Option<i64>, ranks: u64| match (score, line, rank) {
                (_, _, Some(r)) if r <= ranks => Some(0),
                (Some(s), Some(l), _) => Some((l - s + 1).max(0)),
                _ => None,
            };
            json!({
                "name": name,
                "score": score,
                "hands_played": entry.and_then(|e| e["hands_played"].as_u64()),
                "win_rate": entry.and_then(|e| e["win_rate"].as_f64()),
                "rank_score": rank,
                "rank_win_rate": rank_of(by_win_rate, name),
                "rank_hands_played": rank_of(by_hands, name),
                "medal": medal,
                "to_badge": gap(bronze, BADGE_RANKS),
                "to_prize": gap(prize, PRIZE_RANKS),
            })
        })
        .collect();
    json!({"bots": bots, "bronze_score": bronze, "prize_line_score": prize, "badge_ranks": BADGE_RANKS, "prize_ranks": PRIZE_RANKS})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(rank: u64, name: &str, score: i64) -> Value {
        json!({"rank": rank, "bot_name": name, "score": score, "hands_played": 4000, "win_rate": 0.49})
    }

    #[test]
    fn race_reports_medals_ranks_and_gaps() {
        let mut score: Vec<Value> = vec![entry(1, "SvanBotV10", 282_836), entry(2, "Svanar", 262_946), entry(3, "Svanism", 232_592)];
        score.extend((4..=40).map(|r| entry(r, &format!("bot{r}"), 200_000 - r as i64 * 1_000)));
        score.push(entry(41, "SurSvan", 134_381));
        let win = vec![entry(1, "SvanBotV10", 0), entry(2, "Svanism", 0)];
        let hands = vec![entry(7, "SurSvan", 0)];
        let ours = ["SvanBotV10", "Svanism", "SurSvan", "Ghost"].map(String::from);
        let r = badge_race(&score, &win, &hands, &ours);
        let b = &r["bots"];
        assert_eq!((b[0]["medal"].as_str(), b[0]["to_badge"].as_i64()), (Some("gold"), Some(0)));
        assert_eq!((b[1]["medal"].as_str(), b[1]["rank_win_rate"].as_u64()), (Some("bronze"), Some(2)));
        // SurSvan (41st) needs to pass bronze (232,592) and the 30th (170,000) by one point.
        assert_eq!(b[2]["to_badge"].as_i64(), Some(232_592 - 134_381 + 1));
        assert_eq!(b[2]["to_prize"].as_i64(), Some(170_000 - 134_381 + 1));
        assert_eq!(b[2]["rank_hands_played"].as_u64(), Some(7));
        // A bot missing from the board has no rank and no gap.
        assert!(b[3]["rank_score"].is_null() && b[3]["to_badge"].is_null());
        assert_eq!(r["prize_line_score"].as_i64(), Some(170_000));
    }
}

//! The season board the dashboard shows, shaped from the venue's public replies
//! (<https://docs.openpoker.ai/llms-full.txt>, rev 2026-09-02, "Season": `GET /season/current`
//! and `GET /season/leaderboard`).

use serde_json::{Value, json};
use svanbot_runtime::season::{LEADERBOARD, board};

/// The docs' own example of `GET /season/current`.
fn current() -> Value {
    json!({
        "season_id": "550e8400-...",
        "season_number": 1,
        "start_date": "2026-03-17T00:00:00+00:00",
        "end_date": "2026-03-31T00:00:00+00:00",
        "status": "active",
        "time_remaining_seconds": 864000.0,
        "winding_down": false,
        "total_registered": 42
    })
}

/// One leaderboard row in the docs' shape.
fn row(rank: u64, name: &str, score: i64, hands: u64) -> Value {
    json!({
        "rank": rank,
        "bot_name": name,
        "score": score,
        "chip_balance": score - 2000,
        "chips_at_table": 2000,
        "rebuys": 1,
        "hands_played": hands,
        "hands_won": hands / 4,
        "win_rate": 0.25,
        "pro": false
    })
}

#[test]
fn the_board_is_the_top_of_the_leaderboard_and_every_bot_of_ours_wherever_it_ranks() {
    let leaderboard = json!([
        row(1, "SharpAce42", 11000, 347),
        row(2, "SurSvan", 9000, 300),
        row(3, "<b>rival</b>", 8000, 200),
        row(4, "filler", 7000, 100),
        row(5, "Svanar", 6000, 90),
    ]);
    let ours = ["SurSvan".to_owned(), "Svanar".to_owned(), "Unranked".to_owned()];
    let board = board(&current(), &leaderboard, &ours, 3).unwrap();
    assert_eq!(board["number"], 1);
    assert_eq!(board["ends"], "2026-03-31T00:00:00+00:00");
    assert_eq!(board["remaining_s"], 864000.0);
    assert_eq!(board["registered"], 42);
    assert_eq!(board["winding_down"], false);
    // Every ranked entry counts, shown or not, and the leader's score is the bar's full length.
    assert_eq!(board["ranked"], 5);
    assert_eq!(board["leader"], 11000);
    let rows: Vec<(u64, &str, bool)> = board["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["rank"].as_u64().unwrap(),
                row["name"].as_str().unwrap(),
                row["ours"].as_bool().unwrap(),
            )
        })
        .collect();
    // The top three, then Svanar below the cut; a bot of ours with no rank is no row.
    assert_eq!(
        rows,
        [
            (1, "SharpAce42", false),
            (2, "SurSvan", true),
            (3, "<b>rival</b>", false),
            (5, "Svanar", true)
        ]
    );
    let second = &board["rows"][1];
    assert_eq!(second["score"], 9000);
    assert_eq!(second["hands"], 300);
    assert_eq!(second["won"], 75);
    assert_eq!(second["rebuys"], 1);
}

#[test]
fn a_reply_that_is_not_the_venues_shape_is_an_error_not_an_empty_board() {
    assert!(board(&json!({"detail": "down"}), &json!([]), &[], 10).is_err());
    assert!(board(&current(), &json!({"detail": "slow down"}), &[], 10).is_err());
    let broken = json!([{"rank": 1, "score": 5}]);
    let why = board(&current(), &broken, &[], 10).unwrap_err();
    assert!(why.contains("bot_name"), "{why}");
}

#[test]
fn the_leaderboard_is_read_as_the_official_display_counts_it() {
    // At least 10 hands, as prizes and the official display count, and the whole board at once.
    assert_eq!(
        LEADERBOARD,
        "/season/leaderboard?sort_by=score&min_hands=10&limit=1000"
    );
}

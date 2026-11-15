//! A hand two of our seats both sat in is stored once per seat under one `hand_id`; the live-hand
//! read the response model trains and validates on must hand it over once.

use super::*;
use sv10_store::store::{HandRow, Store};

fn row(bot: &str, id: &str, ended: &str) -> HandRow {
    let summary = HandSummary {
        players: vec![(0, "A".into()), (1, "B".into())],
        button: 0,
        bb: 20,
        history: vec![],
        board: vec![],
        shown: vec![],
        stacks: vec![(0, 2_000), (1, 2_000)],
    };
    HandRow {
        bot: bot.into(),
        hand_id: id.into(),
        ended_at: ended.into(),
        summary: serde_json::to_string(&summary).unwrap(),
        ..Default::default()
    }
}

#[test]
fn a_hand_both_seats_stored_is_one_live_hand() {
    let dir = std::env::temp_dir().join(format!("sv10-neural-shared-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir.join("t.db")).unwrap();
    store.insert_hand(&row("A", "h1", "2026-01-01T00:00:00Z")).unwrap();
    store.insert_hand(&row("B", "h1", "2026-01-01T00:00:00Z")).unwrap();
    store.insert_hand(&row("B", "h2", "2026-01-01T00:01:00Z")).unwrap();
    let ids: Vec<String> = live_hands(&store, &["A".into(), "B".into()]).unwrap().into_iter().map(|h| h.hand_id).collect();
    assert_eq!(ids, ["h1", "h2"]);
}

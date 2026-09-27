//! Offline replay of recorded frame streams through the live parsing.

use super::*;

/// Rebuild finished hands from a recorded frame stream (captures, archived observation
/// databases) with exactly the parsing live play uses. Returns each hand with its result frame's
/// `ts`. Duplicate or regressed frames are dropped, as live.
pub fn replay<'a>(frames: impl IntoIterator<Item = &'a Value>) -> Vec<(Option<String>, FinishedHand)> {
    let mut t = TableTracker::default();
    t.reset_table();
    let mut out = Vec::new();
    for m in frames {
        // Sequence numbers are per table: a stream that moves tables without a `table_joined`
        // frame starts a fresh namespace.
        if let Some(tid) = m["table_id"].as_str()
            && t.table_id.as_deref().is_some_and(|cur| cur != tid)
        {
            t.reset_table();
        }
        if !t.accept_seq(m) {
            continue;
        }
        match m["type"].as_str().unwrap_or("") {
            "table_joined" => t.table_joined(m),
            "table_state" => t.table_state(m),
            "hand_start" => t.hand_start(m),
            "hole_cards" => t.hole_cards(m),
            "player_action" => t.player_action(m),
            "community_cards" => t.community_cards(m),
            "hand_result" => {
                if let Some(f) = t.hand_result(m) {
                    out.push((m["ts"].as_str().map(String::from), f));
                }
            }
            _ => {}
        }
    }
    out
}

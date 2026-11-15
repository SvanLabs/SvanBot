//! A seated bot can watch a hand before it is dealt in; that is not one of its played hands.

use super::*;

#[tokio::test]
async fn a_waiting_hero_does_not_store_or_count_another_players_hand() {
    let mut rig = Rig::new("waiting-result");
    let (tx, _rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.feed(
        &conn,
        json!({"type": "table_joined", "table_id": "t", "seat": 2,
        "players": [{"seat": 2, "name": "A", "stack": 5000}, {"seat": 4, "name": "B", "stack": 2000},
        {"seat": 5, "name": "C", "stack": 2000}]}),
    )
    .await;
    rig.feed(
        &conn,
        json!({"type": "table_state", "table_id": "t", "hand_id": "theirs", "street": "preflop",
        "dealer_seat": 4, "pot": 30, "board": [], "hero": {"seat": 2},
        "seats": [{"seat": 2, "name": "A", "stack": 5000, "bet": 0, "status": "waiting", "in_hand": false},
        {"seat": 4, "name": "B", "stack": 1990, "bet": 10, "status": "active", "in_hand": true},
        {"seat": 5, "name": "C", "stack": 1980, "bet": 20, "status": "active", "in_hand": true}]}),
    )
    .await;
    rig.feed(
        &conn,
        json!({"type": "hand_result", "hand_id": "theirs", "total_pot": 30,
        "final_stacks": {"4": 1990, "5": 2010}, "winners": [{"seat": 5, "name": "C"}]}),
    )
    .await;
    assert!(rig.shared.store.hand("A", "theirs").unwrap().is_none(), "a hand the hero did not play is not its row");
    assert_eq!(rig.shared.bots[0].read().session_hands, 0, "watching is not playing");
    assert!(rig.shared.models.read().watermark.is_none(), "the fleet image must not learn a phantom own hand");
    assert_eq!(rig.tracker.street, None, "the table result still updates the tracker");
    assert_eq!(rig.tracker.seats[&5].stack, 2010, "including other players' stacks");

    // A hero that was dealt in still counts even if its result cannot yet be priced.
    rig.feed(
        &conn,
        json!({"type": "table_state", "table_id": "t", "hand_id": "ours", "street": "preflop",
        "dealer_seat": 4, "pot": 30, "board": [], "hero": {"seat": 2},
        "seats": [{"seat": 2, "name": "A", "stack": 4980, "bet": 20, "status": "active", "in_hand": true},
        {"seat": 4, "name": "B", "stack": 1980, "bet": 10, "status": "active", "in_hand": true}]}),
    )
    .await;
    rig.feed(&conn, json!({"type": "hand_result", "hand_id": "ours", "total_pot": 30, "final_stacks": {"4": 2010}})).await;
    assert_eq!(rig.shared.store.hand("A", "ours").unwrap().expect("a dealt hand is preserved").net, None);
    assert_eq!(rig.shared.bots[0].read().session_hands, 1);
}

#[tokio::test]
async fn private_cards_preserve_a_played_hand_when_its_snapshot_players_are_missing() {
    let mut rig = Rig::new("cards-without-snapshot");
    let (tx, _rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.feed(&conn, json!({"type": "hand_start", "table_id": "t", "hand_id": "ours", "seat": 2})).await;
    rig.feed(&conn, json!({"type": "hole_cards", "cards": ["As", "Kd"]})).await;
    rig.feed(&conn, json!({"type": "hand_result", "hand_id": "ours", "total_pot": 30, "final_stacks": {"2": 5030}})).await;
    let row = rig.shared.store.hand("A", "ours").unwrap().expect("private cards prove the hero played");
    assert_eq!(row.hole, "AsKd");
    assert_eq!(rig.shared.bots[0].read().session_hands, 1);
}

#[tokio::test]
async fn an_absurd_rebuy_cooldown_does_not_end_the_session() {
    // #945: Instant + Duration panics on overflow, so a cooldown the server never sends ended this bot's
    // session, and its pending turn with it.
    let mut rig = Rig::new("rebuy-overflow");
    let (tx, _rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.feed(&conn, json!({"type": "auto_rebuy_scheduled", "cooldown_seconds": u64::MAX})).await;
}

/// Spec (reconnection-idempotency, recovery loop guard): only a *player* resync ends recovery. After an
/// outage past the 120 s seat window the server answers our resync as a spectator; the bot used to
/// install that snapshot, call itself playing and wait for turns that never come. It rejoins instead.
#[tokio::test]
async fn a_resync_answered_as_a_spectator_sends_the_bot_back_to_the_lobby() {
    let mut rig = Rig::new("spectator-resync");
    let (tx, _rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.feed(&conn, json!({"type": "table_joined", "table_id": "t1", "seat": 2, "players": []})).await;
    assert!(!rig.seat.pending_join);
    rig.feed(
        &conn,
        json!({"type": "resync_response", "role": "spectator", "to_table_seq": 50, "replayed_events": [],
        "snapshot": {"type": "table_state", "hand_id": "h1", "actor_seat": 4, "street": "flop", "pot": 90, "board": [], "seats": []}}),
    )
    .await;
    assert_eq!(rig.tracker.table_id, None, "the table is no longer ours");
    assert!(rig.seat.pending_join, "and the bot re-queues");
    assert!(rig.shared.log.lock().iter().any(|l| l.message.contains("spectator")), "and says why");
    // A player resync is the normal path and changes nothing.
    rig.feed(&conn, json!({"type": "table_joined", "table_id": "t2", "seat": 2, "players": []})).await;
    rig.seat.pending_join = false;
    rig.feed(&conn, json!({"type": "resync_response", "role": "player", "to_table_seq": 3, "replayed_events": [],
        "snapshot": {"type": "table_state", "hand_id": "h2", "street": "preflop", "pot": 30, "board": [], "seats": [], "hero": {"seat": 2}}})).await;
    assert_eq!(rig.tracker.table_id.as_deref(), Some("t2"));
    assert!(!rig.seat.pending_join);
}

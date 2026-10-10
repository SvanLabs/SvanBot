//! A missing-state turn must give the snapshot a chance to restore this turn's cards.
use super::*;

pub(super) fn restored_turn(token: &str) -> Value {
    json!({"type":"resync_response", "role":"player", "replayed_events":[], "snapshot":{
        "table_id":"t1", "hand_id":"h1", "street":"preflop", "pot":30, "board":[], "big_blind":20,
        "seats":[{"seat":0,"name":"A","stack":1980,"bet":20,"in_hand":true},
                 {"seat":1,"name":"B","stack":1990,"bet":10,"in_hand":true}],
        "hero":{"seat":0,"hole_cards":["As","Kd"],"turn_token":token,"valid_actions":[{"action":"check"}]}
    }})
}

#[tokio::test]
async fn missing_state_waits_for_resync_before_answering_the_same_turn() {
    let mut rig = Rig::new("missing-state-first-recover");
    rig.shared.params.write().samples = 64;
    rig.tracker.table_id = Some("t1".into());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.feed(&conn, sequenced_turn("h1", "token1", 41)).await;
    let out: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert_eq!(out.len(), 1, "missing state requests recovery before sending any fallback: {out:?}");
    assert_eq!(out[0]["type"], "resync_request");
    rig.feed(&conn, restored_turn("token1")).await;
    let sent = actions(&mut rx);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["turn_token"], "token1");
    assert_eq!(sent[0]["hand_id"], "h1");
    assert_eq!(sent[0]["action"], "check");
    assert_eq!(rig.shared.bots[0].read().decisions, 1, "the recovered pending turn uses the real policy");
    rig.feed(&conn, restored_turn("token1")).await;
    assert!(actions(&mut rx).is_empty(), "a repeated snapshot cannot answer twice");
}

#[tokio::test]
async fn missing_state_without_a_snapshot_falls_back_once_at_the_original_deadline() {
    let mut rig = Rig::new("missing-state-deadline");
    rig.tracker.table_id = Some("t1".into());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    let mut offer = sequenced_turn("h1", "token1", 41);
    offer["valid_actions"] = json!([{"action":"fold"},{"action":"call","amount":20}]);
    rig.feed(&conn, offer.clone()).await;
    assert!(actions(&mut rx).is_empty());
    let deadline = rig.seat.turn.as_ref().unwrap().deadline;
    rig.feed(&conn, offer).await;
    assert_eq!(rig.seat.turn.as_ref().unwrap().deadline, deadline, "redelivery never extends the turn clock");
    // Advance the controlled clock at the timer callback seam without another server frame.
    turns::finish(&rig.shared, 0, &rig.bot, &mut rig.tracker, &conn, &mut rig.rng, &mut rig.seat.turn, deadline - Duration::from_millis(1))
        .await;
    assert!(actions(&mut rx).is_empty(), "the timer cannot finish before its deadline");
    turns::finish(&rig.shared, 0, &rig.bot, &mut rig.tracker, &conn, &mut rig.rng, &mut rig.seat.turn, deadline).await;
    let sent = actions(&mut rx);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["action"], "fold");
    assert_eq!(sent[0]["hand_id"], "h1");
    assert_eq!(sent[0]["turn_token"], "token1");
    assert_eq!(rig.shared.bots[0].read().decisions, 0);
    rig.feed(&conn, restored_turn("token1")).await;
    assert!(actions(&mut rx).is_empty(), "late recovery cannot re-answer the fallback");
}

#[tokio::test]
async fn a_new_snapshot_authority_supersedes_the_waiting_turn_and_rejects_late_older_turns() {
    let mut rig = Rig::new("missing-state-new-authority");
    rig.shared.params.write().samples = 64;
    rig.tracker.table_id = Some("t1".into());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.feed(&conn, sequenced_turn("h1", "old-token", 41)).await;
    assert!(actions(&mut rx).is_empty());
    let mut snapshot = restored_turn("new-token");
    snapshot["to_table_seq"] = json!(42);
    rig.feed(&conn, snapshot).await;
    let sent = actions(&mut rx);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["turn_token"], "new-token");
    rig.feed(&conn, sequenced_turn("h1", "old-token", 41)).await;
    assert!(actions(&mut rx).is_empty(), "older authority cannot act after the recovered new turn");
}

#[tokio::test]
async fn a_recovery_snapshot_with_no_legal_offer_cannot_send_an_invented_fold() {
    let mut rig = Rig::new("missing-state-empty-offer");
    rig.shared.params.write().samples = 64;
    rig.tracker.table_id = Some("t1".into());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.feed(&conn, sequenced_turn("h1", "token1", 41)).await;
    assert!(actions(&mut rx).is_empty());
    let mut snapshot = restored_turn("token1");
    snapshot["snapshot"]["hero"]["valid_actions"] = json!([]);
    rig.feed(&conn, snapshot).await;
    assert!(actions(&mut rx).is_empty(), "an empty offer authorizes no action");
    assert!(rig.seat.turn.is_none(), "the old offer cannot later be used against this snapshot");
}

#[tokio::test]
async fn older_offers_cannot_cancel_the_current_pending_authority() {
    for old_hand in ["h1", "h0"] {
        for empty in [true, false] {
            let mut rig = Rig::new(&format!("missing-state-stale-offer-{old_hand}-{empty}"));
            rig.tracker.table_id = Some("t1".into());
            let (tx, mut rx) = mpsc::unbounded_channel();
            let conn = Conn { out: tx };
            rig.feed(&conn, sequenced_turn("h1", "new-token", 42)).await;
            assert!(actions(&mut rx).is_empty());
            let deadline = rig.seat.turn.as_ref().unwrap().deadline;
            let mut stale = sequenced_turn(old_hand, "old-token", 41);
            if empty {
                stale["valid_actions"] = json!([]);
            }
            rig.feed(&conn, stale).await;
            assert_eq!(rig.seat.turn.as_ref().map(|turn| turn.deadline), Some(deadline), "obsolete offers cannot cancel current authority");
            turns::finish(&rig.shared, 0, &rig.bot, &mut rig.tracker, &conn, &mut rig.rng, &mut rig.seat.turn, deadline).await;
            let sent = actions(&mut rx);
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0]["turn_token"], "new-token");
        }
    }
}

#[tokio::test]
async fn missing_state_authority_is_cancelled_when_the_hand_or_seat_is_gone() {
    let frames = [
        json!({"type":"table_closed","table_id":"t1"}),
        json!({"type":"hand_start","table_id":"t1","hand_id":"h2"}),
        json!({"type":"hand_result","hand_id":"h1","winners":[],"final_stacks":{}}),
        json!({"type":"resync_response","role":"spectator","snapshot":{}}),
        json!({"type":"resync_response","role":"player","snapshot":null}),
        json!({"type":"resync_response","role":"player","snapshot":{"hand_id":"h1","hero":{"seat":0}}}),
        json!({"type":"player_action","hand_id":"h1","seat":0,"action":"fold"}),
    ];
    for (index, frame) in frames.into_iter().enumerate() {
        let mut rig = Rig::new(&format!("missing-state-cancel-{index}"));
        rig.tracker.table_id = Some("t1".into());
        rig.tracker.hand_id = Some("h1".into());
        rig.tracker.hero_seat = Some(0);
        let (tx, mut rx) = mpsc::unbounded_channel();
        let conn = Conn { out: tx };
        rig.feed(&conn, sequenced_turn("h1", "token1", 41)).await;
        assert!(actions(&mut rx).is_empty());
        rig.feed(&conn, frame).await;
        turns::finish(
            &rig.shared,
            0,
            &rig.bot,
            &mut rig.tracker,
            &conn,
            &mut rig.rng,
            &mut rig.seat.turn,
            Instant::now() + Duration::from_secs(3),
        )
        .await;
        assert!(rig.seat.turn.is_none(), "ended authority stayed pending in case {index}");
        assert!(actions(&mut rx).is_empty(), "ended authority acted in case {index}");
    }
}

#[tokio::test]
async fn a_missing_state_fallback_never_folds_when_the_offer_only_allows_calling() {
    let mut rig = Rig::new("missing-state-no-passive-offer");
    rig.tracker.table_id = Some("t1".into());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    let mut offer = sequenced_turn("h1", "token1", 41);
    offer["valid_actions"] = json!([{"action":"call","amount":20}]);
    rig.feed(&conn, offer).await;
    assert!(actions(&mut rx).is_empty());
    turns::finish(
        &rig.shared,
        0,
        &rig.bot,
        &mut rig.tracker,
        &conn,
        &mut rig.rng,
        &mut rig.seat.turn,
        Instant::now() + Duration::from_secs(3),
    )
    .await;
    assert!(actions(&mut rx).is_empty(), "no passive action was authorized, so a fallback cannot invent fold");
}

//! Hands the fleet did not finish: the ones a restart or a dropped socket left in progress (0315),
//! settled from the resync replay rather than lost, per bot and across processes.

use super::*;

#[tokio::test]
async fn missing_state_turns_resync_once_per_hand_without_double_acting() {
    let mut rig = Rig::new("missing-state-resync");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.tracker.table_id = Some("t1".into());
    rig.tracker.last_table_seq = 40;
    assert!(rig.tracker.hole.is_none(), "the fixture has no decision state");
    rig.feed(&conn, sequenced_turn("h1", "t1", 41)).await;
    let drain = |rx: &mut mpsc::UnboundedReceiver<Value>| std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
    let out = drain(&mut rx);
    assert_eq!(out.len(), 1, "resync goes first while authority waits: {out:?}");
    assert_eq!(out[0]["type"], "resync_request");
    assert!(
        out.iter().any(|v| v["type"] == "resync_request" && v["table_id"] == "t1" && v["last_table_seq"] == 41),
        "missing state must recover: {out:?}"
    );
    rig.feed(&conn, sequenced_turn("h1", "t1", 41)).await;
    assert!(drain(&mut rx).is_empty(), "duplicate authority sends neither action nor recovery");
    rig.feed(
        &conn,
        json!({"type": "resync_response", "replayed_events": [], "snapshot": {
            "hand_id": "h1", "pot": 0, "board": [], "seats": [],
            "hero": {"seat": 0, "turn_token": "t1", "valid_actions": [{"action": "check"}]}
        }}),
    )
    .await;
    assert!(drain(&mut rx).is_empty(), "same-token incomplete snapshot cannot double-act or resync-loop");
    rig.feed(&conn, sequenced_turn("h1", "t2", 42)).await;
    let out = drain(&mut rx);
    assert_eq!(out.len(), 1, "a new token in the same broken hand stays bounded: {out:?}");
    assert_eq!(out[0]["type"], "action");
    rig.feed(&conn, sequenced_turn("h2", "t3", 43)).await;
    let out = drain(&mut rx);
    assert_eq!(out.iter().filter(|v| v["type"] == "resync_request").count(), 1, "a new hand can recover again: {out:?}");
}

#[tokio::test]
async fn a_resync_restores_missing_cards_for_later_turns() {
    let mut rig = Rig::new("missing-state-restored");
    rig.shared.params.write().samples = 64;
    rig.tracker.table_id = Some("t1".into());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.feed(&conn, turn("t1")).await;
    let _ = actions(&mut rx);
    rig.feed(
        &conn,
        json!({"type": "resync_response", "role": "player", "replayed_events": [], "snapshot": {
            "table_id": "t1", "hand_id": "h1", "street": "preflop", "pot": 30, "board": [], "big_blind": 20,
            "seats": [{"seat": 0, "name": "A", "stack": 1980, "bet": 20, "in_hand": true},
                      {"seat": 1, "name": "B", "stack": 1990, "bet": 10, "in_hand": true}],
            "hero": {"seat": 0, "hole_cards": ["As", "Kd"], "turn_token": "t1", "valid_actions": [{"action": "check"}]}
        }}),
    )
    .await;
    assert_eq!(actions(&mut rx).len(), 1, "the recovered waiting token is answered");
    assert!(rig.tracker.hole.is_some());
    let mut next = turn("t2");
    next["valid_actions"] = json!([{"action": "check"}]);
    rig.feed(&conn, next).await;
    let out = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(out.len(), 1, "healthy state needs no new recovery request: {out:?}");
    assert_eq!(out[0]["type"], "action");
    assert_eq!(rig.shared.bots[0].read().decisions, 2, "both the recovered and later turn ran the real policy");
}

#[tokio::test]
async fn a_failed_writer_does_not_consume_missing_state_recovery() {
    let mut rig = Rig::new("missing-state-writer");
    rig.tracker.table_id = Some("t1".into());
    let (tx, rx) = mpsc::unbounded_channel();
    drop(rx);
    rig.feed(&Conn { out: tx }, turn("t1")).await;
    assert!(rig.shared.bots[0].read().missing_state_resync.is_none());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.feed(&conn, turn("t1")).await;
    let out = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(out.iter().filter(|v| v["type"] == "resync_request").count(), 1, "new writer retries unanswered authority: {out:?}");
    rig.tracker.table_id = Some("t2".into());
    rig.feed(&conn, turn("t2")).await;
    let out = std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(
        out.iter().filter(|v| v["type"] == "resync_request" && v["table_id"] == "t2").count(),
        1,
        "another table can recover the same hand id"
    );
}

#[tokio::test]
async fn an_earlier_unstored_replay_result_preserves_the_saved_open_hand() {
    let mut rig = Rig::new("recover-earlier-result");
    let (tx, _rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    open_a_hand(&mut rig, &conn).await;
    rig.shared.save_open_hands();
    rig.tracker = TableTracker::default();
    rig.tracker.reset_table();
    rig.shared.resumable.lock().extend(crate::live::take_open_hands(&rig.shared.store, &["A".to_string()]));
    assert_eq!(rig.shared.resumable.lock()["A"].hand_id, "h9", "the restart saved the hand we must recover");
    let mut earlier = result_frame();
    earlier["hand_id"] = json!("h-older");
    earlier["table_seq"] = json!(1);
    let mut saved = result_frame();
    saved["table_seq"] = json!(2);
    rig.feed(
        &conn,
        json!({"type": "resync_response", "replayed_events": [earlier, saved],
        "snapshot": {"hand_id": "h10", "pot": 0, "board": [], "seats": [], "hero": {"seat": 2}}}),
    )
    .await;
    assert!(rig.shared.store.hand("A", "h-older").unwrap().is_none(), "an unknown start cannot price the older hand");
    let recovered = rig.shared.store.hand("A", "h9").unwrap().expect("the earlier result must not discard h9's saved start");
    assert_eq!(recovered.net, Some(60));
    assert_eq!(rig.shared.bots[0].read().session_hands, 1);
    assert_eq!(rig.shared.bots[0].read().session_net, 60);
    assert!(rig.shared.resumable.lock().is_empty(), "the matching result consumes the saved start once");
}

/// 0315: the release watch exits between hands while a hand is in progress; the new process never
/// sees it end, so it was neither stored nor observed (5–10 hands a day). The hand is saved at
/// exit, and the resync replay's `hand_result` settles it here: stored once, with its real net.
#[tokio::test]
async fn a_hand_left_in_progress_is_recovered_from_the_resync_replay() {
    let mut rig = Rig::new("recover-restart");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    open_a_hand(&mut rig, &conn).await;
    // The process exits mid-hand: the hand is saved for whoever starts next.
    rig.shared.save_open_hands();

    // The next process starts cold: a fresh tracker, the saved hand, the resync replay.
    rig.tracker = TableTracker::default();
    rig.tracker.reset_table();
    rig.shared.resumable.lock().extend(crate::live::take_open_hands(&rig.shared.store, &["A".to_string()]));
    rig.feed(
        &conn,
        json!({"type": "resync_response", "replayed_events": [result_frame()],
        "snapshot": {"hand_id": "h9", "pot": 0, "board": [], "seats": [], "hero": {"seat": 2}}}),
    )
    .await;
    let row = rig.shared.store.hand("A", "h9").unwrap().expect("the recovered hand is stored");
    assert_eq!(row.net, Some(60), "net from the replayed final stack against the saved start stack");
    assert_eq!(row.table_id, "t");
    assert_eq!(row.hole, "AsKd");
    assert_eq!(rig.shared.bots[0].read().session_hands, 1, "the recovered hand counts as played");
    assert!(rig.shared.models.read().watermark.is_some(), "and the opponent models observe it");
    assert!(rig.shared.unstored_hands.lock().is_empty(), "a recovered hand is stored, not queued for retry");
    assert!(actions(&mut rx).is_empty(), "recovering a result must not answer a turn");
    // Frames repeat (LESSONS 14): a second resync carrying the same result stores nothing twice.
    rig.feed(
        &conn,
        json!({"type": "resync_response", "replayed_events": [result_frame()],
        "snapshot": {"hand_id": "h9", "pot": 0, "board": [], "seats": [], "hero": {"seat": 2}}}),
    )
    .await;
    assert_eq!(rig.shared.store.hand("A", "h9").unwrap().unwrap().net, Some(60));
    assert!(rig.shared.unstored_hands.lock().is_empty(), "the second replay is a no-op");
}

/// 0315 in production: across four hot swaps on 2026-09-27 no saved hand was ever recovered, and
/// nothing said so — the cold resync's replay held no result for the hand the swap interrupted (it had
/// ended, and a new hand begun, in the seconds before we resynced). That case must be named: the saved
/// hand is over (the snapshot is a different hand), the replay did not settle it, so its row is lost.
#[tokio::test]
async fn a_saved_hand_the_replay_cannot_settle_is_reported_lost_once() {
    let mut rig = Rig::new("recover-lost");
    let (tx, _rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    open_a_hand(&mut rig, &conn).await;
    rig.shared.save_open_hands();
    rig.tracker = TableTracker::default();
    rig.tracker.reset_table();
    rig.shared.resumable.lock().extend(crate::live::take_open_hands(&rig.shared.store, &["A".to_string()]));
    // The replay holds only the next hand's start; the snapshot is that next hand.
    let resync = json!({"type": "resync_response", "from_table_seq": 1, "to_table_seq": 3,
        "replayed_events": [{"type": "hand_start", "hand_id": "h10", "table_seq": 3, "dealer_seat": 2}],
        "snapshot": {"hand_id": "h10", "pot": 30, "board": [], "seats": [], "hero": {"seat": 2}}});
    rig.feed(&conn, resync.clone()).await;
    assert!(rig.shared.store.hand("A", "h9").unwrap().is_none(), "nothing to settle it from");
    let lost = |rig: &Rig| rig.shared.log.lock().iter().filter(|l| l.message.contains("h9") && l.message.contains("lost")).count();
    assert_eq!(lost(&rig), 1, "the lost row is named: {:?}", rig.shared.log.lock().iter().map(|l| &l.message).collect::<Vec<_>>());
    assert!(rig.shared.resumable.lock().is_empty(), "and dropped, so the next resync does not repeat it");
    rig.feed(&conn, resync).await;
    assert_eq!(lost(&rig), 1, "once");
}

/// A saved hand that is still the table's hand is not lost: its result arrives live (0323).
#[tokio::test]
async fn a_saved_hand_still_in_play_is_kept_for_its_live_result() {
    let mut rig = Rig::new("recover-still");
    let (tx, _rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    open_a_hand(&mut rig, &conn).await;
    rig.shared.save_open_hands();
    rig.tracker = TableTracker::default();
    rig.tracker.reset_table();
    rig.shared.resumable.lock().extend(crate::live::take_open_hands(&rig.shared.store, &["A".to_string()]));
    rig.feed(
        &conn,
        json!({"type": "resync_response", "replayed_events": [],
        "snapshot": {"hand_id": "h9", "pot": 30, "board": [], "seats": [], "hero": {"seat": 2}}}),
    )
    .await;
    assert!(rig.shared.resumable.lock().contains_key("A"), "still in play: kept for the live result");
    assert!(!rig.shared.log.lock().iter().any(|l| l.message.contains("lost")));
}

/// 0323: the other half of the same seam. The swap's hand ends *live* — 3.4 s after the reconnect for
/// one of the four hands at the 2026-09-27 07:55:33 UTC swap — so no replay carries it, and the new
/// process resynced into it past preflop, where `start_stacks` (and so the net) is never filled. The
/// row was stored unpriced and silent: 18–40 hands a day, invisible to `review`, the leak finder and
/// the experiment pair. The saved hand holds the missing start.
#[tokio::test]
async fn a_hand_that_ends_live_after_a_swap_is_priced_from_the_saved_start() {
    let mut rig = Rig::new("recover-live");
    let (tx, _rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    open_a_hand(&mut rig, &conn).await;
    rig.shared.save_open_hands();

    // The new process: a fresh tracker that resyncs into the hand on the flop. A flop `table_state`
    // does not fill `start_stacks`, and our seat was folded in the snapshot, so nothing else does.
    rig.tracker = TableTracker::default();
    rig.tracker.reset_table();
    rig.shared.resumable.lock().extend(crate::live::take_open_hands(&rig.shared.store, &["A".to_string()]));
    rig.feed(
        &conn,
        json!({"type": "table_state", "table_id": "t", "hand_id": "h9", "table_seq": 4, "street": "flop",
        "dealer_seat": 4, "small_blind": 10, "big_blind": 20, "pot": 30,
        "seats": [{"seat": 4, "name": "B", "stack": 1990, "bet": 0, "in_hand": true, "status": "playing"}],
        "hero": {"seat": 2}}),
    )
    .await;
    assert!(!rig.tracker.knows_hand("h9"), "the resync left the tracker unable to price the hand");

    rig.feed(&conn, result_frame()).await;
    let row = rig.shared.store.hand("A", "h9").unwrap().expect("the live result stores the row");
    assert_eq!(row.net, Some(60), "net from the final stack against the start saved before the swap");
    assert_eq!(rig.shared.bots[0].read().session_net, 60, "and the live path counts it");
    assert!(rig.shared.resumable.lock().is_empty(), "the saved hand is spent once it has settled");
}

/// 0325: the recovered hand's net is as real as any, and `status.sh` prints the count beside the net,
/// so a recovery that counts the hand but not its chips drifts the two apart.
#[tokio::test]
async fn a_recovered_hand_counts_in_the_session_net_too() {
    let mut rig = Rig::new("recover-session-net");
    let (tx, _rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    open_a_hand(&mut rig, &conn).await;
    rig.shared.save_open_hands();
    rig.tracker = TableTracker::default();
    rig.tracker.reset_table();
    rig.shared.resumable.lock().extend(crate::live::take_open_hands(&rig.shared.store, &["A".to_string()]));
    rig.feed(
        &conn,
        json!({"type": "resync_response", "replayed_events": [result_frame()],
        "snapshot": {"hand_id": "h9", "pot": 0, "board": [], "seats": [], "hero": {"seat": 2}}}),
    )
    .await;
    let bot = rig.shared.bots[0].read();
    assert_eq!((bot.session_hands, bot.session_net), (1, 60), "one hand played, its 60 chips counted");
}

/// 0315: the same recovery without a restart — the socket drops mid-hand and the resync replay
/// carries the result of the hand the running process still has. The tracker knows the hand, so
/// the result is settled from its own start stacks.
#[tokio::test]
async fn a_reconnect_inside_one_process_stores_the_replayed_result() {
    let mut rig = Rig::new("recover-reconnect");
    let (tx, _rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    open_a_hand(&mut rig, &conn).await;
    rig.feed(
        &conn,
        json!({"type": "resync_response", "replayed_events": [result_frame()],
        "snapshot": {"hand_id": "h9", "pot": 0, "board": [], "seats": [], "hero": {"seat": 2}}}),
    )
    .await;
    assert_eq!(rig.shared.store.hand("A", "h9").unwrap().expect("stored after the reconnect").net, Some(60));
}

/// 0315 across processes: a split fleet runs one process per bot, so each saves its own hand and
/// takes only its own — a process that plays no bot (the head) or another bot must not settle a hand
/// it did not play, and a hand is taken once (0128).
#[tokio::test]
async fn open_hands_are_saved_and_taken_around_other_processes() {
    use crate::live::{OPEN_HANDS_KEY, OpenHands, take_open_hands};
    let shared = Shared::for_test("open-hands-seam", &["A", "B"]);
    let hand =
        |id: &str, table: &str| sv10_venue::tracker::OpenHand { hand_id: id.into(), table_id: Some(table.into()), ..Default::default() };
    // B's process saved its hand at its own exit; we are mid-hand and exit too.
    let saved: OpenHands = [("B".to_string(), hand("h-b", "tb"))].into_iter().collect();
    shared.store.put_kv(OPEN_HANDS_KEY, &serde_json::to_string(&saved).unwrap()).unwrap();
    shared.bots[0].write().open_hand = Some(hand("h-a", "ta"));
    shared.save_open_hands();
    let both: OpenHands = serde_json::from_str(&shared.store.get_kv(OPEN_HANDS_KEY).unwrap().unwrap()).unwrap();
    assert!(both.contains_key("A") && both.contains_key("B"), "our save keeps B's hand, not only ours");

    let mine = take_open_hands(&shared.store, &["A".to_string()]);
    assert_eq!(mine.keys().collect::<Vec<_>>(), ["A"], "we take only the bots this process plays");
    assert_eq!(mine["A"].hand_id, "h-a");
    let left: OpenHands = serde_json::from_str(&shared.store.get_kv(OPEN_HANDS_KEY).unwrap().unwrap()).unwrap();
    assert_eq!(left.keys().collect::<Vec<_>>(), ["B"], "B's hand waits for B's process");
    // A process that plays no bot (the head) takes nothing, and a taken hand is taken once.
    assert!(take_open_hands(&shared.store, &[]).is_empty());
    assert_eq!(left.keys().collect::<Vec<_>>(), ["B"]);
    assert_eq!(take_open_hands(&shared.store, &["B".to_string()])["B"].hand_id, "h-b");
    assert!(take_open_hands(&shared.store, &["B".to_string()]).is_empty(), "a settled hand is not settled twice");
}

/// The recovery is narrow: a replayed result for a hand we never played is not ours to store — and a
/// hand our seat did play that we cannot settle (its start was never saved) is logged, not silent.
#[tokio::test]
async fn a_replayed_result_for_an_unknown_hand_is_not_stored() {
    let mut rig = Rig::new("recover-unknown");
    let (tx, _rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    let mut result = result_frame();
    result["hand_id"] = json!("h-other");
    rig.feed(
        &conn,
        json!({"type": "resync_response", "replayed_events": [result],
        "snapshot": {"hand_id": "h9", "pot": 0, "board": [], "seats": [], "hero": {"seat": 2}}}),
    )
    .await;
    assert!(rig.shared.store.hand("A", "h-other").unwrap().is_none());
    let warned = rig.shared.log.lock().iter().filter(|l| l.message.contains("cannot be settled")).count();
    assert_eq!(warned, 1, "a hand seat 2 played that we cannot settle is logged once");
    // A result for a hand seat 2 was not dealt into is another seat's business: stored nowhere, said
    // nowhere. Only the seat's own rows matter.
    let mut theirs = result_frame();
    theirs["hand_id"] = json!("h-theirs");
    theirs["final_stacks"] = json!({"4": 1940, "6": 2060});
    theirs["winners"] = json!([{"seat": 6, "name": "C", "stack": 2060, "amount": 60}]);
    rig.feed(
        &conn,
        json!({"type": "resync_response", "replayed_events": [theirs],
        "snapshot": {"hand_id": "h9", "pot": 0, "board": [], "seats": [], "hero": {"seat": 2}}}),
    )
    .await;
    assert!(rig.shared.store.hand("A", "h-theirs").unwrap().is_none());
    assert_eq!(rig.shared.log.lock().iter().filter(|l| l.message.contains("cannot be settled")).count(), 1);
}

#[tokio::test]
async fn a_player_resync_without_a_hero_block_lets_the_adopted_table_go() {
    // #933: the join send consumes the queued rejoin (client/mod.rs:336). `already_seated` then adopts
    // a table, so the bot counts as seated and the unseated rejoin timer stays off. A player resync whose
    // snapshot has no hero block gives no seat back, so nothing queues a rejoin until the 10-minute watchdog.
    let mut rig = Rig::new("resync-no-hero");
    let (tx, _rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.feed(&conn, json!({"type": "table_closed", "reason": "closed"})).await;
    rig.seat.pending_join = false; // the join loop sends join_lobby and consumes the queued rejoin
    rig.feed(&conn, json!({"type": "error", "code": "already_seated", "table_id": "t2"})).await;
    rig.feed(
        &conn,
        json!({"type": "resync_response", "role": "player", "replayed_events": [],
        "snapshot": {"pot": 0, "board": []}}),
    )
    .await;
    assert!(
        rig.seat.pending_join || rig.tracker.table_id.is_none(),
        "a player resync with no hero block must let go of the table: table_id {:?}, pending_join {}",
        rig.tracker.table_id,
        rig.seat.pending_join
    );
}

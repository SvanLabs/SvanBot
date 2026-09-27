//! Handler tests: the paths where a server message must answer, store or recover something —
//! duplicate turns, rejections and resyncs, table lifecycle, hand results and the hands a
//! restart left in progress (0315).
//!
//! The rig and its helpers are shared; the recovery paths live in `recover.rs`.

use super::*;

fn turn(token: &str) -> Value {
    json!({"type": "your_turn", "hand_id": "h1", "turn_token": token,
           "valid_actions": [{"action": "fold"}, {"action": "check"}]})
}

fn sequenced_turn(hand_id: &str, token: &str, seq: i64) -> Value {
    let mut value = turn(token);
    value["hand_id"] = json!(hand_id);
    value["table_seq"] = json!(seq);
    value
}

struct Rig {
    shared: Arc<Shared>,
    bot: BotConfig,
    http: reqwest::Client,
    tracker: TableTracker,
    rng: SmallRng,
    seat: Seat,
}

impl Rig {
    fn new(tag: &str) -> Rig {
        let shared = Shared::for_test(tag, &["A"]);
        let bot = shared.config.bots[0].clone();
        let mut tracker = TableTracker::default();
        tracker.reset_table();
        Rig { shared, bot, http: reqwest::Client::new(), tracker, rng: SmallRng::seed_from_u64(1), seat: Seat::default() }
    }

    async fn feed(&mut self, conn: &Conn, msg: Value) {
        self.feed_end(conn, msg).await;
    }

    async fn feed_end(&mut self, conn: &Conn, msg: Value) -> Option<SessionEnd> {
        handle(&self.shared, 0, &self.bot, &self.http, &mut self.tracker, conn, &msg, &mut self.rng, &mut self.seat).await
    }
}

fn actions(rx: &mut mpsc::UnboundedReceiver<Value>) -> Vec<Value> {
    std::iter::from_fn(|| rx.try_recv().ok()).filter(|v| v["type"] == "action").collect()
}

#[tokio::test]
async fn a_redelivered_turn_is_answered_once_unless_the_action_was_rejected() {
    let mut rig = Rig::new("dedup");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.feed(&conn, turn("t1")).await;
    rig.feed(&conn, turn("t1")).await;
    let sent = actions(&mut rx);
    assert_eq!(sent.len(), 1, "duplicate your_turn must not double-act: {sent:?}");
    assert_eq!(sent[0]["turn_token"], "t1");
    assert_eq!(sent[0]["hand_id"], "h1");
    rig.feed(
        &conn,
        json!({"type": "resync_response", "replayed_events": [], "snapshot": {
            "hand_id": "h1", "pot": 0, "board": [], "seats": [],
            "hero": {"seat": 0, "turn_token": "t1", "valid_actions": [{"action": "check"}]}
        }}),
    )
    .await;
    assert!(actions(&mut rx).is_empty(), "resync must not re-answer an accepted (hand, token)");
    // A new token is a new turn.
    rig.feed(&conn, turn("t2")).await;
    assert_eq!(actions(&mut rx).len(), 1);
    // A rejected action leaves the turn open: the redelivery is answered again.
    rig.feed(&conn, json!({"type": "action_rejected", "code": "invalid_amount"})).await;
    rig.feed(&conn, turn("t2")).await;
    assert_eq!(actions(&mut rx).len(), 1);
}

#[tokio::test]
async fn a_stale_or_invalid_rejection_resyncs_to_recover_the_turn_but_never_loops() {
    // Private your_turn messages are not replayed and a rejection does not restart the 45 s
    // timer: the only way to act again is the resync snapshot's token (spec, Reconnection).
    let mut rig = Rig::new("reject-resync");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.tracker.table_id = Some("t1".into());
    rig.feed(&conn, turn("t1")).await;
    let sent = |rx: &mut mpsc::UnboundedReceiver<Value>| std::iter::from_fn(|| rx.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(sent(&mut rx).iter().filter(|v| v["type"] == "action").count(), 1);
    rig.feed(&conn, json!({"type": "action_rejected", "code": "stale_turn_token", "reason": "stale"})).await;
    let out = sent(&mut rx);
    assert!(out.iter().any(|v| v["type"] == "resync_request" && v["table_id"] == "t1"), "{out:?}");
    // The snapshot restores the pending token: the turn is answered again.
    rig.feed(&conn, turn("t1")).await;
    assert_eq!(sent(&mut rx).iter().filter(|v| v["type"] == "action").count(), 1);
    // At most three rejection resyncs per hand: a persistent rejection cannot loop.
    for _ in 0..4 {
        rig.feed(&conn, json!({"type": "action_rejected", "code": "invalid_action"})).await;
    }
    let resyncs = sent(&mut rx).iter().filter(|v| v["type"] == "resync_request").count();
    assert_eq!(resyncs, 2, "three per hand in total, one already used");
    // Protocol bugs do not resync; they are errors to fix, not turns to recover.
    let mut rig = Rig::new("reject-protocol");
    rig.tracker.table_id = Some("t1".into());
    rig.feed(&conn, json!({"type": "action_rejected", "code": "missing_action_id"})).await;
    assert!(!sent(&mut rx).iter().any(|v| v["type"] == "resync_request"));
}

#[tokio::test]
async fn older_turn_authority_is_ignored_but_a_new_hand_may_reuse_a_token() {
    let mut rig = Rig::new("turn-order");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.feed(&conn, sequenced_turn("h1", "t10", 10)).await;
    rig.feed(&conn, sequenced_turn("h1", "t12", 12)).await;
    rig.feed(&conn, sequenced_turn("h1", "late-t11", 11)).await;
    assert_eq!(actions(&mut rx).len(), 2, "late older authority must not act");
    rig.feed(&conn, sequenced_turn("h2", "t12", 1)).await;
    assert_eq!(actions(&mut rx).len(), 1, "deduplication is scoped to (hand, token)");
}

/// A table that closes while we leave to top up completes that leave: rejoin, never pause.
/// A table that closes while the operator paused us stays paused.
#[tokio::test]
async fn a_table_closing_during_a_top_up_rejoins_but_a_pause_stays_paused() {
    let mut rig = Rig::new("closed-topup");
    let (tx, _rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.seat.leave(Leave::Rejoin);
    rig.feed(&conn, json!({"type": "table_closed", "table_id": "t", "reason": "not_enough_players"})).await;
    assert!(rig.seat.pending_join && !rig.seat.leaving());
    assert_ne!(rig.shared.bots[0].read().mode, "paused");
    rig.seat = Seat::default();
    rig.seat.leave(Leave::Pause);
    rig.feed(&conn, json!({"type": "table_closed", "table_id": "t2", "reason": "not_enough_players"})).await;
    assert!(!rig.seat.pending_join && rig.seat.leave_reason() == Some(Leave::Pause));
    assert_eq!(rig.shared.bots[0].read().mode, "paused");
}

/// A hand that ends with a bankable stack leaves to rejoin (0204, 0206); leaving the table then
/// queues the rejoin instead of pausing.
#[tokio::test]
async fn a_deep_stack_at_hand_end_leaves_and_the_leave_rejoins() {
    let mut rig = Rig::new("bank-leave");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.feed(
        &conn,
        json!({"type": "table_joined", "table_id": "t", "seat": 2, "table_seq": 1,
        "players": [{"seat": 2, "name": "A", "stack": 9000}, {"seat": 4, "name": "B", "stack": 5000}]}),
    )
    .await;
    rig.feed(&conn, json!({"type": "hand_start", "table_id": "t", "hand_id": "h1", "table_seq": 2, "dealer_seat": 4})).await;
    rig.tracker.bb = 20;
    rig.feed(&conn, json!({"type": "hand_result", "hand_id": "h1", "table_seq": 3, "final_stacks": {"2": 50_000, "4": 0}})).await;
    let sent: Vec<Value> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert_eq!(sent.iter().filter(|m| m["type"] == "leave_table").count(), 1, "{sent:?}");
    assert_eq!(rig.seat.leave_reason(), Some(Leave::Rejoin));
    rig.feed(&conn, json!({"type": "player_left", "name": "A", "reason": "left", "table_seq": 4})).await;
    assert!(rig.seat.pending_join && !rig.seat.leaving());
    assert_ne!(rig.shared.bots[0].read().mode, "paused");
}

/// Season end as the spec describes it (0029): wind-down refusals, the table force-closed mid-hand,
/// then `season_ended`. The bot must drop the table (and any leave or top-up in flight), show no
/// stale table, never act on the dead hand, and queue exactly one rejoin for the new season.
#[tokio::test]
async fn a_connection_refusal_ends_the_session_and_backs_off() {
    let mut rig = Rig::new("throttled");
    let (tx, _rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    // The server's wording at the 2026-09-22 15:21 start.
    let refusal = json!({"type": "error", "code": "rate_limited", "message": "Too many connection attempts for this play pool"});
    assert!(matches!(rig.feed_end(&conn, refusal).await, Some(SessionEnd::Throttled)));
    // The documented message-rate case only drops one message; the session stays up.
    let dropped = json!({"type": "error", "code": "rate_limited", "message": "Exceeded 20 messages/second"});
    assert!(rig.feed_end(&conn, dropped).await.is_none());
    let one = Duration::from_secs(1);
    let brief = Duration::ZERO;
    let first = next_backoff(Some(&SessionEnd::Throttled), one, brief);
    assert_eq!(first, Duration::from_secs(5));
    assert_eq!(next_backoff(Some(&SessionEnd::Throttled), first, brief), Duration::from_secs(10));
    assert_eq!(next_backoff(Some(&SessionEnd::Throttled), Duration::from_secs(50), brief), Duration::from_secs(60));
    assert_eq!(next_backoff(Some(&SessionEnd::Closed), Duration::from_secs(40), brief), one);
}

/// 0268: the reducer contract asks the client to sort the replay window by `table_seq` and drop
/// repeats, because `apply_event` is not idempotent. A shuffled window with a duplicate must
/// produce one action per event, in sequence order.
#[test]
fn a_resync_replay_window_is_sorted_by_table_seq_and_deduplicated() {
    let msg = json!({"replayed_events": [
        {"type": "player_action", "table_seq": 30, "seat": 2, "action": "bet"},
        {"type": "player_action", "table_seq": 20, "seat": 1, "action": "bet"},
        {"type": "player_action", "table_seq": 30, "seat": 2, "action": "bet"},
        {"type": "hand_start", "table_seq": 10, "hand_id": "h1"},
        {"type": "table_state", "table_seq": 40, "pot": 90},
    ]});
    let window = crate::client::recover::replay_window(&msg);
    let seqs: Vec<i64> = window.iter().map(|e| e["table_seq"].as_i64().unwrap()).collect();
    assert_eq!(seqs, [10, 20, 30, 40], "sorted, and the repeated action is applied once");
    // An unsequenced event keeps its place after the sequenced ones rather than guessing an order.
    let mixed = json!({"replayed_events": [
        {"type": "player_action", "seat": 1, "action": "bet"},
        {"type": "hand_start", "table_seq": 10, "hand_id": "h1"},
    ]});
    let window = crate::client::recover::replay_window(&mixed);
    assert_eq!(window.len(), 2);
    assert_eq!(window[0]["table_seq"].as_i64(), Some(10), "the sequenced event first");
    assert_eq!(window[1]["type"], "player_action", "the unsequenced one after it");
}

/// 0264: the spec asks for one final `leave_table` when `table_closed` overtakes our own
/// `player_left`, and none when we were never leaving.
#[tokio::test]
async fn a_table_that_closes_mid_leave_gets_the_final_leave_frame() {
    let mut rig = Rig::new("closed-leave");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    let frames = |rx: &mut mpsc::UnboundedReceiver<Value>| {
        std::iter::from_fn(|| rx.try_recv().ok()).map(|v| v["type"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>()
    };
    rig.feed(&conn, json!({"type": "table_joined", "table_id": "t1", "seat": 0, "players": []})).await;
    frames(&mut rx);
    // A table closing on its own: nothing to release, and we still rejoin.
    rig.feed(&conn, json!({"type": "table_closed", "reason": "insufficient_players"})).await;
    assert_eq!(frames(&mut rx), Vec::<String>::new(), "we were not leaving, so no final frame");
    assert!(rig.seat.pending_join, "and the bot is re-queued");

    // A leave in flight, then the table closes before our own player_left.
    rig.feed(&conn, json!({"type": "table_joined", "table_id": "t2", "seat": 0, "players": []})).await;
    frames(&mut rx);
    assert!(rig.seat.leave(Leave::Rejoin));
    conn.send(json!({"type": "leave_table"}));
    frames(&mut rx); // our own leave, so the next frame is the handler's
    rig.feed(&conn, json!({"type": "table_closed", "reason": "insufficient_players"})).await;
    assert_eq!(frames(&mut rx), ["leave_table"], "the spec's final leave_table, exactly once");
    assert!(rig.seat.pending_join, "and the rejoin still follows");
    // `not_at_table` after it is a clean exit, not an error.
    assert!(rig.feed_end(&conn, json!({"type": "error", "code": "not_at_table"})).await.is_none(), "a clean exit, not a session end");
}

#[test]
fn a_reset_after_a_healthy_session_reconnects_at_once() {
    // 2026-09-26: five server resets mid-play each reconnected 61 s later, past the 45 s turn
    // deadline, because the transport-error backoff had doubled to its cap over earlier errors
    // and a healthy session never reset it.
    let capped = Duration::from_secs(60);
    assert_eq!(next_backoff(None, capped, HEALTHY_SESSION), Duration::from_secs(2));
    assert_eq!(next_backoff(None, capped, Duration::from_secs(3 * 3600)), Duration::from_secs(2));
    // Failed connection attempts (short sessions) still back off to the cap.
    assert_eq!(next_backoff(None, Duration::from_secs(8), Duration::from_secs(20)), Duration::from_secs(16));
    assert_eq!(next_backoff(None, Duration::from_secs(40), Duration::from_secs(1)), capped);
    // A refusal after a long session starts its own ladder at 5 s.
    assert_eq!(next_backoff(Some(&SessionEnd::Throttled), capped, HEALTHY_SESSION), Duration::from_secs(5));
}

#[tokio::test]
async fn season_end_closes_the_table_and_queues_a_fresh_join() {
    let mut rig = Rig::new("season-end");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.feed(
        &conn,
        json!({"type": "table_joined", "table_id": "t-old", "seat": 2, "table_seq": 1,
        "players": [{"seat": 2, "name": "A", "stack": 9000}, {"seat": 4, "name": "B", "stack": 5000}]}),
    )
    .await;
    rig.feed(&conn, json!({"type": "hand_start", "table_id": "t-old", "hand_id": "h-last", "table_seq": 2, "dealer_seat": 4})).await;
    assert_eq!(rig.tracker.table_id.as_deref(), Some("t-old"));
    // A top-up leave was in flight when the season wound down.
    rig.seat.leave(Leave::Rejoin);
    rig.feed(&conn, json!({"type": "error", "code": "winding_down", "message": "Season is ending"})).await;
    rig.feed(&conn, json!({"type": "table_closed", "table_id": "t-old", "reason": "season_ended", "table_seq": 3})).await;
    rig.feed(&conn, json!({"type": "season_ended", "season_number": 12, "next_season_number": 13})).await;
    assert!(rig.seat.pending_join && !rig.seat.leaving(), "seat after season end: {:?}", rig.seat);
    assert_eq!(rig.tracker.table_id, None);
    {
        let b = rig.shared.bots[0].read();
        assert_eq!((b.table_id.as_deref(), b.hand_id.as_deref()), (None, None), "dashboard still shows the closed table");
    }
    // A late turn for the dead hand is not answered from a cleared table.
    rig.feed(&conn, turn("stale")).await;
    let sent: Vec<Value> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert!(sent.iter().all(|m| m["type"] != "join_lobby"), "joins are sent by the session loop, not the handler");
    assert!(sent.iter().filter(|m| m["type"] == "action").all(|m| m["action"] == "fold" || m["action"] == "check"), "{sent:?}");
    // Seq numbering restarts on the new season's table.
    rig.feed(
        &conn,
        json!({"type": "table_joined", "table_id": "t-new", "seat": 1, "table_seq": 1,
        "players": [{"seat": 1, "name": "A", "stack": 5000}, {"seat": 3, "name": "C", "stack": 5000}]}),
    )
    .await;
    assert_eq!(rig.tracker.table_id.as_deref(), Some("t-new"));
    assert_eq!(rig.shared.bots[0].read().table_id.as_deref(), Some("t-new"));
}

#[tokio::test]
async fn an_action_lost_to_a_dead_writer_is_not_marked_answered() {
    let mut rig = Rig::new("deadwriter");
    rig.tracker.table_id = Some("table".into());
    rig.tracker.hand_id = Some("h1".into());
    rig.tracker.hero_seat = Some(0);
    rig.tracker.street = Some(sv10_core::engine::Street::Preflop);
    rig.tracker.pot = 40;
    rig.tracker.hole = Some([sv10_core::cards::Card::parse("Ah").unwrap(), sv10_core::cards::Card::parse("Kd").unwrap()]);
    rig.tracker
        .seats
        .insert(0, sv10_venue::tracker::SeatView { seat: 0, name: "A".into(), stack: 1_980, bet: 20, in_hand: true, ..Default::default() });
    rig.tracker
        .seats
        .insert(1, sv10_venue::tracker::SeatView { seat: 1, name: "B".into(), stack: 1_980, bet: 20, in_hand: true, ..Default::default() });
    let (tx, rx) = mpsc::unbounded_channel();
    drop(rx);
    let conn = Conn { out: tx };
    rig.feed(&conn, turn("t9")).await;
    assert_eq!(rig.shared.bots[0].read().last_acted_turn_token, None);
    assert!(rig.shared.store.decisions_for_hand("A", "h1").unwrap().is_empty());
    assert!(rig.shared.store.audit_batch(10).unwrap().is_empty());
    assert!(rig.tracker.pending_calibration.is_empty());
    // After reconnecting, the resynced turn with the same token is answered.
    let (tx, mut rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.feed(&conn, turn("t9")).await;
    assert_eq!(actions(&mut rx).len(), 1);
    assert_eq!(rig.shared.store.decisions_for_hand("A", "h1").unwrap().len(), 1);
    assert_eq!(rig.shared.store.audit_batch(10).unwrap().len(), 1);
    assert_eq!(rig.tracker.pending_calibration.len(), 1);
    rig.feed(&conn, turn("t9")).await;
    assert!(actions(&mut rx).is_empty());
    assert_eq!(rig.shared.store.decisions_for_hand("A", "h1").unwrap().len(), 1);
}

/// A hand in progress at table `t` with us on seat 2 holding AsKd, 2,000 chips each (0315).
async fn open_a_hand(rig: &mut Rig, conn: &Conn) {
    rig.feed(
        conn,
        json!({"type": "table_joined", "table_id": "t", "seat": 2, "table_seq": 1,
        "players": [{"seat": 2, "name": "A", "stack": 2000}, {"seat": 4, "name": "B", "stack": 2000}]}),
    )
    .await;
    rig.feed(
        conn,
        json!({"type": "hand_start", "table_id": "t", "hand_id": "h9", "table_seq": 2, "dealer_seat": 4,
        "seat": 2, "blinds": {"small_blind": 10, "big_blind": 20}}),
    )
    .await;
    rig.feed(
        conn,
        json!({"type": "table_state", "table_id": "t", "hand_id": "h9", "table_seq": 3, "street": "preflop",
        "dealer_seat": 4, "small_blind": 10, "big_blind": 20, "pot": 30,
        "seats": [{"seat": 2, "name": "A", "stack": 1980, "bet": 20, "in_hand": true, "status": "playing"},
                  {"seat": 4, "name": "B", "stack": 1990, "bet": 10, "in_hand": true, "status": "playing"}],
        "hero": {"seat": 2, "hole_cards": ["As", "Kd"]}}),
    )
    .await;
}

/// The server's `hand_result` for the hand `open_a_hand` started: we win 60.
fn result_frame() -> Value {
    json!({"type": "hand_result", "hand_id": "h9", "table_seq": 9, "total_pot": 60,
    "winners": [{"seat": 2, "name": "A", "stack": 2060, "amount": 60, "hand_description": "Pair of Aces"}],
    "final_stacks": {"2": 2060, "4": 1940}})
}

/// 0321: pricing models the range opponents read us for, but `observe` skips our own seat, so that
/// image was empty for the fleet's whole life and every opponent read us as an unknown player. The
/// store path that folds a finished hand feeds both halves through one call.
#[tokio::test]
async fn a_stored_hand_feeds_the_image_of_our_own_play() {
    use sv10_core::model::{HERO_SEEN_ALL, HERO_SEEN_ONE};
    let mut rig = Rig::new("hero-image");
    let (tx, _rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    open_a_hand(&mut rig, &conn).await;
    rig.feed(&conn, result_frame()).await;
    assert!(rig.shared.store.hand("A", "h9").unwrap().is_some(), "the hand is stored");
    let models = rig.shared.models.read();
    assert_eq!(models.players["B"].hands, 1.0, "the opponent is modelled as before");
    assert!(!models.players.contains_key("A"), "we are not our own opponent");
    for key in [format!("{HERO_SEEN_ONE}A"), HERO_SEEN_ALL.to_string()] {
        assert_eq!(models.hero_seen[&key].hands, 1.0, "our own play feeds the image at {key}");
    }
}

/// A `table_state` frame carrying the hash the server would send for it.
fn hashed_table_state(seq: i64, pot: i64) -> Value {
    let mut msg = json!({"type": "table_state", "table_id": "t1", "table_seq": seq, "pot": pot, "seats": []});
    let hash = sv10_venue::statehash::compute(&msg);
    msg["state_hash"] = json!(hash);
    msg
}

/// 0301: every verified snapshot is counted twice — for the session (what a hot swap resets) and for
/// the lifetime (what the Runtime health panel keeps). A mismatch keeps its verdict, and the verdict
/// is real: the tracker advances its watermark before the frame is judged, so a genuinely divergent
/// snapshot used to be filed as STALE (a replayed frame) and a serializer bug would have looked
/// harmless in the incident table and in `review state-hash`.
#[tokio::test]
async fn a_state_hash_mismatch_is_counted_and_keeps_a_truthful_verdict() {
    let mut rig = Rig::new("state-hash-count");
    let (tx, _rx) = mpsc::unbounded_channel();
    let conn = Conn { out: tx };
    rig.feed(&conn, hashed_table_state(1, 100)).await;
    rig.feed(&conn, hashed_table_state(2, 120)).await;
    {
        let b = rig.shared.bots[0].read();
        assert_eq!((b.state_hash_ok, b.state_hash_bad), (2, 0));
        assert_eq!(b.state_hash_lifetime.ok, 2, "the lifetime tally counts the same verifications");
        assert!(b.state_hash_lifetime.last_mismatch.is_none(), "nothing has mismatched");
    }
    // Same sequence, different content: a replayed frame from the resync window, judged against the
    // watermark we had already applied — STALE is the honest verdict.
    let mut replayed = hashed_table_state(2, 999);
    replayed["state_hash"] = json!(sv10_venue::statehash::compute(&hashed_table_state(2, 121)));
    rig.feed(&conn, replayed).await;
    // A newer frame whose hash belongs to a different snapshot: the server and our serializer
    // disagree about a state we have never seen — that is a divergence.
    let mut diverged = hashed_table_state(3, 130);
    diverged["state_hash"] = json!(sv10_venue::statehash::compute(&hashed_table_state(3, 131)));
    rig.feed(&conn, diverged).await;
    let b = rig.shared.bots[0].read();
    assert_eq!((b.state_hash_ok, b.state_hash_bad), (2, 2));
    assert_eq!(b.state_hash_lifetime.bad, 2);
    let m = b.state_hash_lifetime.last_mismatch.as_ref().expect("the mismatch is kept for the panel");
    assert_eq!((m.table.as_str(), m.verdict.as_str()), ("t1", "DIVERGED"));
    assert!(m.summary.starts_with("DIVERGED table_seq=3 (last applied 2)"), "{}", m.summary);
    assert!(m.at > 1_700_000_000.0, "a real time, not a placeholder: {}", m.at);
    drop(b);
    // `review state-hash` reads the same rows from the incident table, newest first.
    let incidents = rig.shared.store.hash_incidents(5).unwrap();
    assert_eq!(incidents.len(), 2);
    assert_eq!((incidents[0].verdict.as_str(), incidents[0].table.as_str(), incidents[0].table_seq), ("DIVERGED", "t1", 3));
    assert_eq!(incidents[1].verdict, "STALE", "a replayed frame is still told apart from a divergence");
}

mod recover;

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

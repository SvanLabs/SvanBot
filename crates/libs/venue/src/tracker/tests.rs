use super::*;
use serde_json::json;
use sv10_model::model::ModelStore;

mod preflop_rebuild;

mod recovery;

fn frames() -> Vec<Value> {
    include_str!("../../tests/fixtures/hand_capture.jsonl")
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|v| v["dir"] == "in")
        .map(|v| v["msg"].clone())
        .collect()
}

/// The server reports river shoves with `amount: 0` (live hand c27d0070); the chips must still
/// be recorded, from the pot change or the stack the player had behind.
#[test]
fn all_in_with_zero_amount_records_its_chips() {
    for (with_pot_after, stack_fields) in [(true, true), (false, false), (false, true)] {
        let mut t = TableTracker::default();
        t.reset_table();
        t.street = Some(Street::River);
        t.pot = 470;
        t.seats.insert(1, SeatView { seat: 1, name: "silentflute".into(), stack: 906_631, in_hand: true, ..Default::default() });
        let mut m =
            json!({"type": "player_action", "seat": 1, "action": "all_in", "amount": 0, "street": "river", "pot_before": 470, "stack": 0});
        if with_pot_after {
            m["pot_after"] = json!(907_101);
        }
        if stack_fields {
            // Both already post-action, so their difference says nothing.
            m["stack_before"] = json!(0);
            m["stack_after"] = json!(0);
        }
        t.player_action(&m);
        let r = t.history.last().unwrap();
        assert_eq!((r.kind, r.to), (ActionKind::AllIn, 906_631), "pot_after {with_pot_after}, stack fields {stack_fields}");
        assert_eq!(t.seats[&1].bet, 906_631);
    }
}

#[test]
fn replay_matches_the_live_dispatch() {
    let frames = frames();
    let hands = replay(&frames);
    assert_eq!(hands.len(), 1);
    let (ts, f) = &hands[0];
    assert!(ts.is_some());
    assert_eq!(f.summary.players.len(), 6);
    assert_eq!(f.hero_hole.map(|h| h[0].to_string()).as_deref(), Some("6c"));
    assert!(f.summary.history.iter().any(|r| r.kind == ActionKind::Raise && r.to == 64));
}

#[test]
fn replays_captured_hand() {
    let mut t = TableTracker::default();
    t.reset_table();
    let mut sit = None;
    let mut finished = None;
    for m in frames() {
        t.accept_seq(&m);
        match m["type"].as_str().unwrap() {
            "table_joined" => t.table_joined(&m),
            "table_state" => t.table_state(&m),
            "hand_start" => t.hand_start(&m),
            "hole_cards" => t.hole_cards(&m),
            "player_action" => t.player_action(&m),
            "community_cards" => t.community_cards(&m),
            "your_turn" => {
                let legal = LegalActions::parse(&m["valid_actions"]);
                sit = t.situation(&m, &legal);
            }
            "hand_result" => finished = t.hand_result(&m),
            _ => {}
        }
    }
    let sit = sit.expect("situation at your_turn");
    assert_eq!(sit.hero_seat, 0);
    assert_eq!(sit.hole[0].to_string(), "6c");
    assert_eq!(sit.pot, 286);
    assert_eq!(sit.call_amount, 192);
    assert_eq!(sit.min_raise_to, Some(320));
    assert_eq!(sit.button, 0);
    assert_eq!(sit.players.len(), 6);
    // Blinds come from table_state bets, not events.
    let sb = sit.players.iter().find(|p| p.seat == 1).unwrap();
    assert_eq!(sb.bet, 10);
    assert_eq!(sit.history.len(), 3);
    assert_eq!(sit.history[0].kind, ActionKind::Raise);
    assert_eq!(sit.history[0].to, 64);
    assert!(sit.history[0].full_raise);
    assert_eq!(sit.history[2].to, 192);
    assert_eq!(sit.history[2].bet_before, 0);

    let f = finished.expect("hand result");
    let streets: Vec<Street> = f.summary.history.iter().map(|r| r.street).collect();
    // aido's preflop-closing call is labeled "flop" by the server; we must keep it preflop.
    let calls: Vec<&ActionRecord> = f.summary.history.iter().filter(|r| r.seat == 5 && r.kind == ActionKind::Call).collect();
    assert_eq!(calls[0].street, Street::Preflop);
    assert!(streets.contains(&Street::River));
    assert_eq!(f.hero_net, Some(0));
    assert_eq!(f.board.len(), 5);
    assert_eq!(f.summary.players.len(), 6);
    assert_eq!(f.winners, vec!["aido".to_string()]);
    let history_total: usize = f.summary.history.len();
    assert_eq!(history_total, 15);
}

#[test]
fn no_situation_when_hero_not_seated() {
    let mut t = TableTracker::default();
    t.reset_table();
    t.hole = Some([Card::parse("Ah").unwrap(), Card::parse("Kd").unwrap()]);
    t.seats.insert(1, SeatView { seat: 1, name: "villain".into(), stack: 1000, in_hand: true, ..Default::default() });
    t.seats.insert(2, SeatView { seat: 2, name: "other".into(), stack: 1000, in_hand: true, ..Default::default() });
    let msg = serde_json::json!({"type": "your_turn", "seat": 4, "pot": 30, "valid_actions": [{"action": "fold"}, {"action": "call", "amount": 20}]});
    let legal = LegalActions::parse(&msg["valid_actions"]);
    assert!(t.situation(&msg, &legal).is_none());
}

/// After a resync without replayed events, rebuilt preflop raises must carry the real pot
/// geometry: blinds counted once, each raise adding only its excess (0092).
#[test]
fn resync_rebuilt_raises_have_consistent_pot_geometry() {
    let mut t = TableTracker::default();
    t.reset_table();
    t.bb = 20;
    t.hero_seat = Some(0);
    t.hole = Some([Card::parse("As").unwrap(), Card::parse("Kd").unwrap()]);
    t.dealer = 0;
    let seat = |seat: usize, bet: i64| SeatView { seat, name: format!("p{seat}"), stack: 2_000, bet, in_hand: true, ..Default::default() };
    // Hero on the button with nothing in; SB 10, BB 20; seat 3 opened to 60, seat 4 3-bet to 180.
    for (s, b) in [(0, 0), (1, 10), (2, 20), (3, 60), (4, 180)] {
        t.seats.insert(s, seat(s, b));
    }
    let legal = LegalActions {
        can_fold: true,
        can_check: false,
        call: Some(180),
        raise_min: Some(300),
        raise_max: Some(2_000),
        all_in: Some(2_000),
    };
    let sit = t.situation(&json!({"type": "your_turn", "seat": 0, "pot": 270}), &legal).unwrap();
    let raises: Vec<_> = sit.history.iter().filter(|r| r.kind == ActionKind::Raise).collect();
    assert_eq!(raises.len(), 2);
    // Only the blinds before the open; the open added 60; the 3-bettor faced 60 more.
    assert_eq!((raises[0].to, raises[0].pot_before, raises[0].to_call_before), (60, 30, 20));
    assert_eq!((raises[1].to, raises[1].pot_before, raises[1].to_call_before), (180, 90, 60));
    // Every chip on the table is counted once: 90 + 180 = 270, the reported pot.
    assert_eq!(raises[1].pot_before + 180, sit.pot);
}

/// A snapshot-only reconnect at the same authoritative turn must preserve every reconstructible
/// policy input. The protocol snapshot marks folded seats but cannot reproduce their action order;
/// folds do not narrow ranges and folded seats cannot respond, so compare the actionable history.
#[test]
fn snapshot_only_resync_preserves_reconstructible_policy_inputs() {
    let frames = frames();
    let turn_index = frames.iter().position(|m| m["type"] == "your_turn").expect("captured turn");
    let turn = &frames[turn_index];
    let snapshot = frames[..turn_index].iter().rev().find(|m| m["type"] == "table_state").expect("authoritative snapshot before turn");

    let mut incremental = TableTracker::default();
    incremental.reset_table();
    for m in &frames[..turn_index] {
        match m["type"].as_str().unwrap_or("") {
            "table_joined" => incremental.table_joined(m),
            "table_state" => incremental.table_state(m),
            "hand_start" => incremental.hand_start(m),
            "hole_cards" => incremental.hole_cards(m),
            "player_action" => incremental.player_action(m),
            "community_cards" => incremental.community_cards(m),
            _ => {}
        }
    }
    let legal = LegalActions::parse(&turn["valid_actions"]);
    let live = incremental.situation(turn, &legal).expect("incremental situation");

    let mut resynced = TableTracker::default();
    resynced.reset_table();
    resynced.table_state(snapshot);
    let restored = resynced.situation(turn, &legal).expect("snapshot situation");

    assert_eq!(restored.pot, live.pot);
    assert_eq!(restored.call_amount, live.call_amount);
    assert_eq!((restored.min_raise_to, restored.max_raise_to), (live.min_raise_to, live.max_raise_to));
    assert_eq!(serde_json::to_value(&restored.players).unwrap(), serde_json::to_value(&live.players).unwrap());
    assert_eq!(restored.board, live.board);
    assert_eq!(restored.street, live.street);
    assert_eq!(restored.button, live.button);
    // Think times (0234) exist only in the event stream: a snapshot cannot rebuild them, and the
    // policy treats a missing one as neutral, so they are left out of the comparison.
    let actionable = |history: &[ActionRecord]| {
        history
            .iter()
            .filter(|r| r.kind != ActionKind::Fold)
            .map(|r| serde_json::to_value(ActionRecord { think_ms: None, street_open: false, ..r.clone() }).unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(actionable(&restored.history), actionable(&live.history));
    assert_eq!(restored.history.len(), 2, "snapshot reconstructs the two raises");
    assert_eq!(live.history.len(), 3, "event stream also records the intervening fold");
    assert!(restored.players.iter().any(|p| p.seat == 4 && p.folded), "snapshot retains the fold state");
}

#[test]
fn calibration_realizations_keep_each_pre_action_frontier() {
    let pending = [
        PendingCalibration {
            category: "turn:call".into(),
            predicted_incremental_chips: 40.0,
            hero_stack_before_action: 1_000,
            scale_chips: 200,
        },
        PendingCalibration {
            category: "river:raise:medium".into(),
            predicted_incremental_chips: -20.0,
            hero_stack_before_action: 900,
            scale_chips: 400,
        },
    ];
    assert_eq!(realized_incremental_chips(1_100, pending[0].hero_stack_before_action), 100);
    assert_eq!(realized_incremental_chips(1_100, pending[1].hero_stack_before_action), 200);
    assert_eq!(realized_incremental_chips(500, pending[1].hero_stack_before_action), -400);
}

/// A snapshot reconnect must still produce one name-owned model observation for the completed
/// hand. Processing the remaining authoritative frames must not lose a player or duplicate the
/// observation merely because the earlier action stream was replaced by snapshot state.
#[test]
fn resync_hand_summary_is_observed_once_by_name() {
    let frames = frames();
    let turn_index = frames.iter().position(|m| m["type"] == "your_turn").expect("captured turn");
    let snapshot_index = (0..turn_index).rev().find(|&i| frames[i]["type"] == "table_state").expect("snapshot before turn");

    let mut tracker = TableTracker::default();
    tracker.reset_table();
    tracker.table_state(&frames[snapshot_index]);

    let mut finished = None;
    for message in &frames[snapshot_index + 1..] {
        if !tracker.accept_seq(message) {
            continue;
        }
        match message["type"].as_str().unwrap_or("") {
            "table_state" => tracker.table_state(message),
            "player_action" => tracker.player_action(message),
            "community_cards" => tracker.community_cards(message),
            "hand_result" => finished = tracker.hand_result(message),
            _ => {}
        }
    }

    let finished = finished.expect("one completed resynced hand");
    let expected_names = finished.summary.players.iter().map(|(_, name)| name.clone()).collect::<Vec<_>>();
    assert_eq!(expected_names.len(), 6);

    let mut models = ModelStore::default();
    models.observe(&finished.summary, None);
    for name in expected_names {
        assert_eq!(models.players[&name].hands, 1.0, "{name} must receive exactly one observation");
    }
    assert_eq!(models.population.hands, 6.0);
}

/// splitmix64: a self-contained generator for the frame fuzzer (no test dependency).
struct Mix(u64);

impl Mix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Paths (object keys and array indices) to every value inside `v`.
fn paths(v: &Value, prefix: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                prefix.push(k.clone());
                out.push(prefix.clone());
                paths(child, prefix, out);
                prefix.pop();
            }
        }
        Value::Array(items) => {
            for (i, child) in items.iter().enumerate() {
                prefix.push(i.to_string());
                out.push(prefix.clone());
                paths(child, prefix, out);
                prefix.pop();
            }
        }
        _ => {}
    }
}

fn at_mut<'a>(v: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    path.iter().try_fold(v, |cur, key| match cur {
        Value::Object(map) => map.get_mut(key),
        Value::Array(items) => key.parse::<usize>().ok().and_then(|i| items.get_mut(i)),
        _ => None,
    })
}

/// One random corruption of a frame stream: a value replaced by a hostile one, a key removed, a
/// frame dropped, duplicated or moved.
fn mutate(frames: &mut Vec<Value>, rng: &mut Mix) {
    let hostile = |rng: &mut Mix| -> Value {
        match rng.below(10) {
            0 => Value::Null,
            1 => json!(-1),
            2 => json!(i64::MAX),
            3 => json!(i64::MIN),
            4 => json!(1e308),
            5 => json!("Zz"),
            6 => json!(""),
            7 => json!([]),
            8 => json!({}),
            _ => json!(rng.next() as i64 % 100_000),
        }
    };
    let i = rng.below(frames.len());
    match rng.below(6) {
        0 | 1 => {
            let mut ps = Vec::new();
            paths(&frames[i], &mut Vec::new(), &mut ps);
            if !ps.is_empty() {
                let p = ps[rng.below(ps.len())].clone();
                let value = hostile(rng);
                if let Some(slot) = at_mut(&mut frames[i], &p) {
                    *slot = value;
                }
            }
        }
        2 => {
            let mut ps = Vec::new();
            paths(&frames[i], &mut Vec::new(), &mut ps);
            if let Some(p) = ps.get(rng.below(ps.len())).cloned()
                && let Some((last, parent)) = p.split_last()
                && let Some(Value::Object(map)) = at_mut(&mut frames[i], parent)
            {
                map.remove(last);
            }
        }
        3 => {
            frames.remove(i);
        }
        4 => {
            let copy = frames[i].clone();
            frames.insert(rng.below(frames.len() + 1), copy);
        }
        _ => {
            let j = rng.below(frames.len());
            frames.swap(i, j);
        }
    }
}

/// Live dispatch over a (possibly corrupted) stream, including turn situations.
fn dispatch_all(frames: &[Value]) {
    let mut t = TableTracker::default();
    t.reset_table();
    for m in frames {
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
                let _ = t.hand_result(m);
            }
            "your_turn" => {
                let legal = LegalActions::parse(&m["valid_actions"]);
                if let Some(sit) = t.situation(m, &legal) {
                    // What the policy relies on: hero is seated and amounts are not negative.
                    let _ = sit.hero();
                    assert!(sit.pot >= 0 && sit.call_amount >= 0, "pot {} call {}", sit.pot, sit.call_amount);
                    assert!(sit.players.iter().all(|p| p.stack >= 0 && p.bet >= 0), "{:?}", sit.players);
                    assert!(sit.min_raise_to.is_none_or(|r| r > 0) && sit.max_raise_to.is_none_or(|r| r > 0));
                }
            }
            _ => {}
        }
    }
    let _ = replay(frames);
}

/// Server frames are untrusted input: no corruption of a real capture may panic the tracker
/// (a panic there ends a bot's session). Cases scale with `SV10_PROP_CASES`.
#[test]
fn corrupted_frame_streams_never_panic() {
    let base = frames();
    let cases: u64 = std::env::var("SV10_PROP_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(5_000);
    for case in 0..cases {
        let mut rng = Mix(0xf00d_0000 + case);
        let mut stream = base.clone();
        for _ in 0..1 + rng.below(6) {
            mutate(&mut stream, &mut rng);
        }
        let result = std::panic::catch_unwind(|| dispatch_all(&stream));
        assert!(result.is_ok(), "case {case} panicked; reproduce with Mix(0xf00d_0000 + {case})");
    }
}

/// 0180: the server's seat avatars reach the dashboard; only http(s) URLs and site paths are kept, and a snapshot
/// without the field keeps the last one.
#[test]
fn table_state_keeps_seat_avatars() {
    let mut t = TableTracker::default();
    let seat = |avatar: Value| json!({"seat": 2, "name": "villain", "stack": 1960, "bet": 0, "status": "active", "in_hand": true, "avatar_url": avatar});
    t.table_state(&json!({"table_id": "t1", "seats": [seat(json!("https://cdn.openpoker.ai/a/villain.png"))]}));
    assert_eq!(t.seats[&2].avatar_url.as_deref(), Some("https://cdn.openpoker.ai/a/villain.png"));
    t.table_state(&json!({"table_id": "t1", "seats": [{"seat": 2, "name": "villain", "stack": 1900, "status": "active"}]}));
    assert_eq!(t.seats[&2].avatar_url.as_deref(), Some("https://cdn.openpoker.ai/a/villain.png"));
    // The server sends its own avatars as site paths (2026-09-23 leaderboard: "/api/public-avatar/...").
    t.table_state(&json!({"table_id": "t1", "seats": [seat(json!("/api/public-avatar/avatars/a/b.webp"))]}));
    assert_eq!(t.seats[&2].avatar_url.as_deref(), Some("/api/public-avatar/avatars/a/b.webp"));
    for bad in ["javascript:alert(1)", "//evil.example/x.png", "data:image/png;base64,AA"] {
        t.table_state(&json!({"table_id": "t1", "seats": [seat(json!(bad))]}));
        assert_eq!(t.seats[&2].avatar_url, None, "{bad}");
    }
    t.table_state(&json!({"table_id": "t1", "seats": [seat(Value::Null)]}));
    assert_eq!(t.seats[&2].avatar_url, None);
}

#[test]
fn server_timestamps_parse_to_epoch_milliseconds() {
    // Reference values from Python's datetime.fromisoformat.
    assert_eq!(rfc3339_ms("2026-09-14T09:46:52.751076+00:00"), Some(1_789_379_212_751));
    assert_eq!(rfc3339_ms("2026-09-14T09:46:17.083997+00:00"), Some(1_789_379_177_083));
    assert_eq!(rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(rfc3339_ms("2000-02-29T23:59:59.5-05:30"), Some(951_888_599_500));
    for bad in ["", "2026-09-14", "2026-13-14T00:00:00Z", "2026-09-14T09:46:52", "yesterday at noon, give or take"] {
        assert_eq!(rfc3339_ms(bad), None, "{bad}");
    }
}

/// 0234: every opponent action carries its think time by the server's clock; the server paces an
/// action that follows another action (~3 s here) but not one that follows new cards.
#[test]
fn opponent_actions_carry_server_think_times() {
    let hands = replay(&frames());
    let history = &hands[0].1.summary.history;
    let hero = 0;
    assert!(history.iter().filter(|r| r.seat == hero).all(|r| r.think_ms.is_none()), "our own actions are not timed");
    let opp: Vec<&ActionRecord> = history.iter().filter(|r| r.seat != hero).collect();
    assert!(opp.iter().all(|r| r.think_ms.is_some()), "{opp:?}");
    // First to act on the flop, turn and river: tens of milliseconds after the cards.
    let open: Vec<u32> = opp.iter().filter(|r| r.street_open && r.street != Street::Preflop).map(|r| r.think_ms.unwrap()).collect();
    assert_eq!(open.len(), 3, "{opp:?}");
    assert!(open.iter().all(|&ms| ms < 100), "{open:?}");
    // After another action: the server's pacing, about three seconds.
    let paced: Vec<u32> = opp.iter().filter(|r| !r.street_open).map(|r| r.think_ms.unwrap()).collect();
    assert!(!paced.is_empty() && paced.iter().all(|&ms| (2_900..=3_200).contains(&ms)), "{paced:?}");
    // The flop check came 20 ms after the flop was dealt (09:46:17.083997 -> 17.104055; whole
    // milliseconds: 17.083 -> 17.104).
    let flop_check = opp.iter().find(|r| r.street == Street::Flop && r.kind == ActionKind::Check).unwrap();
    assert_eq!((flop_check.think_ms, flop_check.street_open), (Some(21), true));
}

#[test]
fn a_resync_or_a_gap_never_invents_a_think_time() {
    let mut t = TableTracker::default();
    t.reset_table();
    t.hero_seat = Some(0);
    t.hand_start(&json!({"type": "hand_start", "hand_id": "h", "dealer_seat": 1, "ts": "2026-09-14T09:00:00.000+00:00"}));
    t.player_action(&json!({"type": "player_action", "seat": 2, "action": "fold", "ts": "2026-09-14T09:00:03.000+00:00"}));
    assert_eq!(t.history.last().unwrap().think_ms, Some(3_000));
    // A resync clears the clock: the next action is untimed, the one after timed again.
    t.clear_event_clock();
    t.player_action(&json!({"type": "player_action", "seat": 3, "action": "fold", "ts": "2026-09-14T09:00:09.000+00:00"}));
    assert_eq!(t.history.last().unwrap().think_ms, None);
    t.player_action(&json!({"type": "player_action", "seat": 4, "action": "call", "ts": "2026-09-14T09:00:12.500+00:00"}));
    assert_eq!(t.history.last().unwrap().think_ms, Some(3_500));
    // More than a minute: a gap in the stream, not a decision.
    t.player_action(&json!({"type": "player_action", "seat": 5, "action": "call", "ts": "2026-09-14T09:05:00.000+00:00"}));
    assert_eq!(t.history.last().unwrap().think_ms, None);
    // No server time at all (older captures): untimed, and the clock is kept.
    t.player_action(&json!({"type": "player_action", "seat": 1, "action": "call"}));
    assert_eq!(t.history.last().unwrap().think_ms, None);
}

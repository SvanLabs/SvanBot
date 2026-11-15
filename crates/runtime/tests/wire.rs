//! The venue's wire format (<https://docs.openpoker.ai/llms-full.txt>, rev 2026-09-02, "WebSocket
//! Protocol"). Each inbound case is the docs' own example; each outbound case is the shape the docs
//! give.

use serde_json::{Value, json};
use svanbot_core::card::Card;
use svanbot_core::rules::RaiseRange;
use svanbot_runtime::wire::{Inbound, Offer, Outbound, VenueAction, parse, parse_frame};

fn card(text: &str) -> Card {
    text.parse().unwrap()
}

#[test]
fn a_turn_carries_the_hand_the_token_and_only_the_offered_actions() {
    let text = r#"{
      "type": "your_turn", "hand_id": "h-xyz789", "table_id": "t-abc123", "table_seq": 17,
      "valid_actions": [
        {"action": "fold"},
        {"action": "call", "amount": 20},
        {"action": "raise", "min": 40, "max": 2000},
        {"action": "all_in", "amount": 2000}
      ],
      "pot": 30, "community_cards": [],
      "players": [
        {"seat": 0, "name": "alpha_bot", "stack": 1980},
        {"seat": 2, "name": "my_bot", "stack": 1990}
      ],
      "min_raise": 40, "max_raise": 2000, "turn_token": "a1b2c3d4"
    }"#;
    let Inbound::YourTurn(turn) = parse(text).unwrap() else {
        panic!("not a turn")
    };
    assert_eq!(turn.hand_id.as_deref(), Some("h-xyz789"));
    assert_eq!(turn.turn_token, "a1b2c3d4");
    assert_eq!(turn.pot, 30);
    assert!(turn.board.is_empty());
    assert_eq!(
        turn.offer,
        Offer {
            fold: true,
            check: false,
            call: Some(20),
            raise: Some(RaiseRange { min: 40, max: 2000 }),
            all_in: Some(2000),
        }
    );
    assert_eq!(turn.players.len(), 2);
    assert_eq!(
        (
            turn.players[1].seat,
            turn.players[1].name.as_str(),
            turn.players[1].stack
        ),
        (2, "my_bot", 1990)
    );
}

#[test]
fn the_hand_and_its_cards_read_as_cards() {
    let start = parse(
        r#"{"type": "hand_start", "hand_id": "h-xyz789", "seat": 2, "dealer_seat": 0,
        "blinds": {"small_blind": 10, "big_blind": 20}}"#,
    )
    .unwrap();
    assert_eq!(
        start,
        Inbound::HandStart {
            hand_id: "h-xyz789".into(),
            seat: 2,
            dealer_seat: 0,
            small_blind: 10,
            big_blind: 20
        }
    );
    let hole = parse(r#"{"type": "hole_cards", "cards": ["Ah", "Kd"]}"#).unwrap();
    assert_eq!(
        hole,
        Inbound::HoleCards {
            cards: vec![card("Ah"), card("Kd")]
        }
    );
    let flop =
        parse(r#"{"type": "community_cards", "cards": ["7d", "Ts", "2c"], "street": "flop"}"#)
            .unwrap();
    assert_eq!(
        flop,
        Inbound::CommunityCards {
            cards: vec![card("7d"), card("Ts"), card("2c")]
        }
    );
}

#[test]
fn the_seat_list_frames_name_who_sits_where() {
    let joined = parse(r#"{"type": "table_joined", "table_id": "t-abc123", "seat": 2, "players": [
        {"seat": 0, "name": "alpha_bot", "stack": 2000}, {"seat": 2, "name": "my_bot", "stack": 2000}]}"#)
    .unwrap();
    let Inbound::TableJoined {
        table_id,
        seat,
        players,
    } = joined
    else {
        panic!("{joined:?}")
    };
    assert_eq!(
        (table_id.as_str(), seat, players[0].name.as_str()),
        ("t-abc123", 2, "alpha_bot")
    );
    let arrived =
        parse(r#"{"type": "player_joined", "seat": 4, "name": "new_bot", "stack": 2000}"#).unwrap();
    let Inbound::PlayerJoined(player) = arrived else {
        panic!("{arrived:?}")
    };
    assert_eq!((player.seat, player.name.as_str()), (4, "new_bot"));
    let left = parse(r#"{"type": "player_left", "seat": 4, "name": "new_bot", "reason": "left"}"#)
        .unwrap();
    assert_eq!(left, Inbound::PlayerLeft { seat: 4 });
}

#[test]
fn a_check_or_fold_has_a_null_amount_which_reads_as_zero() {
    let fold = parse(
        r#"{"type": "player_action", "seat": 0, "name": "alpha_bot", "action": "fold",
        "amount": null, "street": "preflop", "stack": 1960, "pot": 60}"#,
    )
    .unwrap();
    assert_eq!(
        fold,
        Inbound::PlayerAction {
            seat: 0,
            action: "fold".into(),
            amount: 0,
            stack: 1960
        }
    );
}

#[test]
fn a_hand_result_gives_the_final_stacks() {
    let result = parse(
        r#"{"type": "hand_result", "winners": [], "pot": 60,
        "final_stacks": {"0": 1960, "2": 2060}, "payouts": [{"seat": 2, "amount": 60}]}"#,
    )
    .unwrap();
    assert_eq!(
        result,
        Inbound::HandResult {
            final_stacks: vec![(0, 1960), (2, 2060)]
        }
    );
}

#[test]
fn errors_and_rejections_branch_on_their_code_and_unknown_frames_are_kept() {
    assert_eq!(
        parse(r#"{"type":"error","code":"auth_failed","message":"bad key"}"#).unwrap(),
        Inbound::Error {
            code: "auth_failed".into(),
            table_id: None
        }
    );
    assert_eq!(
        parse(r#"{"type": "action_rejected", "code": "not_your_turn", "reason": "Not your turn"}"#)
            .unwrap(),
        Inbound::ActionRejected {
            code: "not_your_turn".into()
        }
    );
    assert_eq!(
        parse(r#"{"type": "chips_skimmed", "excess": 500}"#).unwrap(),
        Inbound::Other {
            kind: "chips_skimmed".into()
        }
    );
    assert!(parse("not json").is_err());
    assert!(parse(r#"{"type": "hole_cards", "cards": ["Ah", "??"]}"#).is_err());
}

#[test]
fn an_action_echoes_the_hand_and_token_and_names_an_amount_only_for_a_raise() {
    let action = |action| {
        let text = Outbound::Action {
            hand_id: "h-1".into(),
            action,
            client_action_id: "c-1".into(),
            turn_token: "t-1".into(),
        }
        .to_json();
        serde_json::from_str::<Value>(&text).unwrap()
    };
    assert_eq!(
        action(VenueAction::Raise(60)),
        json!({"type": "action", "hand_id": "h-1", "action": "raise", "amount": 60,
               "client_action_id": "c-1", "turn_token": "t-1"})
    );
    assert_eq!(
        action(VenueAction::Call),
        json!({"type": "action", "hand_id": "h-1", "action": "call", "client_action_id": "c-1", "turn_token": "t-1"})
    );
    assert_eq!(action(VenueAction::AllIn)["action"], "all_in");
    let join: Value =
        serde_json::from_str(&Outbound::JoinLobby { buy_in: 2000 }.to_json()).unwrap();
    assert_eq!(join, json!({"type": "join_lobby", "buy_in": 2000}));
}

#[test]
fn a_frame_carries_its_table_seq_from_the_envelope() {
    let frame = parse_frame(
        r#"{"type": "hole_cards", "hand_id": "h-1", "table_seq": 18, "cards": ["Ah", "Kd"]}"#,
    )
    .unwrap();
    assert_eq!(frame.table_seq, Some(18));
    assert!(matches!(frame.inbound, Inbound::HoleCards { .. }));
    let bare = parse_frame(r#"{"type": "table_closed", "reason": "insufficient_players"}"#);
    assert_eq!(bare.unwrap().table_seq, None);
}

/// The docs' `table_state` example, as the actor sees it in a resync snapshot.
const SNAPSHOT: &str = r#"{
  "type": "table_state", "hand_id": "h-7", "street": "flop", "dealer_seat": 0,
  "small_blind": 10, "big_blind": 20, "pot": 120, "actor_seat": 2, "to_call": 40,
  "min_raise_to": 80, "max_raise_to": 1880, "board": ["7d", "Ts", "2c"],
  "seats": [
    {"seat": 0, "name": "alpha_bot", "stack": 1940, "bet": 0, "status": "active", "in_hand": true, "folded": true, "is_pro": false},
    {"seat": 1, "name": null, "stack": 0, "bet": 0, "status": "empty", "is_pro": false},
    {"seat": 2, "name": "my_bot", "stack": 1960, "bet": 40, "status": "active", "in_hand": true, "folded": false, "is_pro": true},
    {"seat": 4, "name": "beta_bot", "stack": 2000, "bet": 0, "status": "sitting_out", "is_pro": false}
  ],
  "hero": {
    "seat": 2, "hole_cards": ["Ah", "Kd"], "turn_token": "tok-9",
    "valid_actions": [
      {"action": "fold"}, {"action": "call", "amount": 40},
      {"action": "raise", "min": 80, "max": 1880}, {"action": "all_in", "amount": 1880}
    ]
  }
}"#;

#[test]
fn a_table_state_lists_the_occupied_seats_and_the_heros_cards_offer_and_token() {
    let Inbound::TableState(snapshot) = parse(SNAPSHOT).unwrap() else {
        panic!("not a snapshot")
    };
    assert_eq!(snapshot.hand_id.as_deref(), Some("h-7"));
    assert_eq!(snapshot.pot, 120);
    assert_eq!(snapshot.board, [card("7d"), card("Ts"), card("2c")]);
    let seats: Vec<_> = snapshot
        .seats
        .iter()
        .map(|seat| (seat.player.seat, seat.player.name.as_str(), seat.folded))
        .collect();
    assert_eq!(
        seats,
        [
            (0, "alpha_bot", Some(true)),
            (2, "my_bot", Some(false)),
            (4, "beta_bot", None)
        ]
    );
    let hero = snapshot.hero.unwrap();
    assert_eq!(hero.hole, [card("Ah"), card("Kd")]);
    assert_eq!(hero.turn_token.as_deref(), Some("tok-9"));
    assert_eq!(hero.offer.unwrap().call, Some(40));
}

#[test]
fn a_pushed_table_state_has_no_token_and_a_spectator_has_no_hero() {
    let pushed = r#"{"type": "table_state", "pot": 0, "board": [], "seats": [],
      "hero": {"seat": 2, "hole_cards": ["Ah", "Kd"]}}"#;
    let Inbound::TableState(snapshot) = parse(pushed).unwrap() else {
        panic!("not a snapshot")
    };
    let hero = snapshot.hero.unwrap();
    assert_eq!((hero.turn_token, hero.offer), (None, None));
    let Inbound::TableState(spectator) =
        parse(r#"{"type": "table_state", "pot": 0, "board": [], "seats": []}"#).unwrap()
    else {
        panic!("not a snapshot")
    };
    assert!(spectator.hero.is_none());
}

#[test]
fn a_resync_replays_its_events_in_sequence_order_then_gives_the_snapshot() {
    let snapshot: Value = serde_json::from_str(SNAPSHOT).unwrap();
    let text = json!({
        "type": "resync_response", "role": "player", "table_id": "t-1",
        "from_table_seq": 43, "to_table_seq": 50,
        "replayed_events": [
            {"type": "player_action", "table_seq": 47, "seat": 0, "action": "fold", "amount": null, "stack": 1940},
            {"type": "community_cards", "table_seq": 45, "cards": ["7d", "Ts", "2c"]}
        ],
        "snapshot": snapshot
    })
    .to_string();
    let Inbound::Resync(resync) = parse(&text).unwrap() else {
        panic!("not a resync")
    };
    assert!(resync.player);
    assert_eq!(resync.to_table_seq, Some(50));
    let seqs: Vec<_> = resync
        .replayed
        .iter()
        .map(|frame| frame.table_seq)
        .collect();
    assert_eq!(seqs, [Some(45), Some(47)]);
    assert_eq!(resync.snapshot.unwrap().hand_id.as_deref(), Some("h-7"));
    let spectator =
        parse(r#"{"type": "resync_response", "role": "spectator", "replayed_events": []}"#);
    let Inbound::Resync(spectator) = spectator.unwrap() else {
        panic!("not a resync")
    };
    assert!(!spectator.player && spectator.snapshot.is_none());
}

#[test]
fn already_seated_names_the_table_and_a_resync_request_names_the_watermark() {
    assert_eq!(
        parse(r#"{"type": "error", "code": "already_seated", "message": "seated", "table_id": "t-3", "seat": 1}"#)
            .unwrap(),
        Inbound::Error {
            code: "already_seated".into(),
            table_id: Some("t-3".into())
        }
    );
    let resync: Value = serde_json::from_str(
        &Outbound::ResyncRequest {
            table_id: "t-3".into(),
            last_table_seq: 42,
        }
        .to_json(),
    )
    .unwrap();
    assert_eq!(
        resync,
        json!({"type": "resync_request", "table_id": "t-3", "last_table_seq": 42})
    );
}

//! The venue's wire format (<https://docs.openpoker.ai/llms-full.txt>, rev 2026-09-02, "WebSocket
//! Protocol"). Each inbound case is the docs' own example; each outbound case is the shape the docs
//! give.

use serde_json::{Value, json};
use svanbot_core::card::Card;
use svanbot_core::rules::RaiseRange;
use svanbot_runtime::wire::{Inbound, Offer, Outbound, VenueAction, parse};

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
            code: "auth_failed".into()
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

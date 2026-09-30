use super::*;

#[test]
fn empty_recovery_board_keeps_known_cards_only_within_the_same_hand() {
    let mut t = TableTracker::default();
    t.reset_table();
    let mut turn = None;
    for m in frames() {
        match m["type"].as_str().unwrap() {
            "table_joined" => t.table_joined(&m),
            "table_state" => t.table_state(&m),
            "hand_start" => t.hand_start(&m),
            "hole_cards" => t.hole_cards(&m),
            "player_action" => t.player_action(&m),
            "your_turn" => turn = Some(m.clone()),
            "community_cards" => {
                t.community_cards(&m);
                if t.board.len() == 3 {
                    break;
                }
            }
            _ => {}
        }
    }
    let board = t.board.clone();
    assert_eq!(board.len(), 3);
    let mut turn = turn.unwrap();
    turn["community_cards"] = json!([]);
    let legal = LegalActions::parse(&turn["valid_actions"]);
    let sit = t.situation(&turn, &legal).unwrap();
    assert_eq!(sit.board, board);
    assert_eq!(sit.street, Street::Flop);
    assert_eq!(sit.call_amount, legal.call.unwrap_or(0));
    let snapshot = json!({"hand_id": t.hand_id, "street": "flop", "board": []});
    t.table_state(&snapshot);
    assert_eq!(t.board, board);
    t.hand_start(&json!({"hand_id": "next-hand", "dealer_seat": 0}));
    assert!(t.board.is_empty());
    // Empty arrays are still authoritative at the next hand's preflop frontier.
    t.table_state(&json!({"hand_id": "next-hand", "street": "preflop", "board": []}));
    assert!(t.board.is_empty());
}

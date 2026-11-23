//! Lines and stored hands: each seat's actions with their street, board, facing and size, and a
//! stored hand turned into the fit's observations.

use serde_json::json;
use svanbot_core::card::CardSet;
use svanbot_core::eval::strength;
use svanbot_core::rules::Street;
use svanbot_core::spot::{Act, ActKind};
use svanbot_learn::fit::Outcome;
use svanbot_runtime::history::observed;
use svanbot_runtime::lines::Lines;
use svanbot_runtime::record::Hand;
use svanbot_runtime::wire::parse;
use svanbot_strategy::action::Move;

fn cards(text: &str) -> CardSet {
    text.parse().unwrap()
}

/// Seat 2 is us. Seat 0 raises, calls the flop, bets the river and shows; seat 4 calls to the
/// showdown and mucks; seat 5 folds preflop.
fn frames() -> Vec<String> {
    let action = |seat: u32,
                  action: &str,
                  amount: Option<u32>,
                  added: u32,
                  pot: u32,
                  to_call: u32| {
        json!({"type": "player_action", "seat": seat, "action": action, "amount": amount,
               "stack": 1000, "contribution_delta": added, "pot_before": pot,
               "to_call_before": if to_call == 0 { serde_json::Value::Null } else { json!(to_call) }})
    };
    [
        json!({"type": "hand_start", "hand_id": "h-1", "seat": 2, "dealer_seat": 0,
               "blinds": {"small_blind": 10, "big_blind": 20}}),
        json!({"type": "hole_cards", "hand_id": "h-1", "cards": ["3c", "3d"]}),
        action(0, "raise", Some(60), 60, 30, 20),
        action(5, "fold", None, 0, 90, 60),
        action(4, "call", Some(60), 50, 90, 50),
        action(2, "call", Some(60), 40, 140, 40),
        json!({"type": "community_cards", "cards": ["Ks", "9d", "4c"], "street": "flop"}),
        action(2, "check", None, 0, 180, 0),
        action(0, "bet", Some(60), 60, 180, 0),
        action(4, "call", Some(60), 60, 240, 60),
        action(2, "call", Some(60), 60, 300, 60),
        json!({"type": "community_cards", "cards": ["7h"], "street": "turn"}),
        action(2, "check", None, 0, 360, 0),
        action(0, "check", None, 0, 360, 0),
        action(4, "check", None, 0, 360, 0),
        json!({"type": "community_cards", "cards": ["2s"], "street": "river"}),
        action(2, "check", None, 0, 360, 0),
        action(0, "bet", Some(400), 400, 360, 0),
        action(4, "call", Some(400), 400, 760, 400),
        action(2, "call", Some(400), 400, 1160, 400),
        json!({"type": "hand_result", "final_stacks": {"0": 2000}, "shown_cards": {"0": ["Kd", "Kh"]},
               "winners": [{"seat": 0}]}),
    ]
    .map(|frame| frame.to_string())
    .to_vec()
}

#[test]
fn a_repeated_community_cards_frame_keeps_the_board_in_deal_order_once() {
    let mut lines = Lines::default();
    let flop = parse(r#"{"type":"community_cards","cards":["Ah","Kd","7c"]}"#).unwrap();
    lines.on(&flop);
    lines.on(&flop);
    let shown: Vec<String> = lines
        .board_order()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(shown, ["Ah", "Kd", "7c"]);
}

#[test]
fn each_seat_s_line_has_its_street_board_facing_and_size() {
    let mut lines = Lines::default();
    for text in frames() {
        lines.on(&parse(&text).unwrap());
    }
    let raiser = lines.of(0);
    assert_eq!(raiser.len(), 4);
    assert_eq!(
        raiser[0],
        Act {
            street: Street::Preflop,
            board: CardSet::EMPTY,
            facing: true,
            kind: ActKind::Bet { added: 60, pot: 30 }
        }
    );
    assert_eq!(raiser[1].street, Street::Flop);
    assert_eq!(raiser[1].board, cards("Ks9d4c"));
    assert!(!raiser[1].facing);
    assert_eq!(raiser[3].board, cards("Ks9d4c7h2s"));
    let dealt: Vec<String> = lines
        .board_order()
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(dealt, ["Ks", "9d", "4c", "7h", "2s"]);
    assert_eq!(
        raiser[3].kind,
        ActKind::Bet {
            added: 400,
            pot: 360
        }
    );
    assert!(lines.folded(5) && !lines.folded(4));
    // The river: each seat's chips in on the street, and the big blind of the hand.
    let dealt = [0, 2, 4, 5];
    assert_eq!(
        (lines.committed(0, &dealt), lines.committed(5, &dealt)),
        (400, 0)
    );
    assert_eq!(lines.big_blind(), 20);
}

#[test]
fn a_posted_blind_counts_in_what_its_seat_has_put_in_before_the_flop() {
    // The venue posts the small blind left of the dealer and the big blind two left; heads up
    // the dealer posts the small one. A raise-to is committed plus what the seat adds.
    let frame = |value: serde_json::Value| parse(&value.to_string()).unwrap();
    let start = |dealer: u32| {
        frame(json!({"type": "hand_start", "hand_id": "h", "seat": 2, "dealer_seat": dealer,
                     "blinds": {"small_blind": 10, "big_blind": 20}}))
    };
    let mut lines = Lines::default();
    lines.on(&start(5));
    let dealt = [0, 2, 4, 5];
    let posted: Vec<u32> = dealt
        .iter()
        .map(|&seat| lines.committed(seat, &dealt))
        .collect();
    assert_eq!(posted, [10, 20, 0, 0]);
    lines.on(&frame(json!({"type": "player_action", "seat": 0, "action": "call",
                           "amount": 20, "stack": 1980, "contribution_delta": 10})));
    assert_eq!(lines.committed(0, &dealt), 20);
    lines.on(&frame(json!({"type": "community_cards", "street": "flop",
                           "cards": ["Ks", "9d", "4c"]})));
    assert_eq!(lines.committed(2, &dealt), 0);
    // Heads up, the dealer is the small blind.
    lines.on(&start(4));
    assert_eq!(
        (lines.committed(4, &[2, 4]), lines.committed(2, &[2, 4])),
        (10, 20)
    );
}

#[test]
fn a_stored_hand_gives_a_shown_a_mucked_and_an_unseen_opponent_and_never_us() {
    let hand = Hand {
        hand_id: "h-1".into(),
        table_id: "t-1".into(),
        frames: frames(),
        traced: Vec::new(),
    };
    let sides = observed(&hand, 7);
    assert_eq!(sides.len(), 3, "seats 0, 4 and 5; not ours");
    let board = cards("Ks9d4c7h2s");
    let [shown, mucked, folded] = &sides[..] else {
        unreachable!()
    };
    assert_eq!(shown.outcome, Outcome::Shown(cards("KdKh")));
    assert_eq!(shown.seen.last().unwrap().mv, Move::Large);
    // Seat 4 called to the showdown and showed nothing: a pair that lost to the shown set.
    assert_eq!(
        mucked.outcome,
        Outcome::Mucked {
            board,
            winner: strength(cards("KdKh") | board)
        }
    );
    assert_eq!(folded.outcome, Outcome::Unseen);
    assert_eq!(folded.seen.len(), 1);
    for side in &sides {
        assert_eq!(side.hand, 7);
        assert!(
            side.dead.contains("3c".parse().unwrap()),
            "our cards are dead"
        );
    }
    // The shown pair is dead for the others, not for its owner.
    assert!(mucked.dead.contains("Kd".parse().unwrap()));
    assert!(!shown.dead.contains("Kd".parse().unwrap()));
}

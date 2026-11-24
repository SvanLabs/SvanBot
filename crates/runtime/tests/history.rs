//! Lines and stored hands: each seat's actions with their street, board, facing and size, and a
//! stored hand turned into the fit's observations.

#[allow(clippy::disallowed_methods)]
mod support;

use serde_json::json;
use svanbot_core::card::CardSet;
use svanbot_core::eval::strength;
use svanbot_core::rules::Street;
use svanbot_core::spot::{Act, ActKind};
use svanbot_learn::fit::Outcome;
use svanbot_runtime::history::{dossiers, observed, reads, witnesses};
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
                  amount: Option<u64>,
                  added: u64,
                  pot: u64,
                  to_call: u64| {
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

/// Seat 2 is us. Seat 0 raises, seat 5 folds to it, seat 4 calls, then on the flop seat 0 bets,
/// seat 4 folds and on the turn we bet and seat 0 folds. The names come only from the actions.
fn named_frames() -> Vec<String> {
    let action = |seat: u32,
                  name: Option<&str>,
                  action: &str,
                  added: u64,
                  pot: u64,
                  to_call: u64| {
        json!({"type": "player_action", "seat": seat, "name": name, "action": action,
               "amount": if added == 0 { serde_json::Value::Null } else { json!(added) },
               "stack": 1000, "contribution_delta": added, "pot_before": pot,
               "to_call_before": if to_call == 0 { serde_json::Value::Null } else { json!(to_call) }})
    };
    [
        json!({"type": "hand_start", "hand_id": "h-2", "seat": 2, "dealer_seat": 0,
               "blinds": {"small_blind": 10, "big_blind": 20}}),
        json!({"type": "hole_cards", "hand_id": "h-2", "cards": ["3c", "3d"]}),
        action(0, Some("alpha_bot"), "raise", 60, 30, 20),
        action(5, Some("gamma_bot"), "fold", 0, 90, 60),
        action(4, Some("beta_bot"), "call", 50, 90, 50),
        action(2, Some("SurSvan"), "call", 40, 140, 40),
        json!({"type": "community_cards", "cards": ["Ks", "9d", "4c"], "street": "flop"}),
        action(2, Some("SurSvan"), "check", 0, 180, 0),
        action(0, Some("alpha_bot"), "bet", 60, 180, 0),
        action(4, Some("beta_bot"), "fold", 0, 240, 60),
        json!({"type": "community_cards", "cards": ["7h"], "street": "turn"}),
        action(2, Some("SurSvan"), "bet", 200, 240, 0),
        action(0, Some("alpha_bot"), "fold", 0, 440, 200),
        json!({"type": "hand_result", "final_stacks": {"2": 2400}, "shown_cards": {},
               "winners": [{"seat": 2}]}),
    ]
    .map(|frame| frame.to_string())
    .to_vec()
}

#[test]
fn a_hand_witnesses_each_named_opponent_and_never_us() {
    let hand = Hand {
        hand_id: "h-2".into(),
        table_id: "t-1".into(),
        frames: named_frames(),
        traced: Vec::new(),
    };
    let seen = witnesses(&hand, "SurSvan");
    let names: Vec<&str> = seen.iter().map(|witness| witness.name.as_str()).collect();
    assert_eq!(names, ["alpha_bot", "beta_bot", "gamma_bot"]);
    let [alpha, beta, gamma] = &seen[..] else {
        unreachable!()
    };
    assert_eq!(alpha.actions.len(), 3);
    assert_eq!(alpha.actions[0].mv, Move::Large);
    assert!(alpha.aggressive);
    assert_eq!(alpha.vs_us, Some(true), "alpha folded to our turn bet");
    assert_eq!(alpha.vs_others, None);
    assert!(!beta.aggressive);
    assert_eq!(beta.vs_us, None);
    assert_eq!(
        beta.vs_others,
        Some(false),
        "beta first answered alpha's raise with a call"
    );
    assert!(!gamma.aggressive);
    assert_eq!(gamma.vs_others, Some(true), "gamma folded to alpha's raise");
    assert_eq!(gamma.vs_us, None);
}

#[test]
fn a_hand_that_names_no_seat_witnesses_nobody() {
    let hand = Hand {
        hand_id: "h-1".into(),
        table_id: "t-1".into(),
        frames: frames(),
        traced: Vec::new(),
    };
    assert!(witnesses(&hand, "SurSvan").is_empty());
}

#[test]
fn a_bot_s_stored_hands_give_its_reads_of_each_named_opponent() {
    let hand = |id: &str| Hand {
        hand_id: id.into(),
        table_id: "t-1".into(),
        frames: named_frames(),
        traced: Vec::new(),
    };
    let reads = reads(&[hand("h-2"), hand("h-3")], "SurSvan");
    let names: Vec<&str> = reads.opponents().map(|opponent| opponent.name()).collect();
    assert_eq!(names, ["alpha_bot", "beta_bot", "gamma_bot"]);
    assert_eq!(reads.get("alpha_bot").unwrap().hands(), 2);
    assert_eq!(reads.get("alpha_bot").unwrap().actions(), 6);
    assert!(reads.get("SurSvan").is_none());
    // The status carries each one as aggregates: two hands are too few to say how they adapt.
    let rows = dossiers(&reads);
    assert_eq!(rows.len(), 3);
    assert_eq!(
        (rows[0].name.as_str(), rows[0].class.as_str(), rows[0].hands),
        ("alpha_bot", "unknown", 2)
    );
    assert_eq!(
        (rows[0].actions, rows[0].earned_pct, rows[0].change),
        (6, 0, None)
    );
}

#[test]
fn only_a_bots_own_stored_hands_qualify_profile_stats() {
    use svanbot_strategy::profile::MIN_EVIDENCE_HANDS;

    let played: Vec<_> = (0..MIN_EVIDENCE_HANDS)
        .map(|n| Hand {
            hand_id: format!("h-{n}"),
            table_id: "t-1".into(),
            frames: named_frames(),
            traced: Vec::new(),
        })
        .collect();
    let full = reads(&played, "SurSvan");
    let thin = reads(&played[..1], "SurSvan");
    let seated = [(0, "ALPHA_BOT"), (4, "beta_bot")];
    let [alpha, beta] = full
        .profile_stats(&seated)
        .try_into()
        .unwrap_or_else(|_| panic!("two seats"));
    assert_eq!((alpha.seat, alpha.stats.pfr.value()), (0, Some(1.0)));
    assert_eq!((beta.seat, beta.stats.vpip.value()), (4, Some(1.0)));
    assert_eq!(beta.stats.fold_to_bet.value(), Some(1.0));
    assert_eq!(
        thin.profile_stats(&seated)[1].stats.fold_to_bet.value(),
        None
    );
}

#[test]
fn live_reads_match_the_store_after_a_duplicate_or_failed_hand_write() {
    use svanbot_runtime::history::keep_live;
    use svanbot_runtime::store::Store;

    let path = support::scratch("live-reads").join("bot.db");
    let bot = "SurSvan";
    let store = Store::open(&path, bot).unwrap();
    let mut live = svanbot_learn::reads::Reads::default();
    let mut hand = Hand {
        hand_id: "h-2".into(),
        table_id: "t-1".into(),
        frames: named_frames(),
        traced: Vec::new(),
    };
    assert!(keep_live(&store, &mut live, &hand, bot, 1).unwrap());
    assert_eq!(live.get("alpha_bot").unwrap().hands(), 1);
    assert!(!keep_live(&store, &mut live, &hand, bot, 2).unwrap());

    hand.hand_id = "h-3".into();
    let reader = Store::open_reader(&path).unwrap();
    assert!(keep_live(&reader, &mut live, &hand, bot, 3).is_err());
    assert_eq!(live, reads(&store.hands().unwrap(), bot));
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
        frame(
            json!({"type": "hand_start", "hand_id": "h", "seat": 2, "dealer_seat": dealer,
                     "blinds": {"small_blind": 10, "big_blind": 20}}),
        )
    };
    let mut lines = Lines::default();
    lines.on(&start(5));
    let dealt = [0, 2, 4, 5];
    let posted: Vec<u64> = dealt
        .iter()
        .map(|&seat| lines.committed(seat, &dealt))
        .collect();
    assert_eq!(posted, [10, 20, 0, 0]);
    lines.on(&frame(
        json!({"type": "player_action", "seat": 0, "action": "call",
                           "amount": 20, "stack": 1980, "contribution_delta": 10}),
    ));
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

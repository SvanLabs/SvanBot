//! A hand's trace and its frames each keep the cards, and this is the check that they agree
//! (V11-24, #1102). The trace's copy lets the replayer work without a frame parser; the frames are
//! the venue's own JSON; a drift between the two is invisible without this.
use serde_json::json;
use svanbot_core::card::CardSet;
use svanbot_core::rules::{Action, Street};
use svanbot_runtime::record::{Hand, Recorder};
use svanbot_runtime::session::Session;
use svanbot_runtime::wire::parse_frame;
use svanbot_strategy::policy::{
    Abstract, Branch, Decision, PricedTrace, Ranged, ResolvedTrace, Trace,
};

/// The roster the session guards: a bot at this table with a stablemate would leave.
const ROSTER: [&str; 5] = ["SvanBotV10", "Svanar", "Svanism", "SuraGunnar", "SurSvan"];

#[test]
fn a_trace_and_its_frames_that_disagree_about_the_cards_name_the_disagreement() {
    // The trace keeps its own copy of the cards so the replayer needs no frame parser, and the
    // frames keep the venue's JSON. Two copies can drift apart, and a drift is invisible: the
    // trace comes from the decision path, the frames from the wire, and nothing else compares
    // them. This says which decision disagrees and about what.
    let hand = Hand {
        hand_id: "h-1".into(),
        table_id: "t-1".into(),
        frames: vec![
            json!({"type": "hole_cards", "cards": ["Ah", "As"]}).to_string(),
            json!({"type": "community_cards", "cards": ["Qh"], "street": "flop"}).to_string(),
            json!({"type": "community_cards", "cards": ["Jh"], "street": "turn"}).to_string(),
            json!({"type": "community_cards", "cards": ["Th"], "street": "river"}).to_string(),
        ],
        // A preflop decision with the frames' hole cards and no board, then a flop decision on
        // `QhJhTh` that says its hole was `AhKh`, which the frames never dealt.
        traced: vec![trace(Branch::Priced), trace(Branch::Resolved)],
    };
    assert_eq!(hand.traced[0].street, Street::Preflop);
    assert_eq!(hand.traced[0].board, CardSet::EMPTY);
    assert_eq!(hand.traced[1].street, Street::Flop);
    assert_eq!(hand.traced[1].board, "QhJhTh".parse().unwrap());
    // The priced trace is an `AhAs` preflop decision and the frames agree there; the resolved one
    // says `AhKh` on `QhJhTh`, and the frames' hole cards are `AhAs`, so only its hole differs.
    let wrong = hand.cards_disagree();
    assert_eq!(wrong, vec![(1, "hole")]);

    // Made to agree: a preflop decision with the frames' hole cards and no board yet.
    let mut agree = hand.clone();
    agree.traced[1].hole = "AhAs".parse().unwrap();
    agree.traced[1].board = CardSet::EMPTY;
    agree.traced[1].street = Street::Preflop;
    assert!(agree.cards_disagree().is_empty());
}

#[test]
fn a_hand_whose_frames_name_no_cards_disagrees_with_nothing() {
    // A hand a download produced without card frames, or a trace made before the hand opened:
    // there is nothing to compare, and a missing card is not a disagreement about one.
    let mut hand = Hand {
        hand_id: "h-1".into(),
        table_id: "t-1".into(),
        frames: vec![
            json!({"type": "hand_start", "hand_id": "h-1", "seat": 2,
                            "dealer_seat": 0,
                            "blinds": {"small_blind": 10, "big_blind": 20}})
            .to_string(),
        ],
        traced: vec![trace(Branch::Priced)],
    };
    // The frames parsed and say nothing about cards, which is not a claim that there were none.
    assert!(hand.cards_disagree().is_empty());
    hand.frames = Vec::new();
    assert!(hand.cards_disagree().is_empty());
}

#[test]
fn a_live_hand_through_the_session_and_recorder_states_cards_that_agree_with_its_frames() {
    // The case the ticket asks to pin, end to end: the real session builds the spot from the real
    // frames, the policy traces what it saw, and the recorder keeps trace and frames together. If
    // the two ever describe different cards, this is where it would show first.
    let roster = ROSTER.map(String::from);
    let mut session = Session::new("SurSvan", roster.iter().cloned());
    let mut recorder = Recorder::default();
    let frames = [
        json!({"type": "table_joined", "table_id": "t-9", "seat": 2, "players": []}),
        json!({"type": "hand_start", "hand_id": "h-5", "seat": 2, "dealer_seat": 0,
               "blinds": {"small_blind": 10, "big_blind": 20}}),
        json!({"type": "hole_cards", "hand_id": "h-5", "cards": ["Ah", "As"]}),
        json!({"type": "your_turn", "hand_id": "h-5", "turn_token": "tok-1", "pot": 30,
        "community_cards": [],
        "players": [{"seat": 0, "name": "alpha_bot", "stack": 1980},
                    {"seat": 2, "name": "SurSvan", "stack": 1990}],
        "valid_actions": [
            {"action": "fold"},
            {"action": "call", "amount": 20},
            {"action": "raise", "min": 40, "max": 2000},
        ]}),
        json!({"type": "hand_result", "hand_id": "h-5", "final_stacks": {"2": 2010}}),
    ];
    let mut finished = None;
    for text in frames.map(|frame| frame.to_string()) {
        let frame = parse_frame(&text).expect("the venue's frame");
        if let Some(hand) = recorder.record(&text, &frame) {
            finished = Some(hand);
        }
        let step = session.on_frame(frame, |spot| {
            // What the session handed the policy: the decision path's view of the cards.
            assert_eq!(spot.hole, "AhAs".parse().unwrap());
            Decision {
                action: Action::Call,
                drawn: None,
                trace: Some(Trace {
                    branch: Branch::Priced,
                    street: Street::Preflop,
                    hole: spot.hole,
                    board: spot.board,
                    resolved: None,
                    priced: Some(PricedTrace {
                        equity: 0.7,
                        price: Some(0.25),
                        exploit: None,
                        ranges: vec![],
                    }),
                }),
            }
        });
        recorder.trace(step.traced);
    }
    let hand = finished.expect("the hand closed");
    assert_eq!(hand.traced.len(), 1, "the one decision was traced");
    assert_eq!(hand.cards_disagree(), vec![]);
}

fn trace(branch: Branch) -> Trace {
    match branch {
        Branch::Priced => Trace {
            branch,
            street: Street::Preflop,
            hole: "AhAs".parse().expect("a hole"),
            board: CardSet::EMPTY,
            resolved: None,
            priced: Some(PricedTrace {
                equity: 0.61,
                price: Some(0.25),
                exploit: Some(0.4),
                ranges: vec![Ranged {
                    seat: Some(2),
                    masses: vec![0.0, 0.0, 0.1, 0.2, 0.3, 0.2, 0.1, 0.1],
                }],
            }),
        },
        Branch::Resolved => Trace {
            branch,
            street: Street::Flop,
            hole: "AhKh".parse().expect("a hole"),
            board: "QhJhTh".parse().expect("a board"),
            resolved: Some(ResolvedTrace {
                values: vec![1.5, -0.25],
                strategy: vec![0.75, 0.25],
                taken: 0,
                actions: vec![Abstract::Check, Abstract::Bet(200)],
                ranges: vec![Ranged {
                    seat: Some(2),
                    masses: vec![0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                }],
            }),
            priced: None,
        },
    }
}

/// A decision on one street with the cards it says it saw, for the reconnect cases.
fn trace_on(street: Street, hole: &str, board: &str) -> Trace {
    Trace {
        branch: Branch::Priced,
        street,
        hole: hole.parse().expect("a hole"),
        board: board.parse().expect("a board"),
        resolved: None,
        priced: Some(PricedTrace {
            equity: 0.6,
            price: Some(0.25),
            exploit: None,
            ranges: vec![],
        }),
    }
}

#[test]
fn a_reconnect_that_replays_the_cards_is_not_a_disagreement() {
    // A dropped connection is not a bug. `client.rs` keeps the same `Recorder` across a reconnect
    // precisely so the hand finishes, and the venue sends the missed cards inside
    // `resync_response`: either replayed events or the snapshot's board. A checker that read only
    // top-level `hole_cards` and `community_cards` frames would call every reconnect a crit-level
    // disagreement, which is a lie loud enough to hide the real ones.
    let frames = [
        json!({"type": "hole_cards", "hand_id": "h-7", "cards": ["Ah", "As"]}),
        json!({"type": "resync_response", "role": "player", "table_seq": 12,
        "replayed_events": [],
        "snapshot": {
            "hand_id": "h-7", "pot": 60, "board": ["Qh", "Jh", "Th"],
            "seats": [], "hero": {"seat": 2, "hole_cards": ["Ah", "As"]}
        }}),
    ];
    let hand = Hand {
        hand_id: "h-7".into(),
        table_id: "t-1".into(),
        frames: frames.iter().map(|f| f.to_string()).collect(),
        traced: vec![trace_on(Street::Flop, "AhAs", "QhJhTh")],
    };
    assert_eq!(hand.cards_disagree(), vec![]);
}

#[test]
fn a_reconnect_that_replays_several_streets_reaches_each_decisions_street() {
    // `already_seated` asks for a resync from the start (`last_table_seq: 0`), so the replayed
    // events carry the flop AND the turn, and the snapshot's board is the turn. A checker that
    // folded every replayed card in at once, then compared, would jump the board from empty to
    // the turn and call the flop decision "unreached" — a false alarm on a normal reconnect.
    let frames = [
        json!({"type": "hole_cards", "hand_id": "h-8", "cards": ["Ah", "As"]}),
        json!({"type": "resync_response", "role": "player", "table_seq": 20,
        "replayed_events": [
            {"type": "community_cards", "table_seq": 14, "street": "flop",
             "cards": ["Qh", "Jh", "Th"]},
            {"type": "community_cards", "table_seq": 18, "street": "turn",
             "cards": ["2c"]}
        ],
        "snapshot": {
            "hand_id": "h-8", "pot": 200, "board": ["Qh", "Jh", "Th", "2c"],
            "seats": [], "hero": {"seat": 2, "hole_cards": ["Ah", "As"]}
        }}),
    ];
    let hand = Hand {
        hand_id: "h-8".into(),
        table_id: "t-1".into(),
        frames: frames.iter().map(|f| f.to_string()).collect(),
        traced: vec![
            trace_on(Street::Preflop, "AhAs", ""),
            trace_on(Street::Flop, "AhAs", "QhJhTh"),
            trace_on(Street::Turn, "AhAs", "QhJhTh2c"),
        ],
    };
    assert_eq!(hand.cards_disagree(), vec![]);
}

#[test]
fn a_decision_compared_before_its_hole_cards_arrived_is_not_blamed_for_them() {
    // A replay can list the board before the hole cards (a `hole_cards` event replayed after a
    // `community_cards` one, or a snapshot whose hero arrives without them). Comparing a preflop
    // decision at the moment the board shows would then call its `AhAs` wrong against an empty
    // hole. The preflop decision is compared against the hole cards the hand actually had, which
    // arrive later; only the street's board has to be there for the comparison to happen.
    let frames = [
        json!({"type": "resync_response", "role": "player", "table_seq": 5,
               "replayed_events": [
                   {"type": "community_cards", "table_seq": 4, "street": "preflop", "cards": []}
               ],
               "snapshot": {"hand_id": "h-9", "pot": 30, "board": [],
                            "seats": [], "hero": {"seat": 2, "hole_cards": []}}}),
        json!({"type": "hole_cards", "hand_id": "h-9", "cards": ["Ah", "As"]}),
    ];
    let hand = Hand {
        hand_id: "h-9".into(),
        table_id: "t-1".into(),
        frames: frames.iter().map(|f| f.to_string()).collect(),
        traced: vec![trace_on(Street::Preflop, "AhAs", "")],
    };
    assert_eq!(hand.cards_disagree(), vec![]);
}

#[test]
fn hole_cards_that_arrive_late_and_wrong_are_still_caught() {
    // The waiting above must not become a way to pass by never arriving: when the hole cards do
    // come and differ from the trace, that is the disagreement the check exists to find.
    let frames = [
        json!({"type": "resync_response", "role": "player", "table_seq": 5,
               "replayed_events": [], "snapshot": {"hand_id": "h-9", "pot": 30, "board": [],
                                                   "seats": [], "hero": {"seat": 2,
                                                                          "hole_cards": []}}}),
        json!({"type": "hole_cards", "hand_id": "h-9", "cards": ["Ah", "Kh"]}),
    ];
    let hand = Hand {
        hand_id: "h-9".into(),
        table_id: "t-1".into(),
        frames: frames.iter().map(|f| f.to_string()).collect(),
        traced: vec![trace_on(Street::Preflop, "AhAs", "")],
    };
    assert_eq!(hand.cards_disagree(), vec![(0, "hole")]);
}

#[test]
fn a_reconnect_whose_replayed_board_disagrees_is_still_a_disagreement() {
    // The reconnect is not a free pass: cards that arrive through it are compared like any other.
    let frames = [
        json!({"type": "hole_cards", "hand_id": "h-7", "cards": ["Ah", "As"]}),
        json!({"type": "resync_response", "role": "player", "table_seq": 12,
        "replayed_events": [
            {"type": "community_cards", "table_seq": 10, "cards": ["Qh", "Jh", "9c"],
             "street": "flop"}
        ]}),
    ];
    let hand = Hand {
        hand_id: "h-7".into(),
        table_id: "t-1".into(),
        frames: frames.iter().map(|f| f.to_string()).collect(),
        traced: vec![trace_on(Street::Flop, "AhAs", "QhJhTh")],
    };
    assert_eq!(hand.cards_disagree(), vec![(0, "board")]);
}

#[test]
fn a_live_reconnect_through_the_session_and_recorder_is_not_a_disagreement() {
    // The verifier's own case, end to end: the bot reconnects mid-hand and the flop arrives only
    // inside the resync. The policy decides on that flop, from the snapshot's own view, and the
    // hand must not be reported as a disagreement.
    let roster = ROSTER.map(String::from);
    let mut session = Session::new("SurSvan", roster.iter().cloned());
    let mut recorder = Recorder::default();
    let frames = [
        json!({"type": "table_joined", "table_id": "t-9", "seat": 2, "players": []}),
        json!({"type": "hand_start", "hand_id": "h-7", "seat": 2, "dealer_seat": 0,
               "blinds": {"small_blind": 10, "big_blind": 20}}),
        json!({"type": "hole_cards", "hand_id": "h-7", "cards": ["Ah", "As"]}),
        json!({"type": "resync_response", "role": "player", "from_table_seq": 3,
        "to_table_seq": 12, "replayed_events": [
            {"type": "community_cards", "table_seq": 10, "street": "flop",
             "cards": ["Qh", "Jh", "Th"]}
        ],
        "snapshot": {
            "hand_id": "h-7", "pot": 60, "board": ["Qh", "Jh", "Th"],
            "seats": [{"seat": 0, "name": "alpha_bot", "stack": 1980, "status": "active"},
                      {"seat": 2, "name": "SurSvan", "stack": 1990, "status": "active"}],
            "hero": {"seat": 2, "hole_cards": ["Ah", "As"], "turn_token": "tok-2",
                     "valid_actions": [{"action": "check"},
                                       {"action": "raise", "min": 40, "max": 2000}]}
        }}),
        json!({"type": "hand_result", "hand_id": "h-7", "final_stacks": {"2": 2050}}),
    ];
    let mut finished = None;
    let mut saw_flop = false;
    for text in frames.map(|frame| frame.to_string()) {
        let frame = parse_frame(&text).expect("the venue's frame");
        if let Some(hand) = recorder.record(&text, &frame) {
            finished = Some(hand);
        }
        let step = session.on_frame(frame, |spot| {
            if spot.board == "QhJhTh".parse().unwrap() {
                saw_flop = true;
            }
            Decision {
                action: Action::Check,
                drawn: None,
                trace: Some(Trace {
                    branch: Branch::Priced,
                    street: Street::of(spot.board),
                    hole: spot.hole,
                    board: spot.board,
                    resolved: None,
                    priced: Some(PricedTrace {
                        equity: 0.6,
                        price: None,
                        exploit: None,
                        ranges: vec![],
                    }),
                }),
            }
        });
        recorder.trace(step.traced);
    }
    let hand = finished.expect("the hand closed");
    assert!(saw_flop, "the resync's flop reached a decision");
    assert_eq!(hand.cards_disagree(), vec![]);
}

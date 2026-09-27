//! The differential test (0263): the same decision, built the two ways the project builds it.
//!
//! Live play prices a turn from `TableTracker::situation` (reconstructed from server frames); every
//! simulation, the property suite, the learner and the golden snapshot price it from
//! `Situation::from_hand`. The two share no code, so a drift between them means the bot plays
//! something nobody has ever tested while the learner tunes a different game — and nothing in the
//! test suite would see it, because the properties only ever exercise the second path.
//!
//! This builds a scripted hand in the engine, emits the frames the server would send for it, drives
//! the tracker with them, and asserts the two `Situation`s agree field by field.

use super::*;
use serde_json::json;
use sv10_cards::cards::Card;
use sv10_engine::engine::{Action, ActionKind, Hand};
use sv10_engine::situation::PlayerInfo;
use sv10_engine::situation::Situation as EngineSituation;

/// Six seats, blinds 10/20, fixed holes so the frame script is deterministic.
fn table(hero: usize) -> (Hand, Vec<String>) {
    let holes = [["Ah", "Kd"], ["Qc", "8d"], ["7h", "2c"], ["6h", "8d"], ["As", "Td"], ["9c", "2d"]];
    let seats = holes.iter().map(|h| Hand::seat_state(2_000, [Card::parse(h[0]).unwrap(), Card::parse(h[1]).unwrap()])).collect();
    let runout: [Card; 5] = sv10_cards::cards::parse_cards(&["Kh", "7s", "2d", "9h", "4c"]).unwrap().try_into().unwrap();
    let names: Vec<String> = (0..6).map(|i| format!("p{i}")).collect();
    // The button is the seat before the hero, so the hero's position is fixed too.
    let button = (hero + 5) % 6;
    (Hand::with_cards(seats, runout, button, 10, 20), names)
}

/// The frames the server would send for `hand` up to (and including) its current actor, built from
/// the hand's own record so the numbers cannot drift from what the engine believes.
fn script(hand: &Hand, hero: usize) -> (Vec<Value>, Value) {
    let mut out = vec![
        json!({"type": "table_joined", "table_id": "t1", "seat": hero,
               "players": hand.seats.iter().enumerate()
                   .map(|(i, s)| json!({"seat": i, "name": format!("p{i}"), "stack": s.stack})).collect::<Vec<_>>()}),
        json!({"type": "hand_start", "hand_id": "h1", "dealer_seat": hand.button, "seat": hero,
               "blinds": {"small_blind": hand.sb, "big_blind": hand.bb}, "table_seq": 1}),
    ];
    let board_at = |street: Street| {
        let dealt = match street {
            Street::Flop => 3,
            Street::Turn => 4,
            Street::River => 5,
            Street::Preflop => 0,
        };
        hand.board.iter().take(dealt).map(|c| c.to_string()).collect::<Vec<String>>()
    };
    // A `table_state` snapshot as it stands *at hand start* — blinds posted, nothing folded. Built
    // from `start_stack` and the blind seats rather than from the hand's final state, or every
    // action frame would land on top of bets the snapshot already carried.
    let (sb_seat, bb_seat) = hand.blind_seats();
    let posted = |i: usize| {
        if i == sb_seat {
            hand.sb
        } else if i == bb_seat {
            hand.bb
        } else {
            0
        }
    };
    let snapshot = |seq: i64, street: Street| {
        json!({"type": "table_state", "table_id": "t1", "hand_id": "h1", "street": street, "dealer_seat": hand.button,
               "small_blind": hand.sb, "big_blind": hand.bb, "pot": hand.sb + hand.bb,
               "board": board_at(street).iter().map(|c| c.to_string()).collect::<Vec<_>>(),
               "seats": hand.seats.iter().enumerate().map(|(i, s)| json!({
                   "seat": i, "name": format!("p{i}"), "stack": s.start_stack, "bet": posted(i),
                   "in_hand": true, "status": "active", "folded": Value::Null})).collect::<Vec<_>>(),
               "hero": {"seat": hero}, "table_seq": seq})
    };
    out.push(snapshot(3, Street::Preflop));
    let mut seq = 4i64;
    for (i, record) in hand.history.iter().enumerate() {
        // A new street arrives as its cards do.
        if i > 0 && record.street != hand.history[i - 1].street {
            let board = board_at(record.street);
            out.push(json!({"type": "community_cards", "street": record.street.name(),
                           "cards": board.iter().map(|c| c.as_str()).collect::<Vec<_>>(), "table_seq": seq}));
            seq += 1;
        }
        // The server tells a seat its cards as that seat starts to act; the tracker keeps one
        // `hole`, so the frame has to arrive before the turn it belongs to.
        out.push(json!({"type": "hole_cards", "seat": record.seat,
                        "cards": [hand.seats[record.seat].hole[0], hand.seats[record.seat].hole[1]], "table_seq": seq}));
        seq += 1;
        let action = match record.kind {
            ActionKind::Fold => "fold",
            ActionKind::Check => "check",
            ActionKind::Call => "call",
            ActionKind::Raise => "raise",
            ActionKind::AllIn => "all_in",
        };
        let put_in = record.to.saturating_sub(record.bet_before) + record.bet_before;
        let spent = if record.kind == ActionKind::Fold || record.kind == ActionKind::Check {
            0
        } else {
            record.to.saturating_sub(record.bet_before).max(0)
        };
        out.push(json!({"type": "player_action", "seat": record.seat, "action": action,
                        "amount": put_in.max(spent), "pot_before": record.pot_before,
                        "pot_after": record.pot_before + spent, "to_call_before": record.to_call_before,
                        "table_seq": seq}));
        seq += 1;
    }
    let actor = hand.actor().unwrap_or(hero);
    out.push(json!({"type": "hole_cards", "seat": actor,
                    "cards": [hand.seats[actor].hole[0], hand.seats[actor].hole[1]], "table_seq": seq}));
    seq += 1;
    let legal = hand.legal();
    // The turn-time snapshot: the live state the server pushes before a decision, and the one the
    // tracker must take as authoritative. `folded` is present only for a seat that folded.
    out.push(json!({"type": "table_state", "table_id": "t1", "hand_id": "h1", "street": hand.street.name(),
                    "dealer_seat": hand.button, "small_blind": hand.sb, "big_blind": hand.bb, "pot": hand.pot(),
                    "board": hand.board.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
                    "seats": hand.seats.iter().enumerate().map(|(i, s)| json!({
                        "seat": i, "name": format!("p{i}"), "stack": s.stack, "bet": s.bet, "in_hand": true,
                        "status": "active", "folded": if s.folded && i != hero { json!(true) } else { Value::Null }})).collect::<Vec<_>>(),
                    "hero": {"seat": hero}, "table_seq": seq}));
    seq += 1;
    let your_turn = json!({"type": "your_turn", "hand_id": "h1", "seat": actor, "pot": hand.pot(),
                           "turn_token": "tk1", "table_seq": seq,
                           "valid_actions": valid_actions(&legal)});
    (out, your_turn)
}

/// The server's `valid_actions` for an engine `Legal`, in the wire shape the tracker parses.
fn valid_actions(legal: &sv10_engine::engine::Legal) -> Value {
    // Folding is always offered; the engine's `Legal` has no field for it.
    let mut offers: Vec<Value> = vec![json!({"action": "fold"})];
    if legal.can_check {
        offers.push(json!({"action": "check"}));
    }
    if legal.call_amount > 0 {
        let call = legal.call_amount;
        offers.push(json!({"action": "call", "amount": call}));
    }
    if let (Some(lo), Some(hi)) = (legal.min_raise_to, legal.max_raise_to) {
        offers.push(json!({"action": "raise", "min": lo, "max": hi}));
    }
    json!(offers)
}

fn drive(script: &[Value]) -> TableTracker {
    let mut t = TableTracker::default();
    t.reset_table();
    for frame in script {
        match frame["type"].as_str().unwrap_or("") {
            "table_joined" => t.table_joined(frame),
            "hand_start" => t.hand_start(frame),
            "hole_cards" => t.hole_cards(frame),
            "player_action" => t.player_action(frame),
            "community_cards" => t.community_cards(frame),
            "table_state" => t.table_state(frame),
            _ => {}
        }
    }
    t
}

/// The two paths, compared field by field. Any difference here is a decision the fleet makes that no
/// simulation has ever seen.
/// Both situations in one line: street, pot, call, bet-to and each seat's bet/stack, for the
/// assertion messages below.
fn dump(live: &Situation, sim: &EngineSituation) -> String {
    let short = |v: Value| {
        let seats: Vec<String> = v["players"]
            .as_array()
            .map(|p| p.iter().map(|x| format!("{}:{}/{}", x["seat"], x["bet"], x["stack"])).collect())
            .unwrap_or_default();
        format!("street {} pot {} call {} to_bet {} | {}", v["street"], v["pot"], v["call_amount"], v["current_bet_to"], seats.join(" "))
    };
    format!("\n  live: {}\n  sim:  {}", short(serde_json::to_value(live).unwrap()), short(serde_json::to_value(sim).unwrap()))
}

fn assert_same(live: &Situation, sim: &EngineSituation, what: &str) {
    assert_eq!(live.street, sim.street, "{what}: street");
    assert_eq!(live.hole, sim.hole, "{what}: hole cards");
    assert_eq!(live.board, sim.board, "{what}: board");
    assert_eq!(live.button, sim.button, "{what}: button");
    assert_eq!(live.bb, sim.bb, "{what}: big blind");
    assert_eq!(live.pot, sim.pot, "{what}: pot");
    assert_eq!(live.call_amount, sim.call_amount, "{what}: call amount");
    assert_eq!(live.can_check, sim.can_check, "{what}: can check");
    assert_eq!(live.min_raise_to, sim.min_raise_to, "{what}: min raise");
    assert_eq!(live.max_raise_to, sim.max_raise_to, "{what}: max raise");
    assert_eq!(live.current_bet_to, sim.current_bet_to, "{what}: current bet to{}", dump(live, sim));

    let seat = |p: &PlayerInfo| (p.seat, p.stack, p.bet, p.folded);
    let mine: Vec<_> = live.players.iter().map(seat).collect();
    let theirs: Vec<_> = sim.players.iter().map(seat).collect();
    assert_eq!(mine, theirs, "{what}: the players and their stacks, bets and folds");
    assert_eq!(
        live.players.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        sim.players.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        "{what}: names"
    );

    let shape = |r: &ActionRecord| (r.seat, r.street, r.kind, r.to);
    assert_eq!(
        live.history.iter().map(shape).collect::<Vec<_>>(),
        sim.history.iter().map(shape).collect::<Vec<_>>(),
        "{what}: the action history{}",
        dump(live, sim)
    );
}

/// Advance the current betting round with checks/calls so the street turns.
fn close_round(hand: &mut Hand) {
    let street = hand.street;
    let mut guard = 0;
    while hand.street == street && !hand.is_finished() {
        let action = if hand.legal().can_check { Action::Check } else { Action::Call };
        hand.apply(action).expect("closing the round");
        guard += 1;
        assert!(guard < 24, "the round did not complete");
    }
}

/// Drive the tracker with the frames for `hand` and compare both situations. The hero is
/// whoever is to act: the final `hole_cards` frame carries their cards, matching `from_hand`.
fn compare(hand: &Hand, names: &[String], what: &str) {
    let hero = hand.actor().expect("someone to act");
    let (frames, your_turn) = script(hand, hero);
    let mut tracker = drive(&frames);
    let legal = LegalActions::parse(&your_turn["valid_actions"]);
    let live = tracker.situation(&your_turn, &legal).expect("the tracker must be able to decide");
    let sim = EngineSituation::from_hand(hand, hero, names);
    assert_same(&live, &sim, what);
}

/// The property the project has been missing: one decision, two builders, the same numbers.
#[test]
fn the_live_tracker_and_the_simulator_price_the_same_spot() {
    // Hero on the button facing a raise, then a cbet and a call, then a check-back decision.
    let scripts: [&[(usize, Action)]; 5] = [
        &[],
        &[(3, Action::RaiseTo(60))],
        &[(3, Action::RaiseTo(60)), (0, Action::RaiseTo(180))],
        &[(3, Action::RaiseTo(60)), (0, Action::RaiseTo(180)), (3, Action::Call)],
        &[(3, Action::RaiseTo(60)), (0, Action::RaiseTo(180)), (3, Action::Call), (4, Action::Call)],
    ];
    for (n, line) in scripts.iter().enumerate() {
        let (mut hand, names) = table(0);
        for &(_seat, action) in line.iter() {
            // The engine applies to whoever is to act; the tuple names that seat for the reader.
            hand.apply(action).unwrap_or_else(|e| panic!("script {n}: {action:?} rejected: {e}"));
        }
        let (frames, your_turn) = script(&hand, 0);
        let mut tracker = drive(&frames);
        let legal = LegalActions::parse(&your_turn["valid_actions"]);
        let live = tracker.situation(&your_turn, &legal).expect("the tracker must be able to decide");
        let sim = EngineSituation::from_hand(&hand, hand.actor().unwrap_or(0), &names);
        assert_same(&live, &sim, &format!("script {n} ({} actions, hero to act)", line.len()));
    }
}

/// The same comparison one street later, where the board, the cbet and a folded seat all differ —
/// and where the tracker's `folded` rule ("the snapshot is authoritative, else keep the same-hand
/// marker") is the thing under test.
#[test]
fn the_two_paths_agree_postflop_with_a_folded_opponent() {
    let (mut hand, names) = table(0);
    for action in [Action::RaiseTo(60), Action::RaiseTo(180), Action::Call] {
        hand.apply(action).unwrap();
    }
    // The street advances on its own once the betting round completes; then the button c-bets and
    // the aggressor folds, leaving the hero to act against one live opponent.
    let before_flop = hand.street;
    let mut guard = 0;
    while hand.street == before_flop && !hand.is_finished() {
        let _actor = hand.actor().unwrap();
        let action = if hand.legal().can_check { Action::Check } else { Action::Call };
        hand.apply(action).unwrap_or_else(|e| panic!("closing the preflop round: {e}"));
        guard += 1;
        assert!(guard < 12, "the preflop round did not complete");
    }
    let flopper = hand.actor().expect("someone acts on the flop");
    hand.apply(Action::RaiseTo(240)).unwrap();
    if let Some(other) = hand.actor()
        && other != flopper
    {
        hand.apply(Action::Fold).unwrap();
    }
    let (frames, your_turn) = script(&hand, 0);
    let mut tracker = drive(&frames);
    let legal = LegalActions::parse(&your_turn["valid_actions"]);
    let live = tracker.situation(&your_turn, &legal).expect("the tracker must be able to decide");
    let sim = EngineSituation::from_hand(&hand, hand.actor().unwrap_or(0), &names);
    assert_eq!(live.board.len(), 3, "the flop arrived as community cards");
    assert!(live.players.iter().any(|p| p.folded), "the folder is in the hand, marked folded");
    assert_same(&live, &sim, "flop with one folded opponent");
}

/// Turn and river are where the measured decision loss concentrates (0281): the same comparison
/// facing a turn bet, where the pot, the call amount and the bet-to all moved two streets on.
#[test]
fn the_two_paths_agree_on_the_turn_facing_a_bet() {
    let (mut hand, names) = table(0);
    close_round(&mut hand);
    assert_eq!(hand.street, Street::Flop);
    close_round(&mut hand);
    assert_eq!(hand.street, Street::Turn);
    let to = hand.legal().min_raise_to.unwrap_or(200);
    hand.apply(Action::RaiseTo(to)).expect("the turn bet");
    compare(&hand, &names, "turn facing a bet");
}

/// The river version: two streets of checked-down history plus a turn bet and its calls, then a
/// river bet — the longest history the differential covers, and the street whose all-in calls the
/// live fits keep recalibrating.
#[test]
fn the_two_paths_agree_on_the_river_facing_a_bet() {
    let (mut hand, names) = table(0);
    close_round(&mut hand);
    close_round(&mut hand);
    let to = hand.legal().min_raise_to.unwrap_or(200);
    hand.apply(Action::RaiseTo(to)).expect("the turn bet");
    close_round(&mut hand);
    assert_eq!(hand.street, Street::River);
    let to = hand.legal().min_raise_to.unwrap_or(200);
    hand.apply(Action::RaiseTo(to)).expect("the river bet");
    compare(&hand, &names, "river facing a bet");
}

/// Three players see the flop with no folds: every seat stays in both histories, and the pot
/// geometry has three live bets instead of two.
#[test]
fn the_two_paths_agree_multiway() {
    let (mut hand, names) = table(0);
    close_round(&mut hand);
    assert_eq!(hand.street, Street::Flop);
    let live_seats = hand.seats.iter().filter(|s| !s.folded).count();
    assert!(live_seats >= 3, "multiway: {live_seats} seats still in");
    compare(&hand, &names, "multiway flop, no folds");
}

/// A short stack is all-in while the others keep betting: the pot both paths price has a main
/// pot and a side pot, and the all-in seat can neither act nor fold (0080's geometry).
#[test]
fn the_two_paths_agree_with_an_all_in_side_pot() {
    let (mut hand, names) = table(0);
    hand.seats[2].stack = 300;
    hand.seats[2].start_stack = 300;
    close_round(&mut hand);
    assert_eq!(hand.street, Street::Flop);
    close_round(&mut hand);
    assert_eq!(hand.street, Street::Turn);
    // The short stack commits everything; everyone else can only call it and play on.
    let mut guard = 0;
    while hand.street == Street::Turn && !hand.is_finished() {
        let actor = hand.actor().expect("a turn actor");
        let action = if actor == 2 { Action::AllIn } else { Action::Call };
        if hand.legal().can_check && actor != 2 {
            hand.apply(Action::Check).expect("the side pot checks around");
        } else {
            hand.apply(action).unwrap_or_else(|_| hand.apply(Action::Call).expect("a call is always legal"));
        }
        guard += 1;
        assert!(guard < 24, "the turn did not complete");
    }
    let short = &hand.seats[2];
    assert!(short.stack == 0 || hand.is_finished(), "the short stack committed");
    if !hand.is_finished() {
        compare(&hand, &names, "turn with an all-in side pot");
    }
}

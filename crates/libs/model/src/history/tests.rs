use super::*;

fn raw(json: &str) -> RawHand {
    serde_json::from_str(json).unwrap()
}

const HAND: &str = r#"{"hand_id":"h1","table_id":"t","hand_number":5,"seat":1,"stack_start":2000,"profit":-10,
    "all_actions":[
      {"seat":0,"action":"fold","amount":0,"street":"preflop","sequence_in_hand":1},
      {"seat":1,"action":"fold","amount":0,"street":"preflop","sequence_in_hand":2},
      {"seat":2,"action":"raise","amount":45,"street":"preflop","sequence_in_hand":3},
      {"seat":3,"action":"fold","amount":0,"street":"preflop","sequence_in_hand":4},
      {"seat":4,"action":"call","amount":0,"street":"preflop","sequence_in_hand":5},
      {"seat":5,"action":"fold","amount":0,"street":"flop","sequence_in_hand":6},
      {"seat":4,"action":"check","amount":0,"street":"flop","sequence_in_hand":7},
      {"seat":2,"action":"raise","amount":60,"street":"flop","sequence_in_hand":8},
      {"seat":4,"action":"raise","amount":200,"street":"flop","sequence_in_hand":9},
      {"seat":2,"action":"call","amount":0,"street":"turn","sequence_in_hand":10},
      {"seat":4,"action":"check","amount":0,"street":"turn","sequence_in_hand":11},
      {"seat":2,"action":"check","amount":0,"street":"river","sequence_in_hand":12},
      {"seat":4,"action":"check","amount":0,"street":"river","sequence_in_hand":13},
      {"seat":2,"action":"check","amount":0,"street":"showdown","sequence_in_hand":14}],
    "board":["Qc","Jc","5c","3c","8s"],"pot_total":510,"num_players":6,"small_blind":10,"big_blind":20,
    "dealer_seat":3,"winners":[{"seat":2,"name":"gouda"}],"hole_cards":["6d","5h"],
    "shown_cards":{"4":["Td","As"],"2":["Qh","Ts"]}}"#;

#[test]
fn replays_bets_streets_and_pot() {
    let r = raw(HAND);
    let s = to_summary(&r, &HashMap::from([(2, "gouda".to_string())]), &|seat| format!("\u{0}anon{seat}")).unwrap();
    assert_eq!(s.players.len(), 6);
    assert_eq!(s.history.len(), 14);
    // SB (seat 4) calls 45 facing a raise: to-call 35.
    let call = &s.history[4];
    assert_eq!((call.street, call.kind, call.to, call.to_call_before, call.bet_before), (Street::Preflop, ActionKind::Call, 45, 35, 10));
    // BB folds preflop even though its label says flop.
    assert_eq!((s.history[5].street, s.history[5].kind), (Street::Preflop, ActionKind::Fold));
    // Flop: pot 110, bet 60, check-raise to 200 is a full raise, call 140.
    let bet = &s.history[7];
    assert_eq!((bet.street, bet.to, bet.pot_before, bet.to_call_before), (Street::Flop, 60, 110, 0));
    let raise = &s.history[8];
    assert!(raise.full_raise && raise.to == 200 && raise.to_call_before == 60);
    let call = &s.history[9];
    assert_eq!((call.street, call.to, call.to_call_before), (Street::Flop, 200, 140));
    let last = s.history.last().unwrap();
    assert_eq!(last.street, Street::River);
    // Pot reconstruction matches the export: 30 blinds + 25 + 35 + 60 + 200 + 140 + ... = 510.
    assert_eq!(last.pot_before, 510);
    assert_eq!(s.shown.len(), 2);
    assert_eq!(s.players.iter().find(|p| p.0 == 2).unwrap().1, "gouda");
}

/// Replays real exported pages (HISTORY_SAMPLES=dir of page JSON files) and reports how
/// often the reconstructed pot matches the server's pot total.
#[test]
#[ignore]
fn replays_real_exports() {
    let Ok(dir) = std::env::var("HISTORY_SAMPLES") else { return };
    let (mut n, mut ok, mut failed, mut named, mut unparseable) = (0, 0, 0, 0, 0);
    // Hands holding an all-in, and how many of those the pot comparison accepts: the seat's
    // unknown stack is what the replay prices the shove from (1011). Split by whether the
    // shover is our own seat, whose stack the export does give us.
    let (mut allin_n, mut allin_ok) = (0, 0);
    let (mut hero_allin_n, mut hero_allin_ok) = (0, 0);
    // The pot comparison above cancels the very error the shove pricing causes: it subtracts
    // the uncalled top bet, which a shove inflated to our whole stack inflates in step. What
    // moves is the deviation's *magnitude*, and how often an enemy shove is read as aggressive
    // (`aggressive()`) -- the reads that feed pfr, 3-bet and bet_first (1011).
    let (mut dev_sum, mut enemy_raises, mut enemy_shoves) = (0i64, 0, 0);
    for f in std::fs::read_dir(dir).unwrap().flatten() {
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(f.path()).unwrap()).unwrap();
        // One export the reader rejects (a null `amount`, which the server emits on a few folds)
        // must not hide the rest of its page: count it and carry on.
        let hands: Vec<RawHand> = v["hands"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|h| match serde_json::from_value::<RawHand>(h.clone()) {
                Ok(r) => Some(r),
                Err(_) => {
                    unparseable += 1;
                    None
                }
            })
            .collect();
        let refs: Vec<&RawHand> = hands.iter().collect();
        named += attribute_names(&refs, 40).iter().map(|m| m.len()).sum::<usize>();
        for h in &hands {
            n += 1;
            let shoves: Vec<usize> = h.all_actions.iter().filter(|a| a.action == "all_in").map(|a| a.seat).collect();
            if !shoves.is_empty() {
                allin_n += 1;
                if shoves.contains(&h.seat) {
                    hero_allin_n += 1;
                }
            }
            match to_summary(h, &HashMap::new(), &|s| format!("x{s}")) {
                None => failed += 1,
                Some(s) => {
                    // Final pot less the uncalled part of the last street's top bet.
                    let last = s.history.last().unwrap();
                    let mut pot = last.pot_before + (last.to - last.bet_before).max(0);
                    let mut bets: HashMap<usize, i64> = HashMap::new();
                    for r in s.history.iter().filter(|r| r.street == last.street) {
                        if r.to > 0 {
                            bets.insert(r.seat, r.to);
                        }
                    }
                    let mut v: Vec<i64> = bets.values().copied().collect();
                    v.sort_unstable_by(|a, b| b.cmp(a));
                    if last.street == Street::Preflop {
                        // Blinds count as street bets preflop.
                        v.extend([h.small_blind, h.big_blind]);
                        v.sort_unstable_by(|a, b| b.cmp(a));
                    }
                    if v.len() >= 2 {
                        pot -= v[0] - v[1];
                    } else if v.len() == 1 {
                        pot -= v[0];
                    }
                    dev_sum += (pot - h.pot_total).abs();
                    for r in s.history.iter().filter(|r| r.kind == ActionKind::AllIn && r.seat != h.seat) {
                        enemy_shoves += 1;
                        enemy_raises += crate::model::aggressive(r) as i64;
                    }
                    if (pot - h.pot_total).abs() <= h.big_blind {
                        ok += 1;
                        if !shoves.is_empty() {
                            allin_ok += 1;
                            if shoves.contains(&h.seat) {
                                hero_allin_ok += 1;
                            }
                        }
                    } else if std::env::var("HISTORY_VERBOSE").is_ok() && n % 3 == 0 {
                        eprintln!(
                            "pot {pot} vs {} dealer {} :: {:?}",
                            h.pot_total,
                            h.dealer_seat,
                            h.all_actions.iter().map(|a| format!("{}:{}:{}:{}", a.seat, a.action, a.amount, a.street)).collect::<Vec<_>>()
                        );
                    }
                }
            }
        }
    }
    eprintln!(
        "hands {n} unparseable {unparseable} failed {failed} pot-match {ok} ({:.1}%) named seats {named}\n  \
         enemy all-in hands {allin_n} pot-match {allin_ok} ({:.1}%) :: our all-in hands {hero_allin_n} pot-match {hero_allin_ok} ({:.1}%)\n  \
         mean |pot - pot_total| {:.1} :: enemy shoves read aggressive {enemy_raises}/{enemy_shoves} ({:.1}%)",
        100.0 * ok as f64 / n.max(1) as f64,
        100.0 * allin_ok as f64 / allin_n.max(1) as f64,
        100.0 * hero_allin_ok as f64 / hero_allin_n.max(1) as f64,
        dev_sum as f64 / n.max(1) as f64,
        100.0 * enemy_raises as f64 / enemy_shoves.max(1) as f64
    );
}

/// The export carries our stack and nobody else's, so a shove by another seat has to be priced
/// from what the pot says was wagered rather than from our own stack (1011).
#[test]
fn prices_an_unknown_seats_all_in_from_the_pot_not_our_stack() {
    // Heads-up on our button: we raise to 200, the big blind shoves and we fold. The shover
    // really contests 200 and 150 comes back, so the export reports a pot of 400 -- not the
    // 2000 the replay assumes from our own starting stack. Called, the shover's 350 stands.
    let hand = |last: &str, pot_total: i64| {
        raw(&format!(
            r#"{{"hand_id":"h2","table_id":"t","hand_number":6,"seat":0,"stack_start":2000,
                "profit":-200,"all_actions":[
                  {{"seat":0,"action":"raise","amount":200,"street":"preflop","sequence_in_hand":1}},
                  {{"seat":1,"action":"all_in","amount":0,"street":"preflop","sequence_in_hand":2}},
                  {{"seat":0,"action":"{last}","amount":0,"street":"preflop","sequence_in_hand":3}}],
                "board":[],"pot_total":{pot_total},"num_players":2,"small_blind":10,"big_blind":20,
                "dealer_seat":0,"winners":[{{"seat":1,"name":"v"}}],"hole_cards":[],"shown_cards":{{}}}}"#
        ))
    };
    let shove = |r: &RawHand| {
        let s = to_summary(r, &HashMap::new(), &|seat| format!("x{seat}")).unwrap();
        let rec = s.history.iter().find(|h| h.seat == 1).unwrap().clone();
        (rec, s)
    };
    // Folded to: capped at the 400 the pot holds, still an aggressive action.
    let (rec, _) = shove(&hand("fold", 400));
    assert_eq!((rec.kind, rec.to), (ActionKind::AllIn, 400));
    assert!(crate::model::aggressive(&rec));
    // Called: the pot shows more was wagered than the call, so the shove stands above it.
    let (rec, _) = shove(&hand("call", 700));
    assert_eq!((rec.kind, rec.to), (ActionKind::AllIn, 700));
    assert!(crate::model::aggressive(&rec));
}

/// Some exports report a pot smaller than the chips already in it (a hand whose `pot_total` is 20
/// when the blinds alone are 30). The pot cap must not let that price an observed shove *below* what
/// the rules allow a shove to be: facing no bet, a shove is a bet, and reading it as a check deletes
/// an aggression from the tallies (1011).
#[test]
fn an_under_reported_pot_does_not_price_a_shove_below_a_minimum_bet() {
    // Heads-up: we raise and are called, we check the flop, the villain shoves over it. The export
    // reports the pot as our own 200, leaving no room to price the shove into -- but the seat did
    // shove, so it is still a bet of at least the minimum.
    let r = raw(r#"{"hand_id":"h3","table_id":"t","hand_number":7,"seat":0,"stack_start":2000,"profit":-200,
            "all_actions":[
              {"seat":0,"action":"raise","amount":200,"street":"preflop","sequence_in_hand":1},
              {"seat":1,"action":"call","amount":0,"street":"flop","sequence_in_hand":2},
              {"seat":0,"action":"check","amount":0,"street":"flop","sequence_in_hand":3},
              {"seat":1,"action":"all_in","amount":0,"street":"turn","sequence_in_hand":4},
              {"seat":0,"action":"fold","amount":0,"street":"turn","sequence_in_hand":5}],
            "board":["4s","Ac","2h","As","9c"],"pot_total":200,"num_players":2,"small_blind":10,
            "big_blind":20,"dealer_seat":0,"winners":[{"seat":1,"name":"v"}],"hole_cards":[],
            "shown_cards":{}}"#);
    let s = to_summary(&r, &HashMap::new(), &|seat| format!("x{seat}")).unwrap();
    let rec = s.history.iter().find(|h| h.kind == ActionKind::AllIn).unwrap();
    assert_eq!((rec.seat, rec.kind, rec.to), (1, ActionKind::AllIn, 20));
    assert!(crate::model::aggressive(rec));
}

#[test]
fn names_follow_winner_sightings() {
    let mk = |n: i64, winner: Option<(usize, &str)>| {
        let mut r = raw(HAND);
        r.hand_number = n;
        r.winners = winner.map(|(s, name)| vec![RawWinner { seat: s, name: name.into() }]).unwrap_or_default();
        r
    };
    let hands = [mk(1, Some((4, "a"))), mk(2, None), mk(3, Some((4, "a"))), mk(4, None), mk(90, Some((4, "b")))];
    let refs: Vec<&RawHand> = hands.iter().collect();
    let names = attribute_names(&refs, 30);
    assert_eq!(names[1].get(&4).map(String::as_str), Some("a"));
    assert_eq!(names[3].get(&4).map(String::as_str), Some("a"));
    assert_eq!(names[4].get(&4).map(String::as_str), Some("b"));
}

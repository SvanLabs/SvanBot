//! Tests for the rival report: confrontation classification, c-bet answers and the "always win"
//! bound.
//!
//! Split out of `review_rival.rs` (0320: the 500-line rule).

use super::*;

/// The live all-in hand of `sv10_core::flow`'s tests: SurSvan (seat 4, big blind) shoves the flop,
/// POKER_STUDY_AI (seat 5, the preflop raiser) calls 2,000, Bertabot folds.
const ALL_IN: &str = r#"{"players":[[0,"MissCard"],[1,"jonnaBee"],[2,"Bertabot"],[3,"RObert"],[4,"SurSvan"],[5,"POKER_STUDY_AI"]],"button":2,"bb":20,"history":[{"seat":5,"street":"Preflop","kind":"Raise","to":50,"pot_before":30,"to_call_before":20,"bet_before":0,"full_raise":true},{"seat":0,"street":"Preflop","kind":"Fold","to":0,"pot_before":80,"to_call_before":50,"bet_before":0,"full_raise":false},{"seat":1,"street":"Preflop","kind":"Fold","to":0,"pot_before":80,"to_call_before":50,"bet_before":0,"full_raise":false},{"seat":2,"street":"Preflop","kind":"Call","to":50,"pot_before":80,"to_call_before":50,"bet_before":0,"full_raise":false},{"seat":3,"street":"Preflop","kind":"Fold","to":0,"pot_before":130,"to_call_before":40,"bet_before":10,"full_raise":false},{"seat":4,"street":"Preflop","kind":"Call","to":50,"pot_before":130,"to_call_before":30,"bet_before":20,"full_raise":false},{"seat":4,"street":"Flop","kind":"AllIn","to":0,"pot_before":160,"to_call_before":0,"bet_before":0,"full_raise":false},{"seat":5,"street":"Flop","kind":"Raise","to":4629,"pot_before":2110,"to_call_before":1950,"bet_before":0,"full_raise":true},{"seat":2,"street":"Flop","kind":"Fold","to":0,"pot_before":6739,"to_call_before":4629,"bet_before":0,"full_raise":false}],"board":["5d","As","3s","6s","Th"],"shown":[[4,["Ts","Ac"]],[5,["Ah","5h"]]]}"#;

/// #723: `--json` is one parseable object carrying the same per-rival figures the tables print, and
/// the text path — what an operator/script saw before the flag existed — is unchanged.
#[test]
fn the_json_report_parses_and_the_text_path_is_unchanged() {
    let dir = std::env::temp_dir().join(format!("sv10-rival-json-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(&dir.join("t.db")).unwrap();
    let names = vec!["Ghost".to_string()];
    let text = rival(&store, &names, false).unwrap();
    assert!(text.contains("Ghost: never dealt in with our bots"), "{text}");
    let json: serde_json::Value = serde_json::from_str(&rival(&store, &names, true).unwrap()).unwrap();
    assert!(json["since"].is_null(), "no window: {json}");
    assert_eq!(json["rivals"][0]["name"], "Ghost");
    assert_eq!(json["rivals"][0]["hands"], 0);
    assert_eq!(json["rivals"][0]["note"], "never dealt in with our bots");
}

/// 0317: a confrontation is classified by how it ended, the pot type, position and the preflop
/// raiser, and the rival's share of an all-in's luck is exactly 2·min(contribution)/pot of ours.
#[test]
fn a_heads_up_all_in_is_classified_and_its_luck_split_exactly() {
    let h: HandSummary = serde_json::from_str(ALL_IN).unwrap();
    // We lost 2,000 to seat 5; our all-in EV was 600 chips better than the result.
    let c = classify(&h, 4, 5, -2_000.0, 600.0, 4_060).unwrap();
    assert_eq!((c.ending, c.street, c.pot_type, c.opener), ("showdown", Street::Flop, "single-raised", "they"));
    assert!(!c.in_position, "seat 5 acts after the big blind postflop");
    assert_eq!(c.flow_bb, -100.0);
    // 2 · min(2,000, 2,000) / 4,060 of the 600-chip adjustment, in big blinds.
    assert!((c.luck_bb - 600.0 * 4_000.0 / 4_060.0 / 20.0).abs() < 1e-9, "{}", c.luck_bb);
    // Against the folder: they folded on the flop, and no luck is theirs (they were not all in).
    let f = classify(&h, 4, 2, 50.0, 600.0, 4_060).unwrap();
    assert_eq!((f.ending, f.street, f.luck_bb), ("they fold", Street::Flop, 0.0));
    assert_eq!(classify(&h, 4, 0, 0.0, 600.0, 4_060), None, "no chips moved: not a confrontation");
}

/// 0317: the c-bet answer is found only when the rival raised last preflop, bet the flop first,
/// and we acted after the bet — here POKER_STUDY_AI raised, but we led the flop, so it is no c-bet.
#[test]
fn a_flop_c_bet_answer_needs_the_raiser_to_bet_first() {
    let h: HandSummary = serde_json::from_str(ALL_IN).unwrap();
    assert_eq!(cbet_answer(&h, 4), None, "we shoved first: not a c-bet");
    let mut bet = h.clone();
    // Seat 5 bets the flop first; we (seat 4) fold, then Bertabot calls.
    bet.history.truncate(6);
    let r = |seat, kind, to, pot_before, to_call_before| sv10_core::engine::ActionRecord {
        seat,
        street: Street::Flop,
        kind,
        to,
        pot_before,
        to_call_before,
        bet_before: 0,
        full_raise: kind == ActionKind::Raise,
        think_ms: None,
        ..h.history[0].clone()
    };
    bet.history.push(r(4, ActionKind::Check, 0, 160, 0));
    bet.history.push(r(5, ActionKind::Raise, 100, 160, 0));
    bet.history.push(r(2, ActionKind::Call, 100, 260, 100));
    bet.history.push(r(4, ActionKind::Fold, 0, 360, 100));
    assert_eq!(cbet_answer(&bet, 4), Some((5, "fold")));
    assert_eq!(cbet_answer(&bet, 5), None, "the raiser does not answer its own c-bet");
}

/// The honest reading of "always win": a lower bound above zero, or how far away it is.
#[test]
fn hands_to_prove_a_win_follow_the_rate_and_the_spread() {
    let losing = vec![-1.0, 1.0, -2.0, 0.5];
    assert!(hands_to_prove(&losing).starts_with("not ahead"));
    // A mean of 0.1 bb with a 1 bb spread needs about (1.96 · 1 / 0.1)² ≈ 384 hands.
    let small: Vec<f64> = (0..100).map(|i| if i % 2 == 0 { 1.1 } else { -0.9 }).collect();
    let text = hands_to_prove(&small);
    assert!(text.starts_with("about 38"), "{text}");
    let clear: Vec<f64> = (0..400).map(|i| if i % 2 == 0 { 1.5 } else { -0.5 }).collect();
    assert!(hands_to_prove(&clear).starts_with("proven"));
}

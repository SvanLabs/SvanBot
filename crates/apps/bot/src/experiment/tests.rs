//! Tests for the experiment mode: readings, the pair, and the hand-boundary switch.
//!
//! Split out of `experiment.rs` (0320: the 500-line rule).

use super::*;
use crate::search_ledger::LedgerEntry;

fn active_live(now: f64) -> Live {
    let mode = ModeState {
        status: Status::Active,
        season: Some("s13".into()),
        protected: vec!["A".into(), "B".into(), "C".into()],
        pair: vec!["D".into(), "E".into()],
        last_reading_at: Some(now),
        ..Default::default()
    };
    let challenger = Params { fold_scale: 0.9, ..Params::default() };
    let t = Target::new(("v1", 3), "fold_scale", 1.0, 0.9, challenger, LedgerEntry::default(), target::Source::Ledger);
    Live { mode, target: Some(Arc::new(t)), ..Default::default() }
}

/// 0327: a stalled poller is the one way the stored mode could stay `Active` on a reading
/// hours old — a poll that *fails* ends the mode by itself — and `expire` ran nowhere in
/// production, so nothing recorded it: `review experiment` kept printing `Active` and no line
/// said the experiment had ended.
#[test]
fn a_poll_after_a_stall_ends_the_active_mode_and_says_so() {
    let active = || ModeState {
        status: Status::Active,
        season: Some("s13".into()),
        protected: vec!["A".into(), "B".into(), "C".into()],
        pair: vec!["D".into(), "E".into()],
        last_reading_at: Some(1_000.0),
        activated_at: Some(1_000.0),
        ..Default::default()
    };
    let mut mode = active();
    let now = 1_000.0 + mode::STALE_AFTER_SECS + 1.0;
    let said = advance(&mut mode, Err("timeout".into()), now);
    assert!(said[0].contains("stale"), "the stale reading is what ended it: {said:?}");
    assert_eq!(mode.status, Status::Champion);
    assert!(mode.pair.is_empty(), "a stale reading ends the pair's experiment");

    // A reading that arrives in time is applied with no staleness step in between.
    let mut mode = active();
    let fresh = Reading {
        season: "s13".into(),
        at: 1_300.0,
        ranks: [("A", 1), ("B", 2), ("C", 3), ("D", 4), ("E", 5)].into_iter().map(|(n, r)| (n.to_string(), r)).collect(),
    };
    assert!(advance(&mut mode, Ok(fresh), 1_300.0).is_empty());
    assert_eq!(mode.status, Status::Active);
    assert_eq!(mode.pair, vec!["D".to_string(), "E".to_string()]);
}

#[test]
fn only_the_pair_plays_arms_and_only_while_the_mode_is_fresh() {
    let mut live = active_live(1_000.0);
    assert_eq!(decide_policy(&mut live, "A", "h1", 1_000.0, "v1", None).arm, None, "the protected trio plays the champion");
    let d = decide_policy(&mut live, "D", "h1", 1_000.0, "v1", None);
    let e = decide_policy(&mut live, "E", "h2", 1_000.0, "v1", None);
    assert_eq!((d.arm, e.arm), (Some(Arm::Treatment), Some(Arm::Control)));
    assert_eq!(d.record.as_ref().unwrap()["provenance"], "parameter_challenger");
    assert_eq!(e.tag().unwrap().arm, "control");
    let stale = 1_000.0 + mode::STALE_AFTER_SECS + 1.0;
    assert_eq!(decide_policy(&mut live, "D", "h3", stale, "v1", None).arm, None, "a stale reading fails closed");
    live.store_error = Some("disk".into());
    assert_eq!(decide_policy(&mut live, "D", "h4", 1_000.0, "v1", None).arm, None);
    live.store_error = None;
    live.target = None;
    assert_eq!(decide_policy(&mut live, "D", "h5", 1_000.0, "v1", None).arm, None, "no target, no experiment");
}

#[test]
fn treatment_changes_exactly_the_knob_and_keeps_the_live_fits() {
    let mut live = active_live(0.0);
    let p = decide_policy(&mut live, "D", "h1", 0.0, "v1", None);
    let champion = Params { samples: 9_999, fold_logit_shift: [0.1, 0.2, 0.3], ..Params::default() };
    let treated = p.params(&champion);
    assert_eq!(treated.fold_scale, 0.9);
    assert_eq!((treated.samples, treated.fold_logit_shift), (9_999, [0.1, 0.2, 0.3]));
    let control = decide_policy(&mut live, "E", "h1", 0.0, "v1", None);
    assert_eq!(control.params(&champion).fold_scale, champion.fold_scale);
    assert!(HandPolicy::ordinary("h").tag().is_none());
}

#[test]
fn the_audit_lists_every_hand_of_a_target_with_its_arm() {
    let dir = std::env::temp_dir().join(format!("sv10-experiment-review-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = sv10_store::store::Store::open(&dir.join("t.db")).unwrap();
    let mut live = active_live(0.0);
    for (bot, hand) in [("D", "h1"), ("E", "h2")] {
        let p = decide_policy(&mut live, bot, hand, 0.0, "v1", None);
        let row = sv10_store::store::HandRow {
            bot: bot.into(),
            hand_id: hand.into(),
            ended_at: "2026-09-27T00:00:00Z".into(),
            net: Some(40),
            summary: r#"{"bb":20}"#.into(),
            ..Default::default()
        };
        store.insert_hand_tagged(&row, p.tag().as_ref()).unwrap();
    }
    let id = live.target.as_ref().unwrap().id.clone();
    let mut out = Vec::new();
    review(&store, Some(&id), &mut out, false).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("1 treatment / 1 control hands"), "{text}");
    assert!(text.contains("D            treatment h1") && text.contains("E            control   h2"), "{text}");
    let mut out = Vec::new();
    review(&store, None, &mut out, false).unwrap();
    assert!(String::from_utf8(out).unwrap().contains(&id));
    // #723: `--json` is one parseable object carrying every hand the text rows name.
    let mut out = Vec::new();
    review(&store, Some(&id), &mut out, true).unwrap();
    let json: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["target"], id.as_str());
    let rows = json["hands"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{json}");
    assert_eq!(rows[0]["bot"], "D");
    assert_eq!(rows[0]["arm"], "treatment");
    assert!(rows[0]["hand_id"].is_string() && rows[0]["net"].is_number(), "{json}");
}

#[test]
fn a_policy_is_fixed_for_the_whole_hand_and_the_fallback_waits_for_the_next() {
    let shared = Shared::for_test("experiment-latch", &["A", "B", "C", "D", "E"]);
    *shared.experiment.write() = active_live(now_secs());
    *shared.champion_version.write() = "v1".into();
    let first = latch(&shared, 3, "h1");
    assert_eq!(first.arm, Some(Arm::Treatment));
    // The top four is lost mid-hand: this hand keeps its policy, the next one is champion.
    shared.experiment.write().mode.observe(Err("HTTP 503".into()), now_secs());
    assert_eq!(latch(&shared, 3, "h1").arm, Some(Arm::Treatment), "never switch during a hand");
    assert_eq!(latch(&shared, 3, "h2").arm, None);
    assert_eq!(latch(&shared, 0, "h2").arm, None, "the protected trio is never touched");
}

#[test]
fn the_pair_plays_the_champion_while_any_bot_has_its_own_lineage() {
    let shared = Shared::for_test("experiment-lineages", &["A", "B", "C", "D", "E"]);
    *shared.experiment.write() = active_live(now_secs());
    assert_eq!(latch(&shared, 3, "h1").arm, Some(Arm::Treatment));
    shared.update(0, |b| b.slot_params = Some(Default::default()));
    assert_eq!(latch(&shared, 3, "h2").arm, None, "no single champion to test against");
}

#[test]
fn a_bot_swaps_arms_after_each_block() {
    let mut live = active_live(0.0);
    let arms: Vec<Arm> =
        (0..evidence::BLOCK_HANDS * 2).map(|i| decide_policy(&mut live, "D", &format!("h{i}"), 0.0, "v1", None).arm.unwrap()).collect();
    assert!(arms[..evidence::BLOCK_HANDS as usize].iter().all(|a| *a == Arm::Treatment));
    assert!(arms[evidence::BLOCK_HANDS as usize..].iter().all(|a| *a == Arm::Control));
}

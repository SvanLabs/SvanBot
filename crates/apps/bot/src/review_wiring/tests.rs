//! Tests for `review wiring` (split out of `review_wiring.rs`: the 500-line rule).

use super::*;

/// 0316: the dashboard's wiring panel reads this row, so its shape is a contract — and a sample
/// too small to mean anything never replaces a stored report.
#[test]
fn a_stored_report_round_trips_and_a_tiny_sample_is_not_stored() {
    let dir = std::env::temp_dir().join(format!("sv10-wiring-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(&dir.join("svanbot10.db")).unwrap();
    // No replays recorded: nothing measurable, and nothing stored for the panel to show.
    let empty = refresh(&store, 50).unwrap();
    assert_eq!(empty.sample, 0);
    assert_eq!(store.get_kv(WIRING_KEY).unwrap(), None, "an empty sample must not overwrite a stored table");
    // Nothing stored is due; a fresh row is not; an old one is again.
    assert!(due(&store, 1_000.0, 86_400.0), "no report yet");
    store
        .put_kv(
            WIRING_KEY,
            r#"{"at": 900.0, "sample": 200, "unstable": 0, "exact": 1, "exact_with_current": 1,
        "carrying_corrections": 1, "v3": 1, "v3_exact": 1, "chosen_not_best": 0, "rows": []}"#,
        )
        .unwrap();
    assert!(!due(&store, 1_000.0, 86_400.0), "measured 100 s ago");
    assert!(due(&store, 90_000.0, 86_400.0), "measured a day ago");
    store.put_kv(WIRING_KEY, "not json").unwrap();
    assert!(due(&store, 1_000.0, 86_400.0), "an unreadable row must be replaced, not trusted");
    // The row the panel parses: every figure survives JSON, and the components keep their order.
    let report = WiringReport {
        at: 1_790_000_000.0,
        sample: 200,
        unstable: 0,
        exact: 131,
        exact_with_current: 132,
        carrying_corrections: 30,
        v3: 30,
        v3_exact: 30,
        chosen_not_best: 1,
        rows: VARIANTS
            .iter()
            .map(|v| Row { component: v.name.to_string(), changed: 3, share_pct: 1.5, cost_bb: 0.25, max_bb: 40.0, installed: Some(200) })
            .collect(),
        calibration: vec![CalibrationFlips {
            street: "preflop".into(),
            decisions: 3_227,
            flipped: 1_794,
            share_pct: 55.6,
            main: "fold -> call".into(),
            main_count: 1_030,
        }],
        streets: vec![("preflop".into(), 6), ("flop".into(), 80), ("turn".into(), 64), ("river".into(), 50)],
    };
    store.put_kv(WIRING_KEY, &serde_json::to_string(&report).unwrap()).unwrap();
    let back: WiringReport = serde_json::from_str(&store.get_kv(WIRING_KEY).unwrap().unwrap()).unwrap();
    assert_eq!(back, report);
    assert_eq!(back.rows.len(), VARIANTS.len());
    assert_eq!(back.rows[0].component, VARIANTS[0].name, "the table prints in measurement order");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 0332: self-calibration decided over half of all preflop choices while the big-decision table
/// showed it moving 0.7%. A decision counts as flipped when its best action differs with and without
/// the bias recorded on each candidate; sizes of one action are one family; one option is no choice.
#[test]
fn a_decision_counts_as_flipped_when_the_bias_decides_its_best_action() {
    let d = |cands: &str| format!(r#"{{"candidates": [{cands}]}}"#);
    let fold = r#"{"action": "fold", "ev": 0.0, "bias": 0.0}"#;
    let rows = vec![
        // Call −20 without its +50 bias, +30 with it: the bias turned a fold into a call.
        ("preflop".to_string(), d(&format!(r#"{fold}, {{"action": "call", "ev": 30.0, "bias": 50.0}}"#))),
        // Call is best either way.
        ("preflop".to_string(), d(&format!(r#"{fold}, {{"action": "call", "ev": 80.0, "bias": 50.0}}"#))),
        // Two raise sizes trade places under the bias: the same family, no flip.
        (
            "flop".to_string(),
            d(
                r#"{"action": "raise", "ev": 10.0, "bias": 5.0}, {"action": "raise", "ev": 9.0, "bias": 0.0}, {"action": "check", "ev": 1.0}"#,
            ),
        ),
        // One option is no choice at all, and an unreadable detail is skipped.
        ("river".to_string(), d(fold)),
        ("river".to_string(), "not json".to_string()),
    ];
    let f = calibration_flips(&rows);
    assert_eq!(f.len(), 2, "streets with a choice only: {f:?}");
    assert_eq!((f[0].street.as_str(), f[0].decisions, f[0].flipped, f[0].main.as_str()), ("preflop", 2, 1, "fold -> call"));
    assert_eq!(f[0].share_pct, 50.0);
    assert_eq!((f[1].street.as_str(), f[1].decisions, f[1].flipped), ("flop", 1, 0), "streets print in play order");
}

/// A preflop spot facing a raise, recorded with `params` (the same shape as review_drift's tests).
fn preflop_spot(params: Params) -> ReplayRecord {
    use sv10_core::engine::{Action, Hand};
    use sv10_core::model::ModelStore;
    use sv10_core::policy::decide_with;
    use sv10_core::situation::Situation;
    use sv10_rng::SeedableRng;
    use sv10_rng::rngs::SmallRng;
    let mut rng = SmallRng::seed_from_u64(3);
    let mut hand = Hand::new(&[40_000, 40_000, 40_000], 0, 10, 20, &mut rng);
    hand.apply(Action::RaiseTo(60)).unwrap();
    let actor = hand.actor().unwrap();
    let names: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
    let sit = Situation::from_hand(&hand, actor, &names);
    let models = ModelStore::default();
    let d = decide_with(&sit, &models, &params, None, &mut SmallRng::seed_from_u64(99));
    crate::replay::record(&sit, 99, &params, &models, None, &d)
}

/// #315: a component with nothing to switch off on a spot is counted as not there, so "moves none"
/// on a sample of postflop spots is told apart from "moves none where it applies".
#[test]
fn a_component_is_counted_only_on_the_spots_that_have_it() {
    let on = |name: &str, rec: &ReplayRecord| (VARIANTS.iter().find(|v| v.name == name).unwrap().installed)(rec, &None);
    let bare = preflop_spot(Params { samples: 64, ..Default::default() });
    let fitted =
        preflop_spot(Params { samples: 64, preflop_fold_logit_shift: -1.42, fold_logit_shift: [0.27, 0.0, -0.63], ..Default::default() });
    assert!(!on("preflop fold calibration", &bare), "no shift fitted");
    assert!(on("preflop fold calibration", &fitted));
    assert!(!on("street fold calibration", &fitted), "a postflop shift has nothing to act on preflop");
    assert!(!on("per-opponent river sizing tells", &fitted));
    assert!(!on("response network", &fitted), "no network passed");

    let report = measure(vec![(fitted, None), (bare, None)], &Default::default());
    let row = |name: &str| report.rows.iter().find(|r| r.component == name).unwrap().installed;
    assert_eq!(row("preflop fold calibration"), Some(1));
    assert_eq!(row("street fold calibration"), Some(0));
    assert_eq!(report.streets, vec![("preflop".into(), 2), ("flop".into(), 0), ("turn".into(), 0), ("river".into(), 0)]);
}

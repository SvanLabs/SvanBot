//! Tests for the recent-hands read and the audit-by report, split out of `review_recent.rs`
//! (the 500-line rule).

use super::*;

#[test]
fn recent_reports_95_percent_intervals_from_stored_results() {
    use sv10_store::store::HandRow;
    let dir = std::env::temp_dir().join(format!("sv10-recent-interval-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(&dir.join("hands.db")).unwrap();
    for (i, (net, ev)) in [(-20, -10.0), (20, 10.0)].into_iter().enumerate() {
        let id = format!("h{i}");
        store
            .insert_hand(&HandRow {
                bot: "A".into(),
                hand_id: id.clone(),
                ended_at: format!("2026-09-29T00:00:0{i}Z"),
                net: Some(net),
                summary: r#"{"bb":20}"#.into(),
                ..Default::default()
            })
            .unwrap();
        store.set_ev_nets(&[("A".into(), id, ev)]).unwrap();
    }
    // Population SD is 20 chips for net, 10 for EV. In bb/100 the 95% half-widths
    // are 1.96 * SD / sqrt(2) / 20 * 100 = 138.59 and 69.30 respectively.
    let report = recent(&store, "A", 2).unwrap();
    assert!(report.contains("net +0.0 bb/100 (95% -139..+139)"), "{report}");
    assert!(report.contains("all-in EV +0.0 bb/100 (95% -69..+69)"), "{report}");
    drop(store);
    std::fs::remove_dir_all(dir).unwrap();
}

fn graded(version: Option<u32>, gap: f64) -> Graded {
    (
        "2026-09-27T00:00:00Z".into(),
        sv10_store::store::AuditResult {
            street: "turn".into(),
            live_action: "raise".into(),
            gap_bb: gap,
            replay_version: version,
            ..Default::default()
        },
    )
}

/// 0316: a decision-loss class must be readable on the records that carry the live inputs, and a
/// mixed window must say so — the turn/river findings (0282–0284) were measured on records that
/// did not, and this is the instrument that re-measures them.
#[test]
fn a_measurement_can_be_restricted_to_one_replay_version_and_says_what_it_mixed() {
    let rows = vec![graded(None, 9.0), graded(Some(2), 5.0), graded(Some(3), 1.0), graded(Some(3), 1.0)];
    // All four rows, and the header names what they are made of, largest share first.
    assert_eq!(version_mix(&rows), "v3 2, version not recorded 1, v2 1");
    // A version filter takes only that version — never the unrecorded rows, which could be either.
    let only3 = select_version(rows.clone(), Some(3));
    assert_eq!(only3.iter().map(|(_, r)| r.gap_bb).collect::<Vec<_>>(), [1.0, 1.0]);
    assert_eq!(select_version(rows.clone(), Some(2)).len(), 1);
    assert!(select_version(rows.clone(), Some(4)).is_empty(), "a version nobody graded");
    assert_eq!(select_version(rows.clone(), None).len(), 4, "no filter keeps the whole window");
    // The note fires only for a mixed, unfiltered table, and names the fix.
    let note = mixed_note(&rows, None);
    // Two of the four rows are not the current version: the unrecorded one and the v2 one.
    assert!(note.contains("2 of 4") && note.contains("audit-by <days> 3"), "{note}");
    assert!(mixed_note(&rows, Some(3)).is_empty(), "a filtered table is not mixed");
    assert!(mixed_note(&[graded(Some(3), 1.0)], None).is_empty(), "all current: nothing to say");
}

/// 0346: a disagreement is not a preference. Most of what the deep search's modal column says it
/// "would have played" is another *size* of the action we took, so a class counts the two apart
/// and a row read as "it would have checked" (0281) can no longer be read off it.
#[test]
fn a_size_change_is_counted_apart_from_a_different_action() {
    let row = |live: &str, deep: &str, gap: f64| {
        (
            "2026-09-27T00:00:00Z".into(),
            sv10_store::store::AuditResult {
                street: "turn".into(),
                live_action: live.into(),
                deep_action: deep.into(),
                gap_bb: gap,
                replay_version: Some(3),
                ..Default::default()
            },
        )
    };
    let rows = vec![
        row("raise:800", "raise:1605", 2.0),
        row("raise:800", "raise:400", 1.0),
        row("raise:800", "check", 3.0),
        row("raise:800", "raise:800", 0.0),
    ];
    let classes = audit_classes(&rows);
    assert_eq!(classes.len(), 1, "the sized actions are one (street, action) class");
    let ((street, action), c) = &classes[0];
    assert_eq!((street.as_str(), action.as_str()), ("turn", "raise"));
    assert_eq!((c.n, c.total), (4, 6.0));
    // Two keep the action and change its size; one changes the action; the exact match is neither.
    assert_eq!((c.size_only, c.other_action), (2, 1));
    assert_eq!(c.deep["raise:1605"], 1, "every best candidate is counted, an agreement included");
}

/// #723: `--json` is one parseable object over the same `AuditResult` verdicts and class rows the
/// table is built from, and the empty-window text is what an operator read before the flag existed.
#[test]
fn the_audit_json_parses_and_the_empty_text_is_unchanged() {
    let dir = std::env::temp_dir().join(format!("sv10-audit-json-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(&dir.join("t.db")).unwrap();
    assert_eq!(audit_by(&store, 8, None, false).unwrap(), "no analyst re-solves in the last 8 days");
    let json: serde_json::Value = serde_json::from_str(&audit_by(&store, 8, Some(3), true).unwrap()).unwrap();
    assert_eq!((json["days"].as_i64(), json["version"].as_u64()), (Some(8), Some(3)));
    assert_eq!(json["graded"], 0);
    assert_eq!(json["classes"].as_array().map(Vec::len), Some(0));
    assert_eq!(json["verdicts"].as_array().map(Vec::len), Some(0));
    assert!(json["population"].as_str().unwrap().contains("deep search re-solves"), "{json}");

    // A seeded window: two graded raises in one class, so the populated shape is pinned and the
    // JSON's numbers are the ones the text prints (review of #723).
    let verdict = |live: &str, deep: &str, gap: f64| sv10_store::store::AuditResult {
        bot: "A".into(),
        hand_id: "h1".into(),
        street: "turn".into(),
        live_action: live.into(),
        deep_action: deep.into(),
        gap_bb: gap,
        replay_version: Some(3),
        ..Default::default()
    };
    for (live, deep, gap) in [("raise:800", "raise:1605", 2.0), ("raise:800", "check", 3.0)] {
        store.insert_audit("A", "h1", "{}", None).unwrap();
        let id = store.audit_batch(1).unwrap()[0].id;
        store.finish_audit(id, Some(&verdict(live, deep, gap))).unwrap();
    }
    let text = audit_by(&store, 8, None, false).unwrap();
    assert!(text.contains(&format!("   {:<8} {:<8} {:>6} {:>11.1}", "turn", "raise", 2, 5.0)), "{text}");
    let json: serde_json::Value = serde_json::from_str(&audit_by(&store, 8, None, true).unwrap()).unwrap();
    assert_eq!((json["graded"].as_u64(), json["total_gap_bb"].as_f64()), (Some(2), Some(5.0)), "{json}");
    assert_eq!(json["verdicts"].as_array().map(Vec::len), Some(2));
    assert_eq!(json["verdicts"][0]["gap_bb"], 2.0);
    assert_eq!((json["classes"][0]["street"].as_str(), json["classes"][0]["action"].as_str()), (Some("turn"), Some("raise")));
    assert_eq!((json["classes"][0]["class"]["n"].as_u64(), json["classes"][0]["class"]["size_only"].as_u64()), (Some(2), Some(1)));
    assert_eq!(json["classes"][0]["class"]["other_action"], 1);
}

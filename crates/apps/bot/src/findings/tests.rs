//! Tests for the finding loop (0273, 0316, 0328), split out of `findings.rs` (the 500-line rule).

use super::*;

/// A class whose verdicts all gave up exactly the mean, so the spread is zero and the 95% lower bound
/// is the mean: a test then varies only the sample floor or the floor itself. The spread is what
/// separates a leak from a handful of decisions, so the tests that care about it build their stat by
/// hand.
fn class(n: i64, total: f64) -> (ClassKey, ClassStat) {
    let mean = total / n as f64;
    (
        ClassKey::Spot { street: "turn".into(), action: "raise".into() },
        ClassStat { n, total, sumsq: total * mean, big: 0, days: GAP_WINDOW_DAYS },
    )
}

/// A class above the per-decision threshold is a finding; below it, and below the sample
/// floor, is not. Both filters are what keep this from filing a ticket per hand.
#[test]
fn a_decision_loss_needs_both_a_rate_and_a_sample() {
    let big = BTreeMap::from([class(5_000, 5_000.0)]);
    let found = decision_losses(&big, &no_coverage(), &Shapes::new());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "decision-loss:turn:raise");
    assert_eq!(found[0].severity, "P0");
    assert!((found[0].value - 1.0).abs() < 1e-9, "1.0 bb per decision");

    let thin = decision_losses(&BTreeMap::from([class(100, 100.0)]), &no_coverage(), &Shapes::new());
    assert!(thin.is_empty(), "a rate on 100 decisions is not a rate");

    let quiet = decision_losses(&BTreeMap::from([class(5_000, 10.0)]), &no_coverage(), &Shapes::new());
    assert!(quiet.is_empty(), "0.002 bb per decision is inside the instrument's noise");
}

/// 0345: the mean being over the floor is not enough — the 95% lower bound has to clear it. One 30 bb
/// decision inside 600 is a mean of 0.05 with an interval that still contains zero, and a filed claim
/// nobody can act on is worse than a measurement of the same class.
#[test]
fn a_decision_loss_needs_its_lower_bound_over_the_floor() {
    let lumpy = ClassStat { n: 600, total: 30.0, sumsq: 900.0, big: 1, days: GAP_WINDOW_DAYS };
    assert!(lumpy.mean() > GAP_BB_PER_DECISION, "the point estimate clears the floor");
    assert!(lumpy.lower_bound() < GAP_BB_PER_DECISION, "but the interval does not");
    let classes = BTreeMap::from([(ClassKey::Spot { street: "turn".into(), action: "raise".into() }, lumpy)]);
    assert!(decision_losses(&classes, &no_coverage(), &Shapes::new()).is_empty(), "one decision is not a leak");

    // The same total given up, spread over the class instead of riding on one decision.
    let even = ClassStat { sumsq: 30.0 * lumpy.mean(), ..lumpy };
    assert!(even.lower_bound() >= GAP_BB_PER_DECISION);
    assert_eq!(
        decision_losses(
            &BTreeMap::from([(ClassKey::Spot { street: "turn".into(), action: "raise".into() }, even)]),
            &no_coverage(),
            &Shapes::new()
        )
        .len(),
        1
    );
}

/// 0345, R2: the all-in family is one class over the four streets. It is the only grouping that is
/// defensible — every all-in is a big pot, the analyst audits all of them, and no single street can
/// ever reach the sample floor inside any window the store keeps.
#[test]
fn the_all_in_family_is_pooled_across_streets() {
    let rows = [
        verdict(Some(3), "river", "all_in:4000", 20.0),
        verdict(Some(3), "turn", "all_in:900", 0.0),
        verdict(Some(3), "flop", "all_in:120", 0.0),
        verdict(Some(3), "preflop", "all_in:100", 0.0),
    ];
    let (classes, _, excluded) = comparable_classes(rows.iter(), DECISION_LOSS_DAYS);
    assert_eq!(excluded, 0);
    let pooled = classes[&ClassKey::AllIn];
    assert_eq!((pooled.n, pooled.big, pooled.total, pooled.days), (4, 1, 20.0, DECISION_LOSS_DAYS), "one class over four streets");
    let street = ClassKey::Spot { street: "river".into(), action: "all_in".into() };
    assert_eq!(classes[&street].n, 1, "and the street keeps its own line");
    // A grouped raise is still one class per street: nothing else is pooled.
    assert_eq!(classes.len(), 5, "four streets plus the pool: {classes:?}");

    // 0345's measured case: the pooled class on the long window, the only formulation that clears.
    let stats =
        BTreeMap::from([(ClassKey::AllIn, ClassStat { n: 1_315, total: 200.0, sumsq: 2_600.0, big: 26, days: DECISION_LOSS_DAYS })]);
    let found = decision_losses(&stats, &no_coverage(), &Shapes::new());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "decision-loss:all_in", "not decision-loss:all_in:all_in");
    assert!(
        found[0].evidence.contains("all_in, every street") && found[0].evidence.contains("26 gave up >= 1 bb"),
        "{}",
        found[0].evidence
    );
}

/// 0345, R1: a class is tested on the short window when that window holds the sample the filing rule
/// needs, and on the retention-long one when it cannot — the all-in family accrues ~44 comparable
/// verdicts a day, so in 8 days it is hundreds short and in 30 days it is measured.
#[test]
fn a_class_is_tested_on_the_short_window_unless_it_cannot_be() {
    let key = ClassKey::Spot { street: "river".into(), action: "all_in".into() };
    let thin = ClassStat { n: 40, total: 8.0, sumsq: 6.4, big: 2, days: GAP_WINDOW_DAYS };
    let full = ClassStat { n: 1_315, total: 120.0, sumsq: 900.0, big: 20, days: DECISION_LOSS_DAYS };
    let tested = tested_classes(&BTreeMap::from([(key.clone(), thin)]), &BTreeMap::from([(key.clone(), full)]));
    assert_eq!(
        (tested[&key].n, tested[&key].days),
        (1_315, DECISION_LOSS_DAYS),
        "8 days hold 40 verdicts, so the long window is the one tested"
    );
    assert_eq!(decision_losses(&tested, &no_coverage(), &Shapes::new()).len(), 1, "and it is a finding on the window that can speak");

    // The short window holding the floor: it is the tested one, so a leak fixed in a release clears in
    // days rather than weeks — and the long window does not get to overrule it.
    let current = ClassStat { n: 600, total: 6.0, sumsq: 0.06, big: 0, days: GAP_WINDOW_DAYS };
    let stale = ClassStat { n: 2_000, total: 200.0, sumsq: 20.0, big: 100, days: DECISION_LOSS_DAYS };
    let tested = tested_classes(&BTreeMap::from([(key.clone(), current)]), &BTreeMap::from([(key.clone(), stale)]));
    assert_eq!((tested[&key].n, tested[&key].days), (600, GAP_WINDOW_DAYS));
    assert!(decision_losses(&tested, &no_coverage(), &Shapes::new()).is_empty(), "a leak fixed in the last 8 days has cleared");
    assert_eq!(tested.len(), 1, "and every class the long window holds is tested");
}

/// 0345, R3: a class the instrument cannot test is measured, not invisible. It carries its count,
/// window, total, interval and the tally over [`BIG_GAP_BB`] — and a class that filed is left to its
/// own `P0`, so no class appears twice.
#[test]
fn every_class_is_printed_even_the_ones_that_cannot_file_yet() {
    let thin = (
        ClassKey::Spot { street: "river".into(), action: "all_in".into() },
        ClassStat { n: 40, total: 8.0, sumsq: 6.4, big: 2, days: GAP_WINDOW_DAYS },
    );
    let classes = BTreeMap::from([thin, class(2_000, 200.0)]);
    let filed = decision_losses(&classes, &no_coverage(), &Shapes::new());
    assert_eq!(filed.len(), 1);
    let rows = measurements(&classes, &filed, &no_coverage(), &Shapes::new());
    assert_eq!(rows.len(), 1, "the class that filed is not printed twice");
    assert_eq!(rows[0].id, "decision-measurement:river:all_in");
    assert_eq!(rows[0].severity, "P2", "a measurement, not a leak (0269)");
    for want in ["40 deep re-solves over 8 days", "8.0 bb given up", "95%", "2 gave up >= 1 bb"] {
        assert!(rows[0].evidence.contains(want), "no {want:?} in {}", rows[0].evidence);
    }
}

/// 0328: the two risk instruments shared one function, so the scan called it twice and every
/// nemesis came back twice — two entries on the panel, and two tickets, the second of which the
/// close path could never find. One scan, one finding per signature.
#[test]
fn a_nemesis_appears_once_in_a_scan() {
    let dir = std::env::temp_dir().join(format!("sv10-findings-scan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(&dir.join("svanbot10.db")).unwrap();
    let mut bully = HeadToHead::default();
    for i in 0..200 {
        bully.add(if i % 2 == 0 { -60.0 } else { -40.0 });
    }
    let h2h = std::collections::HashMap::from([("Bully".to_string(), bully)]);
    let scan = scan(&store, &h2h, 1_000.0);
    assert_eq!(
        scan.findings.iter().filter(|f| f.id == "nemesis:Bully").count(),
        1,
        "a second copy is a second row on the panel and a second ticket if it is ever P0"
    );
    assert_eq!(merge(&Value::Null, &scan)["findings"].as_array().unwrap().len(), 1, "and the stored set holds it once");
    // 0361: the row names the population it was measured on, not only the correction across tested
    // opponents — a reader cannot see the ledger's read from here (LESSONS 39).
    let nemesis = scan.findings.iter().find(|f| f.id == "nemesis:Bully").unwrap();
    for want in ["200 shared hands of champion play", "experiment-arm hands excluded", "family-wise across"] {
        assert!(nemesis.evidence.contains(want), "no {want:?} in {}", nemesis.evidence);
    }
}

/// The stored set holds one entry per signature even when a caller hands over a repeat: the
/// panel is served this list raw, and a duplicated `P0` signature would file two tickets, only
/// the first of which the close path can ever find (0328).
#[test]
fn a_duplicate_signature_merges_to_one_entry() {
    let loss = || Finding::new("decision-loss:turn:raise", "P0", "turn raise costs 0.03 bb", String::new(), 0.03, 100.0);
    let stored = merge(&Value::Null, &Scan { at: 100.0, findings: vec![loss(), loss()], ..Default::default() });
    assert_eq!(stored["findings"].as_array().unwrap().len(), 1);
    assert_eq!(to_file(&stored, 5).len(), 1, "one finding, one ticket");
}

/// The same scan twice changes nothing: a repeat is recognised, keeps its `since` and does not
/// file a second ticket.
#[test]
fn a_repeated_finding_keeps_its_age_and_its_ticket() {
    let classes = BTreeMap::from([class(2_000, 200.0)]);
    let first =
        merge(&Value::Null, &Scan { at: 100.0, findings: decision_losses(&classes, &no_coverage(), &Shapes::new()), ..Default::default() });
    let one = to_file(&first, 5);
    assert_eq!(one.len(), 1, "the first scan files it");
    let mut stored = first.clone();
    let f = stored["findings"][0]["id"].as_str().unwrap().to_string();
    stored["findings"][0]["ticket"] = json!("0282-some-ticket");
    stored["findings"][0]["since"] = json!(50.0);

    let second =
        merge(&stored, &Scan { at: 200.0, findings: decision_losses(&classes, &no_coverage(), &Shapes::new()), ..Default::default() });
    assert!(to_file(&second, 5).is_empty(), "already filed, so nothing new");
    assert_eq!(second["findings"][0]["since"], json!(50.0), "and it keeps its age");
    assert_eq!(second["findings"][0]["ticket"], json!("0282-some-ticket"));
    assert_eq!(second["findings"].as_array().unwrap().len(), 1);
    let _ = f;
}

/// A finding that stops reproducing is reported as cleared — that is what closes its ticket,
/// and without it the board fills with work that no longer exists.
#[test]
fn a_finding_that_clears_is_reported_as_cleared() {
    let stored = merge(
        &Value::Null,
        &Scan {
            at: 100.0,
            findings: decision_losses(&BTreeMap::from([class(2_000, 200.0)]), &no_coverage(), &Shapes::new()),
            ..Default::default()
        },
    );
    let after = merge(&stored, &Scan { at: 300.0, findings: vec![], ..Default::default() });
    assert_eq!(after["findings"].as_array().unwrap().len(), 0);
    assert_eq!(after["cleared"].as_array().unwrap()[0]["id"], json!("decision-loss:turn:raise"));
}

/// The loop end to end: a new P0 finding files a ticket, a repeat does not, and a finding that
/// stops reproducing closes the ticket it filed. This is the "automatic solution" the ticket
/// asks for, and it is the part that can quietly do damage, so it is tested.
#[test]
fn the_fleet_files_its_own_ticket_and_closes_it_when_the_finding_clears() {
    let root = std::env::temp_dir().join(format!("sv10-findings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let issues = root.join(".scratch").join("svanbot10").join("issues");
    std::fs::create_dir_all(&issues).unwrap();
    let hot = BTreeMap::from([class(2_000, 200.0)]);

    // First scan: the loss is above the floor, so a ticket is filed.
    let first =
        merge(&Value::Null, &Scan { at: 1_000.0, findings: decision_losses(&hot, &no_coverage(), &Shapes::new()), ..Default::default() });
    let (filed, closed) = write_tickets(&root, &first, 1_000.0);
    assert_eq!(filed.len(), 1, "one new finding files one ticket");
    assert!(closed.is_empty());
    let name = &filed[0];
    let text = std::fs::read_to_string(issues.join(format!("{name}.md"))).unwrap();
    assert!(text.contains("status: open"), "and it is a real open ticket");
    assert!(text.contains("assignee: fleet"), "assigned to whoever found it");
    assert!(text.contains("found-by-fleet"), "labelled so it is never mistaken for a session's idea");
    assert!(text.contains("decision-loss:turn:raise"), "naming the class it is about");
    // Record the ticket on the finding, as the loop does.
    let mut stored = first.clone();
    stored["findings"][0]["ticket"] = json!(name);
    stored["findings"][0]["since"] = json!(1_000.0);

    // Second scan, same finding: nothing new, nothing touched.
    let second =
        merge(&stored, &Scan { at: 2_000.0, findings: decision_losses(&hot, &no_coverage(), &Shapes::new()), ..Default::default() });
    let (filed, _) = write_tickets(&root, &second, 2_000.0);
    assert!(filed.is_empty(), "a repeat is not a new ticket");
    assert!(std::fs::read_to_string(issues.join(format!("{name}.md"))).unwrap().contains("status: open"));

    // The class stops reproducing — measured on enough decisions, under the floor: the ticket closes itself.
    let measured = BTreeMap::from([("decision-loss:turn:raise".to_string(), 2_000)]);
    let third = merge(&second, &Scan { at: 3_000.0, sampled: measured, ..Default::default() });
    let (filed, closed) = write_tickets(&root, &third, 3_000.0);
    assert!(filed.is_empty());
    assert_eq!(closed, vec![name.clone()], "the cleared finding closes its own ticket");
    let text = std::fs::read_to_string(issues.join(format!("{name}.md"))).unwrap();
    assert!(text.contains("status: resolved"), "with the resolution recorded:\n{text}");
    assert!(text.contains("stopped reproducing"), "and the reason for closing it");

    let _ = std::fs::remove_dir_all(&root);
}

/// The coverage argument for a test that is not about coverage: an empty map, which every row renders
/// as "coverage unknown" rather than as counts nobody read (0355).
fn no_coverage() -> Coverages {
    Coverages::new()
}

fn verdict(version: Option<u32>, street: &str, action: &str, gap: f64) -> (String, sv10_store::store::AuditResult) {
    let r = sv10_store::store::AuditResult {
        street: street.into(),
        live_action: action.into(),
        gap_bb: gap,
        replay_version: version,
        ..Default::default()
    };
    ("2026-09-27T00:00:00Z".into(), r)
}

/// One stored verdict, straight into the verdict table and with the timestamp a test chooses: the
/// queue path stamps `now`, which puts every row in every window at once.
fn verdict_row(tx: &rusqlite::Transaction<'_>, days_ago: i64, version: Option<u32>, street: &str, action: &str, gap: f64) {
    tx.execute(
        "INSERT INTO decision_audit (ts, bot, hand_id, street, live_action, deep_action, gap_bb, pot_bb, deep_ms, samples, replay_version)
         VALUES (?1, 'A', 'h', ?2, ?3, ?4, ?5, 0, 0, 0, ?6)",
        rusqlite::params![
            (chrono::Utc::now() - chrono::Duration::days(days_ago)).to_rfc3339(),
            street,
            action,
            action,
            gap,
            version.map(i64::from)
        ],
    )
    .unwrap();
}

/// 0316: a verdict graded on a record without the live inputs compares the live choice with a
/// model that saw less than the live choice did, so it is not decision-loss evidence. Only
/// records at [`LIVE_INPUTS_REPLAY_VERSION`] or later count, and the excluded ones are counted.
#[test]
fn only_verdicts_graded_on_the_live_inputs_are_decision_loss_evidence() {
    let rows = [
        verdict(None, "river", "raise:400", 5.0),
        verdict(Some(2), "river", "raise:400", 5.0),
        verdict(Some(3), "river", "raise:400", 0.25),
        verdict(Some(3), "river", "raise:900", 0.75),
        verdict(Some(4), "turn", "call", -1.0),
    ];
    let (classes, _, excluded) = comparable_classes(rows.iter(), GAP_WINDOW_DAYS);
    assert_eq!(excluded, 2, "the unversioned and the v2 verdict are not evidence");
    let raised = classes[&ClassKey::Spot { street: "river".into(), action: "raise".into() }];
    assert_eq!((raised.n, raised.total), (2, 1.0), "sized raises are one family");
    assert_eq!((raised.big, raised.days), (0, GAP_WINDOW_DAYS), "neither was a whole decision, and the window is the one fetched");
    let called = classes[&ClassKey::Spot { street: "turn".into(), action: "call".into() }];
    assert_eq!((called.n, called.total), (1, 0.0), "a later version counts; a negative gap is no loss");
    assert_eq!(classes.len(), 2, "a class with one verdict has no interval: {classes:?}");
    assert!(!called.stderr().is_finite() && called.interval().is_none(), "and no interval may be read out of it");
}

/// 0345, R1 end to end: one read covers the retention-long window and the short test window is sliced
/// out of the same rows, so a class that can speak inside 8 days is tested there — the class below
/// files on the short window and would not file on the long one — while a class that cannot falls back
/// to 30 days and is measured there rather than reported as unmeasured.
#[test]
fn the_scan_tests_each_class_on_the_window_that_can_speak() {
    let dir = std::env::temp_dir().join(format!("sv10-findings-window-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(&dir.join("svanbot10.db")).unwrap();
    // Written straight into the verdict table so the window is the test's to choose: the queue path
    // always stamps `now`, which puts every row in both windows.
    let conn = rusqlite::Connection::open(dir.join("svanbot10.db")).unwrap();
    let tx = conn.unchecked_transaction().unwrap();
    let put = |days_ago: i64, version: Option<u32>, street: &str, action: &str, gap: f64| {
        verdict_row(&tx, days_ago, version, street, action, gap)
    };
    // 600 turn calls at 0.03 bb in the last day, 900 at zero three weeks ago: the short window is a
    // loss, the long one (0.012 bb per decision) is not — the 8-day window is what has to be tested.
    for _ in 0..600 {
        put(1, Some(LIVE_INPUTS_REPLAY_VERSION), "turn", "call", 0.03);
    }
    for _ in 0..900 {
        put(20, Some(LIVE_INPUTS_REPLAY_VERSION), "turn", "call", 0.0);
    }
    // 40 all-ins in the last day (one of them a whole decision), 1,300 older ones: too few to test in 8
    // days, measured in 30 (1,340 verdicts, 0.091 bb per decision on the lower bound).
    put(1, Some(LIVE_INPUTS_REPLAY_VERSION), "river", "all_in:4000", 12.0);
    for _ in 0..39 {
        put(1, Some(LIVE_INPUTS_REPLAY_VERSION), "river", "all_in:120", 0.0);
    }
    for _ in 0..20 {
        put(20, Some(LIVE_INPUTS_REPLAY_VERSION), "river", "all_in:2000", 10.0);
    }
    for _ in 0..1_280 {
        put(20, Some(LIVE_INPUTS_REPLAY_VERSION), "river", "all_in:80", 0.0);
    }
    // One verdict graded before the live inputs: named in the note, never counted.
    put(1, Some(2), "river", "call", 9.0);
    tx.commit().unwrap();

    let scan = scan(&store, &std::collections::HashMap::new(), 1_000.0);
    let finding = |id: &str| scan.findings.iter().find(|f| f.id == id).unwrap_or_else(|| panic!("no {id} in {:?}", scan.findings));
    let calls = finding("decision-loss:turn:call");
    assert_eq!(calls.severity, "P0");
    assert!(calls.evidence.contains("600 deep re-solves over 8 days"), "{}", calls.evidence);
    assert_eq!(scan.sampled["decision-loss:turn:call"], 600, "the tested window's count, not the 1,500 the long one holds");
    let pooled = finding("decision-loss:all_in");
    assert!(pooled.evidence.contains("1340 deep re-solves over 30 days"), "{}", pooled.evidence);
    assert_eq!(scan.sampled["decision-loss:all_in"], 1_340, "the pooled class is tested on the long window, not reported as unmeasured");
    // #321: the note leads the panel, so it is a coverage line and not one of the footnote's questions.
    let note = scan.coverage.iter().find(|u| u.starts_with("1 of 2841")).expect("the excluded verdict is named");
    assert!(note.contains("1 of 2841 re-solves in the last 30 days"), "{note}");
    assert!(scan.unanswered.iter().all(|u| !u.starts_with("decision loss:")), "and not in the footnote: {:?}", scan.unanswered);
    // 0344's note counts the window's comparable verdicts, not the class tallies — which would count
    // the pooled all-in family's 1,340 a second time — and names the largest street class.
    assert!(note.contains("2840 comparable verdicts") && note.contains("(largest turn:call, 1500)"), "{note}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 0316: a finding that clears because its class no longer has enough comparable decisions must
/// not close its ticket as "stopped reproducing" — nobody measured that. The ticket says what did
/// happen, and a class that is measured and under the floor keeps the old wording.
#[test]
fn a_finding_that_loses_its_evidence_says_so_and_does_not_claim_it_stopped_reproducing() {
    let root = std::env::temp_dir().join(format!("sv10-findings-thin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let issues = root.join(".scratch").join("svanbot10").join("issues");
    std::fs::create_dir_all(&issues).unwrap();
    let hot = BTreeMap::from([class(2_000, 200.0)]);
    let mut stored =
        merge(&Value::Null, &Scan { at: 1_000.0, findings: decision_losses(&hot, &no_coverage(), &Shapes::new()), ..Default::default() });
    let (filed, _) = write_tickets(&root, &stored, 1_000.0);
    stored["findings"][0]["ticket"] = json!(filed[0]);

    // Only 40 comparable decisions in the class now: the finding clears, but as unmeasured.
    let thin = Scan { at: 2_000.0, sampled: BTreeMap::from([("decision-loss:turn:raise".to_string(), 40)]), ..Default::default() };
    let after = merge(&stored, &thin);
    assert_eq!(after["cleared"][0]["measured"], json!(40));
    let (_, closed) = write_tickets(&root, &after, 2_000.0);
    assert_eq!(closed, vec![filed[0].clone()]);
    let text = std::fs::read_to_string(issues.join(format!("{}.md", filed[0]))).unwrap();
    assert!(!text.contains("stopped reproducing"), "an unmeasured class did not stop reproducing:\n{text}");
    assert!(text.contains("40 decisions") && text.contains("carry the live inputs"), "{text}");
    assert!(text.contains("files it again"), "and it says what happens if it reproduces:\n{text}");

    // Measured on enough comparable decisions and under the floor: that is a real clear.
    assert_eq!(cleared_reason(Some(900)), None, "a measured class keeps the stopped-reproducing wording");
    assert!(cleared_reason(None).is_some(), "a class with no comparable decisions at all is unmeasured too");
    let _ = std::fs::remove_dir_all(&root);
}

/// A failed read is unknown, not a clean bill (0324/0326's rule, here for the findings): an
/// instrument whose store read failed keeps the findings it had, with their tickets, instead of
/// clearing them — which would close every decision-loss ticket whenever the database was busy.
#[test]
fn an_unreadable_instrument_keeps_its_findings_and_their_tickets() {
    let mut stored = merge(
        &Value::Null,
        &Scan {
            at: 100.0,
            findings: decision_losses(&BTreeMap::from([class(2_000, 200.0)]), &no_coverage(), &Shapes::new()),
            ..Default::default()
        },
    );
    stored["findings"][0]["ticket"] = json!("0400-decision-loss-turn-raise");
    let busy = Scan { at: 200.0, unreadable: vec!["decision-loss:".into()], ..Default::default() };
    let after = merge(&stored, &busy);
    assert!(after["cleared"].as_array().unwrap().is_empty(), "nothing cleared on a failed read: {after}");
    assert_eq!(after["findings"][0]["id"], json!("decision-loss:turn:raise"));
    assert_eq!(after["findings"][0]["ticket"], json!("0400-decision-loss-turn-raise"), "the ticket stays attached");
    // Another instrument's failure does not shield this one: a readable, empty class still clears.
    let other = Scan { at: 300.0, unreadable: vec!["calibration:".into()], ..Default::default() };
    assert_eq!(merge(&stored, &other)["cleared"].as_array().unwrap().len(), 1);
}

/// A scan that could not measure something says so, rather than looking like a clean bill.
#[test]
fn an_unanswered_question_is_named() {
    let scan = Scan { at: 1.0, unanswered: vec!["decision loss: no analyst re-solves".into()], ..Default::default() };
    assert_eq!(scan.unanswered.len(), 1);
    assert!(scan.findings.is_empty());
    let merged = merge(&Value::Null, &scan);
    assert_eq!(merged["unanswered"].as_array().unwrap().len(), 1);
}

/// #321: the two halves of the panel the issue is about. A family's explanation is stated once per
/// scan instead of inside every row, and it resolves by the finding's own id — `decision-loss:` and
/// `decision-measurement:` read the same one without either being named twice.
#[test]
fn a_scan_states_one_explanation_per_family_and_the_rows_carry_their_numbers() {
    let dir = std::env::temp_dir().join(format!("sv10-findings-legends-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(&dir.join("svanbot10.db")).unwrap();
    let scan = scan(&store, &std::collections::HashMap::new(), 1_000.0);
    for key in ["decision", "calibration", "style-drift"] {
        assert!(scan.legends.contains_key(key), "no {key} legend in {:?}", scan.legends.keys());
    }
    let decision = legend_for(&scan.legends, "decision-loss:turn:call").expect("the class rows find it");
    assert_eq!(legend_for(&scan.legends, "decision-measurement:turn:call"), Some(decision), "both families share one");
    assert!(decision.contains("0.02 bb per decision") && decision.contains("500 comparable verdicts"), "{decision}");
    assert_eq!(legend_for(&scan.legends, "nemesis:Bully"), None, "a family with nothing shared states nothing");
    // And a filed ticket carries the legend, because it has no panel above it to state it (#321).
    let filed = scan.legends.clone();
    let text = ticket_text(
        &Finding::new("decision-loss:turn:call", "P0", "turn call costs", "600 deep re-solves".into(), 0.03, 0.0),
        legend_for(&filed, "decision-loss:turn:call"),
        1_000.0,
    );
    assert!(text.contains("the deep re-solve takes") && text.contains("0.02 bb per decision"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The nemesis verdict is priced at the big blind it is given, not at a constant (#611).
///
/// A store whose hands are not at bb 20 printed a number that disagreed with the rivals card beside
/// it, which has always been read at the store's own big blind.
#[test]
fn a_nemesis_is_priced_at_the_big_blind_it_is_given() {
    let mut h = HeadToHead::default();
    for _ in 0..200 {
        h.add(-250.0);
    }
    let map = std::collections::HashMap::from([("Bully".to_string(), h)]);
    let at = |bb: f64| {
        let found = nemesis(&map, bb);
        assert_eq!(found.len(), 1, "a constant -250 chips a hand over 200 hands is a nemesis");
        found[0].value
    };
    // -250 chips a hand is -1250 bb/100 at bb 20 and -500 at bb 50: the same ledger, the store's rate.
    assert!((at(20.0) + 1250.0).abs() < 1e-9, "{}", at(20.0));
    assert!((at(50.0) + 500.0).abs() < 1e-9, "{}", at(50.0));
}

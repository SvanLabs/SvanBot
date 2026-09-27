//! What one pass of the scan read and emitted, as the payload 0356 persists.
//!
//! 0352's decision, implemented here: a finding that cannot be re-derived is a number from an
//! unknown era. The inputs go in beside the outputs — the calibration summary as the scan read it,
//! the installed `calibration.v1` table verbatim (the one input that is destroyed, overwritten in
//! place every learner cycle), the comparable classes with the window each was tested on, every
//! finding with its value, and the constants that define the arithmetic — so re-deriving a past
//! finding is a comparison rather than a reconstruction.
//!
//! The payload holds no wall clock, and nothing else that moves between two passes over the same
//! store: [`sv10_store::store::Store::record_scan_snapshot`] writes a row only when this text
//! changes, so a timestamp in here would write a row every half hour and destroy the distinction the
//! row exists to make — an empty table means the scan did not run, not that it ran and saw nothing.

use super::{
    BIG_GAP_BB, Classes, DECISION_LOSS_DAYS, DRIFT_POINTS, DRIFT_Z, Finding, GAP_BB_PER_DECISION, GAP_MIN_DECISIONS, GAP_WINDOW_DAYS,
    LIVE_INPUTS_REPLAY_VERSION, Z95,
};
use serde_json::json;
use sv10_store::store::{CalibrationRow, Store};

/// Bumped when this payload's shape changes, so a stored row names the record era it belongs to
/// rather than being compared with one it is not shaped like.
pub const SNAPSHOT_VERSION: u32 = 1;

/// One pass of the scan's inputs and outputs, ready to be persisted as one row.
pub struct ScanSnapshot<'a> {
    /// The calibration summary as the scan read it: category, samples, mean predicted and realized
    /// bb, and the variance of the residual.
    pub calibration: &'a [CalibrationRow],
    /// The installed `calibration.v1` table, verbatim — the input a finding's corrections were
    /// computed against and which the next learner cycle overwrites. `None` when the store holds
    /// none.
    pub installed: Option<&'a str>,
    /// The classes the scan tested, by finding id, with the window each was tested on. 0351 and 0355
    /// re-quote these, so they are recorded at the sample size they were measured at, never rounded.
    pub classes: &'a Classes,
    /// Every finding and measurement row this pass emitted.
    pub findings: &'a [Finding],
    /// The questions the scan could not answer this pass (no samples yet, or an unreadable
    /// instrument), so an empty finding list is never silent.
    pub unanswered: &'a [String],
}

impl ScanSnapshot<'_> {
    /// The payload: the inputs and the outputs together, plus the constants that define the
    /// arithmetic.
    ///
    /// The scan's `at` and the row's own `ts`/`seen` are the times. The as-of cutoff behind a class
    /// count is `seen` minus that class's own `days`, which is why every class carries the window it
    /// was tested on (0352: an as-of-`T` summary is recomputable only if the row records `T`).
    pub fn payload(&self) -> String {
        let calibration: Vec<serde_json::Value> = self
            .calibration
            .iter()
            .map(|(category, n, predicted, realized, variance)| {
                json!({"category": category, "n": n, "predicted": predicted, "realized": realized, "variance": variance})
            })
            .collect();
        let classes: serde_json::Map<String, serde_json::Value> = self
            .classes
            .iter()
            .map(|(key, s)| {
                (key.id(), json!({"n": s.n, "total": s.total, "mean": s.mean(), "stderr": s.stderr(), "big": s.big, "days": s.days}))
            })
            .collect();
        let findings: Vec<serde_json::Value> = self
            .findings
            .iter()
            .map(|f| json!({"id": f.id, "severity": f.severity, "title": f.title, "evidence": f.evidence, "value": f.value}))
            .collect();
        json!({
            "snapshot_version": SNAPSHOT_VERSION,
            "calibration": calibration,
            "installed_calibration": self.installed,
            "classes": serde_json::Value::Object(classes),
            "findings": findings,
            "unanswered": self.unanswered,
            "constants": {
                "live_inputs_replay_version": LIVE_INPUTS_REPLAY_VERSION,
                "gap_bb_per_decision": GAP_BB_PER_DECISION,
                "gap_min_decisions": GAP_MIN_DECISIONS,
                "gap_window_days": GAP_WINDOW_DAYS,
                "decision_loss_days": DECISION_LOSS_DAYS,
                "big_gap_bb": BIG_GAP_BB,
                "z95": Z95,
                "drift_points": DRIFT_POINTS,
                "drift_z": DRIFT_Z,
            },
        })
        .to_string()
    }
}

/// One pass's payload, assembled from the store and the pieces the scan has in hand (0356).
///
/// The installed correction table is read here because it is the input this payload exists to
/// preserve: a single kv slot the learner overwrites in place every cycle, destroyed rather than
/// merely unrecorded. A read that fails becomes an `unanswered` line, so the payload says the
/// corrections are unknown rather than absent.
pub fn scan_payload(
    store: &Store,
    calibration: &[CalibrationRow],
    classes: &Classes,
    findings: &[Finding],
    unanswered: &mut Vec<String>,
) -> String {
    let installed = match store.get_kv(crate::CALIBRATION_KEY) {
        Ok(value) => value,
        Err(e) => {
            unanswered.push(format!("calibration table: the installed corrections were unreadable ({e})"));
            None
        }
    };
    ScanSnapshot { calibration, installed: installed.as_deref(), classes, findings, unanswered: &*unanswered }.payload()
}

#[cfg(test)]
mod tests {
    use super::super::scan;
    use super::*;
    use std::collections::HashMap;
    use sv10_store::store::{AuditResult, Store};

    /// A store of its own: the payload records what the scan read, so every test needs a store it
    /// controls.
    fn store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("sv10-findings-snapshot-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Store::open(&dir.join("svanbot10.db")).unwrap()
    }

    /// One deep re-solve verdict graded on a record with the live inputs, through the public paths
    /// the analyst uses.
    fn graded(store: &Store, street: &str, action: &str, gap: f64) {
        store.insert_audit("A", "h", "{}", None).unwrap();
        let job = store.audit_batch(1).unwrap().pop().unwrap();
        let result = AuditResult {
            bot: "A".into(),
            hand_id: "h".into(),
            street: street.into(),
            live_action: action.into(),
            deep_action: action.into(),
            gap_bb: gap,
            pot_bb: 10.0,
            deep_ms: 100.0,
            samples: 1_000,
            replay_version: Some(LIVE_INPUTS_REPLAY_VERSION),
        };
        store.finish_audit(job.id, Some(&result)).unwrap();
    }

    /// The five things 0352 says the row must hold, in one pass over a store that has all of them.
    #[test]
    fn the_payload_holds_the_inputs_and_the_outputs_of_a_pass() {
        let store = store("holds");
        for _ in 0..500 {
            graded(&store, "turn", "call", 0.03);
        }
        for _ in 0..200 {
            store.insert_calibration("A", "h", "river:call", 10.0, 14.0, 20.0).unwrap();
        }
        let installed = "{\"river:call\":{\"bias\":1.5,\"n\":200}}";
        store.put_kv(crate::CALIBRATION_KEY, installed).unwrap();

        let scan = scan(&store, &HashMap::new(), 1_000.0);
        let v: serde_json::Value = serde_json::from_str(&scan.snapshot).unwrap();
        assert_eq!(v["snapshot_version"], SNAPSHOT_VERSION, "the row names the record era it was written in");
        // 2. the correction table a finding was computed against, verbatim.
        assert_eq!(v["installed_calibration"], installed);
        // 1. the calibration summary as the scan read it.
        assert_eq!(v["calibration"][0]["category"], "river:call");
        assert_eq!((v["calibration"][0]["n"].as_i64(), v["calibration"][0]["realized"].as_f64()), (Some(200), Some(14.0)));
        // 3. the comparable classes, with the window each was tested on.
        let class = &v["classes"]["decision-loss:turn:call"];
        assert_eq!(class["n"], 500);
        assert_eq!(class["days"], GAP_WINDOW_DAYS, "the window it was tested on, so its cutoff is recoverable");
        assert!((class["mean"].as_f64().unwrap() - 0.03).abs() < 1e-9, "{class}");
        // 4. every finding and measurement row, with its value.
        let findings = v["findings"].as_array().unwrap();
        let row = |id: &str| findings.iter().find(|f| f["id"] == id).unwrap_or_else(|| panic!("no {id} in {findings:?}")).clone();
        let loss = row("decision-loss:turn:call");
        assert_eq!((loss["severity"].as_str(), loss["value"].as_f64().unwrap().round()), (Some("P0"), 0.0));
        assert!(row("calibration:river:call")["value"].as_f64().unwrap() > 3.9, "the residual the category realizes");
        // 5. the constants that define the arithmetic, so a superseded era is identifiable.
        let constants = &v["constants"];
        assert_eq!(constants["live_inputs_replay_version"], LIVE_INPUTS_REPLAY_VERSION);
        assert_eq!(constants["gap_min_decisions"], GAP_MIN_DECISIONS);
        assert_eq!(constants["gap_window_days"], GAP_WINDOW_DAYS);
        assert_eq!(constants["decision_loss_days"], DECISION_LOSS_DAYS);
        assert_eq!(constants["gap_bb_per_decision"], GAP_BB_PER_DECISION);
    }

    /// 0356's load-bearing property, end to end: the payload has no clock in it, so two passes over
    /// the same store are byte-identical, the table grows only when the scan's view changes, and an
    /// unchanged pass is not a row.
    #[test]
    fn two_passes_over_an_unchanged_store_are_one_row_and_a_changed_input_is_a_second() {
        let store = store("dedup");
        store.put_kv(crate::CALIBRATION_KEY, "{\"river:call\":1.5}").unwrap();
        for _ in 0..20 {
            graded(&store, "turn", "call", 0.01);
        }

        let first = scan(&store, &HashMap::new(), 1_000.0);
        let second = scan(&store, &HashMap::new(), 2_000.0);
        assert_eq!(first.snapshot, second.snapshot, "a later `now` and the same store must be the same bytes");
        assert!(store.record_scan_snapshot(&first.snapshot).unwrap(), "the first pass writes its inputs and outputs");
        assert!(!store.record_scan_snapshot(&second.snapshot).unwrap(), "the second pass changed nothing, so it writes nothing");
        assert_eq!(store.scan_snapshots(10).unwrap().len(), 1, "a year of no movement stays one row");

        // The input that is destroyed every learner cycle: one changed correction is a new era, and
        // the row from before it is what makes the change visible at all.
        store.put_kv(crate::CALIBRATION_KEY, "{\"river:call\":2.0}").unwrap();
        let third = scan(&store, &HashMap::new(), 3_000.0);
        assert_ne!(third.snapshot, first.snapshot, "the installed table is in the payload verbatim");
        assert!(store.record_scan_snapshot(&third.snapshot).unwrap(), "a changed scan writes its own row");
        let rows = store.scan_snapshots(10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].payload.as_str(), rows[1].payload.as_str()), (third.snapshot.as_str(), first.snapshot.as_str()));
    }
}

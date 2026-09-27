//! The wiring table (0316): what each live component is worth on the analyst's newest recorded big
//! decisions, served from the stored measurement (`wiring.v1`).

use super::*;

/// The wiring table (0316): what each live component is worth on the analyst's newest recorded big
/// decisions, so "this component is active" is a measured figure rather than a claim about the code.
pub(super) async fn wiring(State(s): State<Arc<Shared>>) -> Response {
    off_runtime(move || Json(wiring_value(&s))).await.into_response()
}

/// [`crate::review_wiring::WiringReport`] as the panel reads it, with the sample and the age beside
/// the figures. A failed read says so: an unreadable row is `available: false`, never an empty table
/// that would read as "no component is worth anything".
pub(super) fn wiring_value(s: &Shared) -> Value {
    wiring_json(s.store.get_kv(crate::review_wiring::WIRING_KEY), now_secs())
}

/// The row read, as JSON: split from [`wiring_value`] so the three cases that matter — no row, a
/// broken read and a stale row — are testable without a live store.
fn wiring_json(raw: Result<Option<String>>, now: f64) -> Value {
    let raw = match raw {
        Ok(v) => v,
        Err(e) => return json!({"available": false, "reason": format!("store unreadable ({e})")}),
    };
    let Some(report) = raw.and_then(|j| serde_json::from_str::<crate::review_wiring::WiringReport>(&j).ok()) else {
        return json!({"available": false, "reason": "the analyst has not measured the wiring table yet"});
    };
    let age_secs = now - report.at;
    // The analyst re-measures daily: two days without a fresh row means it was not running, and the
    // panel says so instead of showing yesterday's table as today's.
    json!({"available": true, "age_secs": age_secs, "stale": age_secs > 2.0 * 86_400.0, "report": report})
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0316: the wiring panel shows what the analyst measured, and says what the measurement is
    /// worth. Every way of not having a measurement reads as unavailable — never as an empty table,
    /// which a reader would take for "no component is worth anything".
    #[test]
    fn the_wiring_panel_reports_what_it_measured_and_never_an_empty_table() {
        let unreachable = wiring_json(Err(anyhow::anyhow!("database is locked")), 1_000.0);
        assert_eq!(unreachable["available"], false);
        assert!(unreachable["reason"].as_str().unwrap().contains("store unreadable"), "{unreachable}");
        assert_eq!(wiring_json(Ok(None), 1_000.0)["available"], false, "nothing measured yet");
        assert_eq!(wiring_json(Ok(Some("not json".into())), 1_000.0)["available"], false, "an unreadable row is not a table");
        let report = crate::review_wiring::WiringReport {
            at: 1_000.0,
            sample: 200,
            unstable: 0,
            exact: 131,
            exact_with_current: 132,
            carrying_corrections: 30,
            v3: 30,
            v3_exact: 30,
            chosen_not_best: 1,
            rows: vec![crate::review_wiring::Row {
                component: "response network".into(),
                changed: 4,
                share_pct: 2.0,
                cost_bb: 0.5,
                max_bb: 41.0,
            }],
            calibration: vec![crate::review_wiring::CalibrationFlips {
                street: "preflop".into(),
                decisions: 10,
                flipped: 6,
                ..Default::default()
            }],
        };
        let stored = serde_json::to_string(&report).unwrap();
        let fresh = wiring_json(Ok(Some(stored.clone())), 1_000.0 + 3_600.0);
        assert_eq!((&fresh["available"], &fresh["stale"], &fresh["age_secs"]), (&json!(true), &json!(false), &json!(3_600.0)));
        assert_eq!(fresh["report"]["rows"][0]["component"], "response network");
        assert_eq!(fresh["report"]["calibration"][0]["flipped"], 6, "the calibration count travels with the table");
        assert_eq!(fresh["report"]["sample"], 200, "the sample travels with the figures");
        // Two days without a refresh: the analyst was not running, and the panel says so.
        assert_eq!(wiring_json(Ok(Some(stored)), 1_000.0 + 3.0 * 86_400.0)["stale"], json!(true));
    }
}

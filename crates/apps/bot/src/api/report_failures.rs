//! Regression coverage for reports that must distinguish missing data from an unreadable store.
use super::*;

#[tokio::test]
async fn compute_distinguishes_empty_decisions_from_unreadable_decisions() {
    let shared = Shared::for_test("api-compute-read-failure", &["A"]);
    let response = compute(State(shared.clone())).await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let empty: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(empty["decisions"]["n"], 0);
    let conn = rusqlite::Connection::open(shared.config.artifacts.join("svanbot10.db")).unwrap();
    conn.execute_batch("ALTER TABLE decisions RENAME TO decisions_hidden").unwrap();
    let response = compute(State(shared.clone())).await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let error: Value = serde_json::from_slice(&body).unwrap();
    assert!(error["detail"].as_str().unwrap().contains("decision latencies"));
    assert!(error.get("decisions").is_none(), "an outage must not report zero decisions");
    conn.execute_batch("ALTER TABLE decisions_hidden RENAME TO decisions").unwrap();
    assert_eq!(compute(State(shared)).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn readable_empty_reports_remain_successful() {
    let shared = Shared::for_test("api-empty-reports", &["A"]);
    for (name, status) in [
        ("accuracy", decision_accuracy(State(shared.clone())).await.status()),
        ("calibration", calibration(State(shared.clone())).await.status()),
        ("fleet", fleet(State(shared.clone())).await.status()),
        ("highlights", highlights(State(shared.clone())).await.status()),
        ("stories", stories(State(shared.clone())).await.status()),
        ("player card", player_card(State(shared.clone()), Path("A".into())).await.status()),
    ] {
        assert_eq!(status, StatusCode::OK, "readable empty {name} report");
    }
}

#[tokio::test]
async fn unreadable_reports_return_errors_instead_of_empty_successes() {
    let shared = Shared::for_test("api-unreadable-reports", &["A"]);
    let conn = rusqlite::Connection::open(shared.config.artifacts.join("svanbot10.db")).unwrap();
    conn.execute_batch(
        "ALTER TABLE hands RENAME TO hands_hidden;
         ALTER TABLE kv RENAME TO kv_hidden;
         ALTER TABLE decision_audit RENAME TO decision_audit_hidden;",
    )
    .unwrap();
    let statuses = [
        ("accuracy", decision_accuracy(State(shared.clone())).await.status()),
        ("calibration", calibration(State(shared.clone())).await.status()),
        ("fleet", fleet(State(shared.clone())).await.status()),
        ("highlights", highlights(State(shared.clone())).await.status()),
        ("stories", stories(State(shared.clone())).await.status()),
        ("player card", player_card(State(shared.clone()), Path("A".into())).await.status()),
        ("unknown player card", player_card(State(shared.clone()), Path("Unknown".into())).await.status()),
        ("analysis", analysis(State(shared)).await.into_response().status()),
    ];
    let failures: Vec<_> = statuses.into_iter().filter(|(_, status)| *status != StatusCode::INTERNAL_SERVER_ERROR).collect();
    assert!(failures.is_empty(), "unreadable report endpoints falsely succeeded: {failures:?}");
}

#[tokio::test]
async fn unreadable_leak_hands_do_not_become_an_empty_cached_report() {
    let shared = Shared::for_test("api-unreadable-leak-hands", &["A"]);
    let conn = rusqlite::Connection::open(shared.config.artifacts.join("svanbot10.db")).unwrap();
    conn.execute_batch("ALTER TABLE hands RENAME TO hands_hidden").unwrap();
    assert_eq!(analysis(State(shared)).await.into_response().status(), StatusCode::INTERNAL_SERVER_ERROR);
}

#[tokio::test]
async fn corrupt_calibration_rows_are_errors_instead_of_missing_fits() {
    let mut failures = Vec::new();
    for (index, key) in [crate::CALIBRATION_KEY, crate::foldcal::FOLD_CAL_KEY, crate::raisewar::RIVER_JAM_KEY].into_iter().enumerate() {
        let shared = Shared::for_test(&format!("api-corrupt-calibration-{index}"), &["A"]);
        shared.store.put_kv(key, "not json").unwrap();
        let status = calibration(State(shared)).await.status();
        if status != StatusCode::INTERNAL_SERVER_ERROR {
            failures.push((key, status));
        }
    }
    assert!(failures.is_empty(), "corrupt calibration rows falsely succeeded: {failures:?}");
}

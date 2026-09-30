use rusqlite::params;
use serde_json::json;
use sv10_store::store::Store;

#[test]
fn fold_cal_report_counts_preflop_and_postflop_separately() {
    let root = std::env::temp_dir().join(format!("sv10-fold-cal-report-{}", std::process::id()));
    let path = root.join("artifacts/svanbot10.db");
    let store = Store::open(&path).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    let summary = json!({"bb":20, "players":[[0,"Hero"],[1,"Villain"]], "history":[
        {"seat":0,"street":"Preflop","kind":"Raise","to":60},
        {"seat":1,"street":"Preflop","kind":"Call","to":60},
        {"seat":0,"street":"Flop","kind":"Raise","to":40,"to_call_before":0},
        {"seat":1,"street":"Flop","kind":"Fold","to":0}
    ]});
    conn.execute(
        "INSERT INTO hands (bot, hand_id, ended_at, summary) VALUES ('Hero','h','2026-09-30T00:00:00Z',?1)",
        params![summary.to_string()],
    )
    .unwrap();
    for (street, amount) in [("preflop", 60), ("flop", 40)] {
        let detail = json!({"opponents":1,"candidates":[{"action":"raise","amount":amount,"fold_prob":0.5}]}).to_string();
        store.insert_decision("Hero", "h", street, "raise", Some(amount), None, 120, 0, 1.0, &detail).unwrap();
    }
    let report = || {
        let output =
            std::process::Command::new(env!("CARGO_BIN_EXE_review")).arg("fold-cal").env("SVANBOT10_ROOT", &root).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap()
    };
    let mixed = report();
    assert!(mixed.lines().any(|l| l.split_whitespace().take(3).eq(["preflop", "n", "1"])), "{mixed}");
    assert!(mixed.lines().any(|l| l == "1 heads-up postflop bets"), "{mixed}");
    conn.execute("DELETE FROM decisions WHERE street = 'flop'", []).unwrap();
    let preflop_only = report();
    assert!(preflop_only.lines().any(|l| l == "0 heads-up postflop bets"), "{preflop_only}");
}

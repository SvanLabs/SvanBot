//! Force the migration's read/write race with a second real SQLite connection.
use super::*;
use std::cell::RefCell;

thread_local! {
    static AFTER_READ: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
}

pub(super) fn after_read() {
    let hook = AFTER_READ.with(|h| h.borrow_mut().take());
    if let Some(hook) = hook {
        hook();
    }
}

fn fixture(tag: &str) -> (Store, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("sv10-migration-{tag}-{}", std::process::id()));
    let path = dir.join("svanbot10.db");
    let store = Store::open(&path).unwrap();
    store
        .insert_hand(&HandRow {
            bot: "A".into(),
            hand_id: "h".into(),
            ended_at: "2026-09-30T00:00:00Z".into(),
            hero_seat: Some(0),
            summary: serde_json::json!({"players":[[0,"A"]],"button":0,"bb":20,"history":[],"board":[],"shown":[],"stacks":[]}).to_string(),
            ..Default::default()
        })
        .unwrap();
    (store, path)
}

fn rewrite_on_read(path: std::path::PathBuf) {
    AFTER_READ.with(|h| {
        *h.borrow_mut() = Some(Box::new(move || {
            let conn = Connection::open(path).unwrap();
            let summary = serde_json::json!({"players":[[0,"A"]],"button":0,"bb":20,"history":[],"board":[],
                "shown":[[0,["Ah","Kd"]]],"stacks":[]})
            .to_string();
            let digest = crate::integrity::hand_digest("A", "h", "2026-09-30T00:00:00Z", "", "", &summary);
            serde_json::from_str::<sv10_model::model::HandSummary>(&summary).unwrap();
            conn.execute("UPDATE hands SET summary=?1, digest=?2, showdown=1 WHERE hand_id='h'", params![summary, digest]).unwrap();
        }));
    });
}

fn expected_snapshot_conflict(result: Result<()>) {
    if let Err(error) = result {
        let Some(rusqlite::Error::SqliteFailure(code, _)) = error.downcast_ref::<rusqlite::Error>() else {
            panic!("unexpected migration failure: {error}");
        };
        assert_eq!(code.code, rusqlite::ErrorCode::DatabaseBusy, "{error}");
    }
}

#[test]
fn digest_backfill_never_overwrites_a_concurrently_rewritten_hand() {
    let (store, path) = fixture("digest");
    store.write_lock().execute("UPDATE hands SET digest=NULL", []).unwrap();
    rewrite_on_read(path);
    expected_snapshot_conflict(store.backfill_digests());
    let (checked, bad) = store.verify_hand_digests().unwrap();
    assert_eq!(checked, 1);
    assert!(bad.is_empty(), "migration overwrote the peer's correct digest: {bad:?}");
    store.backfill_digests().unwrap();
    assert!(store.verify_hand_digests().unwrap().1.is_empty(), "retry must remain consistent");
}

#[test]
fn showdown_backfill_never_overwrites_a_concurrently_rewritten_hand() {
    let (store, path) = fixture("showdown");
    store.write_lock().execute("UPDATE hands SET showdown=NULL", []).unwrap();
    rewrite_on_read(path);
    expected_snapshot_conflict(store.backfill_showdown());
    let conn = store.read();
    let (summary, showdown): (String, i64) =
        conn.query_row("SELECT summary, showdown FROM hands", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert!(summary.contains("Ah"), "peer write must have committed");
    assert_eq!(showdown, 1, "migration installed the flag from its stale summary");
    drop(conn);
    store.write_lock().execute("UPDATE hands SET showdown=NULL", []).unwrap();
    store.backfill_showdown().unwrap();
    let showdown: i64 = store.read().query_row("SELECT showdown FROM hands", [], |r| r.get(0)).unwrap();
    assert_eq!(showdown, 1, "retry derives the flag from the newer summary");
}

#[test]
fn uncontended_backfills_fill_missing_digest_and_showdown() {
    let (store, _) = fixture("uncontended");
    store.write_lock().execute("UPDATE hands SET digest=NULL, showdown=NULL", []).unwrap();
    store.backfill_showdown().unwrap();
    store.backfill_digests().unwrap();
    let (checked, bad) = store.verify_hand_digests().unwrap();
    assert_eq!(checked, 1);
    assert!(bad.is_empty());
    let showdown: i64 = store.read().query_row("SELECT showdown FROM hands", [], |r| r.get(0)).unwrap();
    assert_eq!(showdown, 0);
}

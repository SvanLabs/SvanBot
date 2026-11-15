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

thread_local! {
    static PEER: RefCell<Option<std::thread::JoinHandle<()>>> = const { RefCell::new(None) };
}

/// While a backfill holds its transaction, a peer tries to rewrite the hand. The backfill opens
/// immediate, so the peer's write waits for the backfill's lock and lands after it: the backfill can
/// never install a value computed from a summary the peer has since replaced (1012; before, the
/// deferred transaction raced the peer and failed with a snapshot conflict). [`join_peer`] waits for it.
fn rewrite_on_read(path: std::path::PathBuf) {
    AFTER_READ.with(|h| {
        *h.borrow_mut() = Some(Box::new(move || {
            let peer = std::thread::spawn(move || {
                let conn = Connection::open(path).unwrap();
                conn.busy_timeout(std::time::Duration::from_secs(10)).unwrap();
                let summary = serde_json::json!({"players":[[0,"A"]],"button":0,"bb":20,"history":[],"board":[],
                    "shown":[[0,["Ah","Kd"]]],"stacks":[]})
                .to_string();
                let digest = crate::integrity::hand_digest("A", "h", "2026-09-30T00:00:00Z", "", "", &summary);
                serde_json::from_str::<sv10_model::model::HandSummary>(&summary).unwrap();
                conn.execute("UPDATE hands SET summary=?1, digest=?2, showdown=1 WHERE hand_id='h'", params![summary, digest]).unwrap();
            });
            // Long enough for the peer to be waiting on the backfill's lock.
            std::thread::sleep(std::time::Duration::from_millis(150));
            PEER.with(|p| *p.borrow_mut() = Some(peer));
        }));
    });
}

fn join_peer() {
    PEER.with(|p| p.borrow_mut().take()).expect("the hook ran").join().unwrap();
}

#[test]
fn digest_backfill_never_overwrites_a_concurrently_rewritten_hand() {
    let (store, path) = fixture("digest");
    store.write_lock().execute("UPDATE hands SET digest=NULL", []).unwrap();
    rewrite_on_read(path);
    store.backfill_digests().unwrap();
    join_peer();
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
    store.backfill_showdown().unwrap();
    join_peer();
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

/// A peer process commits a write after holding SQLite's write lock for 200 ms. Returns once the
/// lock is taken; join the handle after the call under test.
/// The peer is opened by the caller first: opening a store runs the backfills, which would fill the
/// rows the test is about to leave empty.
fn peer_holds_the_write_lock(peer: Store) -> std::thread::JoinHandle<()> {
    let (taken, lock_taken) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let held = peer.write_lock();
        let tx = held.unchecked_transaction().unwrap();
        tx.execute("INSERT INTO kv (key, value, updated) VALUES ('held', '1', 't')", []).unwrap();
        taken.send(()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        tx.commit().unwrap();
    });
    lock_taken.recv().unwrap();
    holder
}

/// A read-then-write transaction opened deferred upgrades from a read, and SQLite does not call the
/// busy handler for an upgrade: it failed at once whenever a peer was writing. The three below open
/// immediate, like `update_kv`, so they wait for the lock first (1012).
#[test]
fn the_digest_backfill_waits_for_a_peer_that_is_writing() {
    let (store, path) = fixture("wait-digest");
    let peer = Store::open(&path).unwrap();
    store.write_lock().execute("UPDATE hands SET digest=NULL", []).unwrap();
    let holder = peer_holds_the_write_lock(peer);
    store.backfill_digests().expect("the backfill must wait for the write lock, not fail at once");
    holder.join().unwrap();
    assert!(store.verify_hand_digests().unwrap().1.is_empty());
}

#[test]
fn the_showdown_backfill_waits_for_a_peer_that_is_writing() {
    let (store, path) = fixture("wait-showdown");
    let peer = Store::open(&path).unwrap();
    store.write_lock().execute("UPDATE hands SET showdown=NULL", []).unwrap();
    let holder = peer_holds_the_write_lock(peer);
    store.backfill_showdown().expect("the backfill must wait for the write lock, not fail at once");
    holder.join().unwrap();
    let showdown: Option<i64> = store.read().query_row("SELECT showdown FROM hands", [], |r| r.get(0)).unwrap();
    assert_eq!(showdown, Some(0));
}

#[test]
fn compaction_waits_for_a_peer_that_is_writing() {
    use crate::packed::{REPLAY_RECORD, compact_candidates, compact_rows};
    let (store, path) = fixture("wait-compact");
    store.write_lock().execute("INSERT INTO replays (id, ts, bot, record) VALUES (1, 't', 'A', '{\"a\":1}')", []).unwrap();
    let rows = compact_candidates(&store.read(), &store.codec, REPLAY_RECORD, 0, 10).unwrap();
    assert_eq!(rows.len(), 1);
    let holder = peer_holds_the_write_lock(Store::open(&path).unwrap());
    let (written, _) = compact_rows(&store.write_lock(), &store.codec, REPLAY_RECORD, &rows)
        .expect("compaction must wait for the write lock, not fail at once");
    holder.join().unwrap();
    assert_eq!(written, 1);
}

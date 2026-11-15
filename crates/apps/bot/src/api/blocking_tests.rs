//! Dashboard handlers must not hold an async worker while they wait on the store (#1016). A second
//! connection holds the write lock on a scratch store; a heartbeat task on the same single-threaded
//! runtime then shows whether the handler under test let the runtime breathe.

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const HOLD: Duration = Duration::from_millis(600);
const TICK: Duration = Duration::from_millis(10);
/// Well under the hold: a handler that waits on the lock off the runtime leaves the heartbeat near 10 ms.
const MAX_STALL: Duration = Duration::from_millis(250);

/// Run `call` while another connection holds the store's write lock for `HOLD`; the longest gap
/// between heartbeat ticks, in milliseconds, is how long the runtime was stalled.
async fn stall_under_write_lock<F: Future>(shared: &Arc<Shared>, call: F) -> u64 {
    let db = shared.config.artifacts.join("svanbot10.db");
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let conn = rusqlite::Connection::open(db).unwrap();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        locked_tx.send(()).unwrap();
        std::thread::sleep(HOLD);
        conn.execute_batch("COMMIT").unwrap();
    });
    locked_rx.recv().unwrap();
    let worst = Arc::new(AtomicU64::new(0));
    let beat = tokio::spawn({
        let worst = worst.clone();
        async move {
            let mut last = Instant::now();
            loop {
                tokio::time::sleep(TICK).await;
                worst.fetch_max(last.elapsed().as_millis() as u64, Ordering::Relaxed);
                last = Instant::now();
            }
        }
    });
    tokio::task::yield_now().await;
    call.await;
    // One more beat records the gap the call left.
    tokio::time::sleep(TICK * 3).await;
    beat.abort();
    holder.join().unwrap();
    worst.load(Ordering::Relaxed)
}

fn assert_runtime_free(name: &str, stalled_ms: u64) {
    println!("{name}: runtime stalled {stalled_ms} ms under a {} ms write lock", HOLD.as_millis());
    assert!(stalled_ms < MAX_STALL.as_millis() as u64, "{name} stalled the async runtime for {stalled_ms} ms");
}

#[tokio::test]
async fn saving_notes_does_not_stall_the_runtime_behind_a_write_lock() {
    let shared = Shared::for_test("blocking-notes", &["A"]);
    let stalled = stall_under_write_lock(&shared, save_notes(State(shared.clone()), Json(json!({"text": "x"})))).await;
    assert_runtime_free("save_notes", stalled);
}

#[tokio::test]
async fn saving_the_layout_does_not_stall_the_runtime_behind_a_write_lock() {
    let shared = Shared::for_test("blocking-layout", &["A"]);
    let stalled = stall_under_write_lock(&shared, save_layout(State(shared.clone()), Json(json!({"left": ["a"]})))).await;
    assert_runtime_free("save_layout", stalled);
}

#[tokio::test]
async fn a_bot_command_does_not_stall_the_runtime_behind_a_write_lock() {
    let shared = Shared::for_test("blocking-command", &["A"]);
    let call = bot_command(State(shared.clone()), Path(0), Json(json!({"command": "pause"})));
    let stalled = stall_under_write_lock(&shared, call).await;
    assert_runtime_free("bot_command", stalled);
}

#[tokio::test]
async fn the_training_handlers_do_not_stall_the_runtime_behind_a_write_lock() {
    let shared = Shared::for_test("blocking-training", &["A"]);
    let call = training_settings(State(shared.clone()), Json(json!({"cooldown_minutes": 5})));
    assert_runtime_free("training_settings", stall_under_write_lock(&shared, call).await);
    let call = compute_profile(State(shared.clone()), Json(json!({"name": "quiet"})));
    assert_runtime_free("compute_profile", stall_under_write_lock(&shared, call).await);
    let call = training_command(State(shared.clone()), Json(json!({"command": "start"})));
    assert_runtime_free("training_command", stall_under_write_lock(&shared, call).await);
}

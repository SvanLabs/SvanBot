//! Store rows the decision leaves behind, written off the read loop, and the retry for a write
//! that fails on a locked database.

use super::*;

/// Waits before each retry of a store write that failed on a locked database (0322): the first
/// attempt's 10 s busy timeout has just expired, so the holder is already long; a quarter second and
/// then seconds clear the stalls seen in the logs (one 55 s hold, 2026-09-27 07:33).
const LOCKED_WRITE_WAITS: [Duration; 3] = [Duration::from_millis(250), Duration::from_secs(2), Duration::from_secs(10)];

/// Run `write` again while it fails, waiting before each attempt, and return the last error (0322).
/// A write that gives up on `database is locked` loses its row for good otherwise: hands have a
/// retry queue (0247), the decision record and the analyst's audit do not, and 18 audits were lost
/// that way in ten days. Bounded by the waits it is given, and never on the decision path — callers
/// spawn it.
pub(crate) async fn retry_locked_write(waits: &[Duration], mut write: impl FnMut() -> anyhow::Result<()>) -> anyhow::Result<()> {
    let mut last = anyhow::anyhow!("no attempt made");
    for wait in waits {
        tokio::time::sleep(*wait).await;
        match write() {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// The blocking twin of [`retry_locked_write`], for callers already on the blocking pool (the
/// calibration round): same waits with `std::thread::sleep` so no runtime is needed (#744).
pub(crate) fn retry_locked_write_blocking(mut write: impl FnMut() -> anyhow::Result<()>) -> anyhow::Result<()> {
    let mut last = anyhow::anyhow!("no attempt made");
    for wait in LOCKED_WRITE_WAITS {
        std::thread::sleep(wait);
        match write() {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Retry a store write that failed on a locked database, off the frame loop, and report the outcome.
pub(crate) fn spawn_locked_write_retry(
    shared: &Arc<Shared>,
    bot: &str,
    what: &str,
    write: impl FnMut() -> anyhow::Result<()> + Send + 'static,
) {
    let (shared, bot, what) = (shared.clone(), bot.to_string(), what.to_string());
    tokio::spawn(async move {
        match retry_locked_write(&LOCKED_WRITE_WAITS, write).await {
            Ok(()) => shared.log(&bot, "info", format!("{what} stored on retry")),
            Err(e) => shared.log(&bot, "warn", format!("{what} lost after {} retries: {e}", LOCKED_WRITE_WAITS.len())),
        }
    });
}

/// A row the decision leaves in the store once its action is on the wire.
pub(crate) enum Row {
    Replay {
        hand: String,
        record: String,
        net: Option<(String, String)>,
    },
    Audit {
        hand: String,
        record: String,
        net: Option<(String, String)>,
    },
    Decision {
        hand: String,
        street: String,
        action: String,
        amount: Option<i64>,
        equity: Option<f64>,
        pot: i64,
        to_call: i64,
        latency: f64,
        detail: String,
    },
}

impl Row {
    fn write(&self, shared: &Shared, bot: &str) -> anyhow::Result<()> {
        fn net_ref(net: &Option<(String, String)>) -> Option<(&str, &str)> {
            net.as_ref().map(|(d, j)| (d.as_str(), j.as_str()))
        }
        match self {
            Row::Replay { hand, record, net } => shared.store.insert_replay(bot, hand, record, net_ref(net)),
            Row::Audit { hand, record, net } => shared.store.insert_audit(bot, hand, record, net_ref(net)),
            Row::Decision { hand, street, action, amount, equity, pot, to_call, latency, detail } => {
                shared.store.insert_decision(bot, hand, street, action, *amount, *equity, *pot, *to_call, *latency, detail)
            }
        }
    }

    /// What the log says when the first attempt fails, and what the retry calls the row.
    fn lost(&self) -> (&'static str, &'static str) {
        match self {
            Row::Replay { .. } => ("replay record not stored", "the replay record"),
            Row::Audit { .. } => ("decision not queued for the analyst", "the analyst audit"),
            Row::Decision { .. } => ("decision record not stored", "the decision record"),
        }
    }
}

/// Write the rows off the read loop (1013): a write waits up to the store's 10 s busy budget for
/// a peer's lock, and the loop that reads the next `your_turn` must not sit in that wait. A row
/// that still fails goes to the locked-write retry, as it did when the write ran inline.
pub(crate) fn queue_rows(shared: &Arc<Shared>, bot: &str, rows: Vec<Row>) {
    if rows.is_empty() {
        return;
    }
    let (shared, bot) = (shared.clone(), bot.to_string());
    tokio::spawn(async move {
        let (s, b) = (shared.clone(), bot.clone());
        let failed = crate::jobs::blocking("decision rows", move || {
            rows.into_iter().filter_map(|row| row.write(&s, &b).err().map(|e| (row, e))).collect::<Vec<_>>()
        })
        .await;
        let Some(failed) = failed else {
            shared.log(&bot, "warn", "decision rows lost: the write job panicked");
            return;
        };
        for (row, e) in failed {
            let (warn, what) = row.lost();
            shared.log(&bot, "warn", format!("{warn}: {e}"));
            let (shared2, bot2) = (shared.clone(), bot.clone());
            spawn_locked_write_retry(&shared, &bot, what, move || row.write(&shared2, &bot2));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0322: a store write that gave up on `database is locked` is tried again (a quarter second
    /// then seconds later), and only a write that fails every attempt is reported lost.
    #[tokio::test]
    async fn a_locked_write_is_retried_and_a_persistent_failure_is_reported() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let tries = Arc::new(AtomicUsize::new(0));
        let count = tries.clone();
        let waits = [Duration::ZERO; LOCKED_WRITE_WAITS.len()];
        let landed =
            retry_locked_write(
                &waits,
                move || {
                    if count.fetch_add(1, Ordering::SeqCst) < 2 { anyhow::bail!("database is locked") } else { Ok(()) }
                },
            )
            .await;
        assert!(landed.is_ok(), "the third attempt lands: {landed:?}");
        assert_eq!(tries.load(Ordering::SeqCst), 3);
        let lost = retry_locked_write(&waits, || anyhow::bail!("database is locked")).await.unwrap_err();
        assert!(lost.to_string().contains("database is locked"), "{lost}");
    }

    /// 0322 wiring: every row that is otherwise lost for good is handed to the retry when its
    /// first write fails (the loop above is tested; a real `database is locked` costs the 10 s busy
    /// timeout before it fails, too slow for the gate).
    #[test]
    fn a_locked_decision_record_or_audit_is_handed_to_the_retry() {
        let source = include_str!("rows.rs");
        let body = source.split("#[cfg(test)]").next().unwrap();
        for (lost, what) in [
            ("decision not queued for the analyst", "the analyst audit"),
            ("decision record not stored", "the decision record"),
            ("replay record not stored", "the replay record"),
        ] {
            assert!(body.contains(&format!("(\"{lost}\", \"{what}\")")), "{lost} must be logged and retried as {what}");
        }
        assert!(body.contains("spawn_locked_write_retry(&shared, &bot, what"), "a failed row must be retried");
    }

    /// 1013: a write that waits on a peer's lock must not hold the read loop. The rows are queued
    /// at once and land when the peer lets go.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_decision_row_waits_for_a_locked_store_off_the_read_loop() {
        let shared = Shared::for_test("rows-locked", &["A"]);
        let db = std::env::temp_dir().join(format!("sv10-shared-rows-locked-{}", std::process::id())).join("svanbot10.db");
        let peer = rusqlite::Connection::open(&db).unwrap();
        peer.execute_batch("BEGIN IMMEDIATE").unwrap();
        let row = Row::Decision {
            hand: "h1".into(),
            street: "preflop".into(),
            action: "call".into(),
            amount: None,
            equity: Some(0.5),
            pot: 30,
            to_call: 10,
            latency: 5.0,
            detail: "{}".into(),
        };
        let started = Instant::now();
        queue_rows(&shared, "A", vec![row]);
        assert!(started.elapsed() < Duration::from_millis(100), "queueing must not wait for the lock: {:?}", started.elapsed());
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(shared.store.decisions_for_hand("A", "h1").unwrap().is_empty(), "the peer still holds the lock");
        peer.execute_batch("COMMIT").unwrap();
        for _ in 0..100 {
            if !shared.store.decisions_for_hand("A", "h1").unwrap().is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the decision row never landed after the peer released the lock");
    }
}

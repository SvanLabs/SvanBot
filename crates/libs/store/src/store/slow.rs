//! Watching the main database's write connection (0322). Every writer in every process shares one
//! SQLite write lock; a writer that holds it longer than the others' busy timeout (10 s) makes them
//! fail with `database is locked` — 38 audits, 17 decisions and 6 hand rows were lost that way in
//! ten days of play. Nothing said *who* held it, so the guard returned by
//! [`Store::write_lock`](crate::store::Store::write_lock) times every hold and names the file and
//! line that took it.
//!
//! A hold alone does not name the holder: a writer that waits out the lock is reported at its own
//! site exactly like one that worked for that long, and the worst holds in the live logs are the
//! trivial single-statement writes — the victims. So the wait for SQLite's lock is measured apart
//! from the hold: [`busy_handler`] replaces `busy_timeout` with the same 10 s and the same sleep
//! schedule, and records when this hold first found the lock taken. The report then says whether the
//! hold was this process's own work (it held the lock) or someone else's (it waited), which is what
//! decides between shortening one transaction and taking the I/O off the hot path.

use parking_lot::MutexGuard;
use rusqlite::Connection;
use std::cell::Cell;
use std::ops::{Deref, DerefMut};
use std::path::Path;
use std::time::{Duration, Instant};

/// A write-connection hold or wait longer than this is reported (0322).
pub(crate) const SLOW: Duration = Duration::from_secs(1);

/// One process holding the write connection for this long is enough to fail every other process's
/// writes: they give up after their 10 s busy timeout.
pub(crate) const FAILING: Duration = Duration::from_secs(10);

/// The write connection's busy budget, milliseconds: what `busy_timeout(10 s)` allowed, and what
/// [`busy_handler`] enforces (0322).
const BUSY_MS: u64 = 10_000;

/// SQLite's own sleep schedule before retry N and the total slept before it (`sqliteDefaultBusyCallback`
/// in `sqlite3.c`, the callback `busy_timeout` installs): copied so this handler waits exactly as long
/// as the timeout it replaced, and gives up at the same point.
const DELAYS: [u64; 12] = [1, 2, 5, 10, 15, 20, 25, 25, 25, 50, 50, 100];
/// Total slept before retry N, from the same table.
const TOTALS: [u64; 12] = [0, 1, 3, 8, 18, 33, 53, 78, 103, 128, 178, 228];

thread_local! {
    /// When this thread's current hold first found SQLite's write lock taken, and the budget this
    /// thread waits (the write budget, or what one test set for itself — a shared cell would let one
    /// test's budget decide another's). The handler runs on the thread executing the statement, so
    /// this is the wait of the hold being timed and no two writers can see each other's.
    static WAIT_STARTED: Cell<Option<Instant>> = const { Cell::new(None) };
    static BUDGET_MS: Cell<u64> = const { Cell::new(BUSY_MS) };
}

/// Shorten the busy budget for this thread (0322), so a test can drive a contended write without
/// waiting out the 10 s in play — waiting 10 s twice over is too slow for the gate; the same thread
/// restores it.
#[cfg(test)]
pub(crate) fn set_busy_ms(ms: u64) {
    BUDGET_MS.with(|b| b.set(ms));
}

/// Start timing a hold: contention from an earlier statement is not this hold's wait.
pub(crate) fn arm() {
    WAIT_STARTED.with(|w| w.set(None));
}

/// How long this hold waited for SQLite's write lock, or `None` when it never found it taken.
fn waited() -> Option<Duration> {
    WAIT_STARTED.with(|w| w.take()).map(|started| started.elapsed())
}

/// The write connection's busy handler (0322): the wait `busy_timeout(10 s)` gave, with the time the
/// lock was found taken recorded for [`WriteGuard`] to report.
pub(crate) fn busy_handler(attempts: i32) -> bool {
    let Some(ms) = wait_ms(attempts, BUDGET_MS.with(Cell::get)) else { return false };
    WAIT_STARTED.with(|w| w.set(w.get().or_else(|| Some(Instant::now()))));
    std::thread::sleep(Duration::from_millis(ms));
    true
}

/// How long to sleep before retrying, in milliseconds, or `None` to give up: SQLite's schedule above
/// against `budget`. `attempts` is the count of earlier calls for the same locking event.
fn wait_ms(attempts: i32, budget: u64) -> Option<u64> {
    let count = usize::try_from(attempts).unwrap_or(0);
    let last = DELAYS.len() - 1;
    let (delay, prior) = if count < DELAYS.len() {
        (DELAYS[count], TOTALS[count])
    } else {
        (DELAYS[last], TOTALS[last] + DELAYS[last] * (count - last) as u64)
    };
    if prior + delay > budget {
        // The last sleep is what is left of the budget, as SQLite does it; out of budget is the end.
        let left = budget.checked_sub(prior)?;
        return (left > 0).then_some(left);
    }
    Some(delay)
}

/// One hold of the write connection: what it cost, what it did, and who held the lock.
pub(crate) struct Hold<'a> {
    /// How long the connection was held.
    pub(crate) held: Duration,
    /// How long the caller waited for the in-process connection (another thread of this process).
    pub(crate) waited: Duration,
    /// How much of the hold was waiting for SQLite's write lock (another process held it).
    pub(crate) lock_wait: Option<Duration>,
    /// Rows changed through this connection during the hold.
    pub(crate) rows: u64,
    /// The size of the write-ahead log, when it could be read.
    pub(crate) wal: Option<u64>,
    /// The file and line that took the connection.
    pub(crate) at: &'a str,
    /// The database path.
    pub(crate) db: &'a Path,
}

impl Hold<'_> {
    /// The report for this hold, or `None` when it was quick. Split out from the guard so a test can
    /// check the thresholds and the wording without a running subscriber.
    pub(crate) fn report(&self) -> Option<String> {
        if self.held >= SLOW {
            let fatal = if self.held >= FAILING { " — long enough to fail every other writer" } else { "" };
            let mut parts = vec![match self.lock_wait {
                Some(wait) if wait >= SLOW => format!("waited {:.1} s for sqlite's write lock held by another process", wait.as_secs_f64()),
                Some(wait) => format!("waited {:.0} ms for sqlite's write lock", wait.as_secs_f64() * 1000.0),
                None => "no wait for sqlite's write lock: this process held it".to_string(),
            }];
            parts.push(format!("{} rows changed", self.rows));
            if let Some(bytes) = self.wal {
                parts.push(format!("wal {:.0} MB", bytes as f64 / 1_048_576.0));
            }
            parts.push(self.db.display().to_string());
            return Some(format!(
                "write connection held {:.1} s at {}{fatal} ({}); pid {}",
                self.held.as_secs_f64(),
                self.at,
                parts.join("; "),
                std::process::id()
            ));
        }
        if self.waited >= SLOW {
            return Some(format!(
                "waited {:.1} s for the write connection at {} ({}); pid {}",
                self.waited.as_secs_f64(),
                self.at,
                self.db.display(),
                std::process::id()
            ));
        }
        None
    }
}

/// The write connection, timed: on drop the hold (and the wait for it) is logged when slow, with
/// the caller that took it and whether the lock was this process's or another's.
pub(crate) struct WriteGuard<'a> {
    guard: MutexGuard<'a, Connection>,
    db: &'a Path,
    at: &'static std::panic::Location<'static>,
    rows_at_take: u64,
    started: Instant,
    acquired: Instant,
}

impl Deref for WriteGuard<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.guard
    }
}

impl DerefMut for WriteGuard<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        &mut self.guard
    }
}

impl WriteGuard<'_> {
    /// The report for this hold and wait, if either is over the threshold. Split out of `Drop` so a
    /// test can check what a real guard says, site included.
    fn report(&self) -> Option<String> {
        let at = format!("{}:{}", self.at.file(), self.at.line());
        Hold {
            held: self.acquired.elapsed(),
            waited: self.acquired - self.started,
            lock_wait: waited(),
            rows: self.guard.total_changes().saturating_sub(self.rows_at_take),
            wal: wal_bytes(self.db),
            at: &at,
            db: self.db,
        }
        .report()
    }
}

/// Bytes in the write-ahead log beside `db`, or `None` when it could not be read. Read only for a
/// hold that is already slow: a big WAL is what a checkpoint during a commit has to pay for.
fn wal_bytes(db: &Path) -> Option<u64> {
    let mut wal = db.as_os_str().to_owned();
    wal.push("-wal");
    std::fs::metadata(Path::new(&wal)).ok().map(|m| m.len())
}

impl Drop for WriteGuard<'_> {
    fn drop(&mut self) {
        if let Some(report) = self.report() {
            tracing::warn!("{report}");
        }
    }
}

impl super::Store {
    /// The write connection (see the module docs): timed, and reported with the calling site.
    #[track_caller]
    pub(crate) fn write_lock(&self) -> WriteGuard<'_> {
        let started = Instant::now();
        arm();
        let guard = self.conn.lock();
        WriteGuard {
            rows_at_take: guard.total_changes(),
            guard,
            db: &self.path,
            at: std::panic::Location::caller(),
            started,
            acquired: Instant::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    /// A throwaway store in the test's own directory.
    fn store(name: &str) -> (std::path::PathBuf, Store) {
        let dir = std::env::temp_dir().join(format!("sv10-slow-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        (dir, store)
    }

    /// A hold with everything but the numbers under test.
    fn hold<'a>(held: Duration, waited: Duration, lock_wait: Option<Duration>, at: &'a str, db: &'a Path) -> Hold<'a> {
        Hold { held, waited, lock_wait, rows: 0, wal: None, at, db }
    }

    #[test]
    fn a_long_hold_or_wait_is_reported_with_its_site() {
        let db = Path::new("artifacts/svanbot10.db");
        let quick = Duration::from_millis(50);
        assert!(hold(quick, quick, None, "hands.rs:9", db).report().is_none(), "quick writes say nothing");
        let held = hold(Duration::from_secs(2), quick, None, "hands.rs:9", db).report().unwrap();
        assert!(held.contains("held 2.0 s at hands.rs:9"), "{held}");
        let waited = hold(quick, Duration::from_secs(4), None, "kv.rs:3", db).report().unwrap();
        assert!(waited.contains("waited 4.0 s") && waited.contains("at kv.rs:3"), "{waited}");
        // Ten seconds is where other processes' writes start failing: say so.
        let fatal = hold(FAILING, quick, None, "mod.rs:1", db).report().unwrap();
        assert!(fatal.contains("long enough to fail every other writer"), "{fatal}");
        assert!(!held.contains("fail every other writer"));
    }

    /// The measurement the whole instrumentation exists for (0322): a hold that waited for another
    /// process's lock and one that held it are told apart, though both say the same duration.
    #[test]
    fn a_hold_names_who_held_the_lock_waiting_apart_from_working() {
        let db = Path::new("artifacts/svanbot10.db");
        let victim =
            hold(Duration::from_secs(11), Duration::from_millis(2), Some(Duration::from_secs(10)), "kv.rs:10", db).report().unwrap();
        assert!(victim.contains("waited 10.0 s for sqlite's write lock held by another process"), "{victim}");
        let holder = hold(Duration::from_secs(11), Duration::from_millis(2), None, "hands.rs:148", db).report().unwrap();
        assert!(holder.contains("no wait for sqlite's write lock: this process held it"), "{holder}");
        // A short wait is named in milliseconds; the rows touched and the WAL size ride along.
        let busy = Hold {
            held: Duration::from_secs(3),
            waited: Duration::from_millis(1),
            lock_wait: Some(Duration::from_millis(250)),
            rows: 501,
            wal: Some(64 * 1_048_576),
            at: "hands.rs:148",
            db,
        }
        .report()
        .unwrap();
        assert!(busy.contains("waited 250 ms for sqlite's write lock"), "{busy}");
        assert!(busy.contains("501 rows changed") && busy.contains("wal 64 MB"), "{busy}");
    }

    /// The handler waits exactly as long as the `busy_timeout` it replaced: SQLite's schedule, cut off
    /// at the budget, and nothing to wait for is not a retry.
    #[test]
    fn the_busy_handler_keeps_the_timeout_it_replaced() {
        assert_eq!(wait_ms(0, 10), Some(1), "1 ms, then 2, 5, ...");
        assert_eq!(wait_ms(1, 10), Some(2));
        assert_eq!(wait_ms(2, 10), Some(5));
        assert_eq!(wait_ms(3, 10), Some(2), "the last sleep is what is left of the budget");
        assert_eq!(wait_ms(4, 10), None, "out of budget: give up, like the timeout it replaced");
        assert_eq!(wait_ms(10, BUSY_MS), Some(50), "the default budget keeps retrying in 100 ms steps");
        assert_eq!(wait_ms(11, BUSY_MS), Some(100));
        assert_eq!(wait_ms(108, BUSY_MS), Some(72), "the last sleep is the remainder of the 10 s budget");
        assert_eq!(wait_ms(109, BUSY_MS), None, "10 s of sleeping is the budget, exactly as busy_timeout was");
    }

    /// The whole point of the instrumentation (0322): a guard held past the threshold names the file
    /// and line that took it, so the next stall says who held it.
    #[test]
    fn a_slow_guard_names_the_site_that_took_it() {
        let (dir, store) = store("backdated");
        let report = backdated(&store, Duration::from_secs(11), Duration::from_millis(2)).report().unwrap();
        assert!(report.contains("held 11.0 s at ") && report.contains("slow.rs:"), "{report}");
        assert!(report.contains("long enough to fail every other writer"), "{report}");
        let waited = backdated(&store, Duration::from_millis(2), Duration::from_secs(11)).report().unwrap();
        assert!(waited.contains("waited 11.0 s for the write connection at ") && waited.contains("slow.rs:"), "{waited}");
        assert!(backdated(&store, Duration::from_millis(2), Duration::from_millis(2)).report().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A guard that looks like it was taken `wait + held` ago and held for `held` (backdating beats
    /// sleeping through the threshold; the fields are private to this module).
    fn backdated(store: &Store, held: Duration, wait: Duration) -> WriteGuard<'_> {
        let mut guard = store.write_lock();
        let now = Instant::now();
        guard.acquired = now - held;
        guard.started = now - held - wait;
        guard
    }

    /// 0322: a write transaction that really sleeps past the threshold is reported with the site that
    /// took it, and one that does not is silent. No backdating: the timing is the guard's own.
    #[test]
    fn a_write_that_sleeps_past_the_threshold_is_reported() {
        let (dir, store) = store("sleeping");
        let quick = store.write_lock();
        quick.execute("INSERT INTO kv (key, value, updated) VALUES ('fast', '1', 't')", []).unwrap();
        assert!(quick.report().is_none(), "an ordinary write says nothing");
        drop(quick);
        let slow = store.write_lock();
        slow.execute("INSERT INTO kv (key, value, updated) VALUES ('slow', '1', 't')", []).unwrap();
        std::thread::sleep(SLOW + Duration::from_millis(50));
        let report = slow.report().expect("a write held past the threshold is reported");
        assert!(report.contains("held 1.") && report.contains("slow.rs:"), "{report}");
        assert!(report.contains("1 rows changed"), "{report}");
        assert!(report.contains("no wait for sqlite's write lock: this process held it"), "{report}");
        drop(slow);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 0322 end to end: one connection holds a write transaction while another tries to write. Both
    /// holds last past the threshold and both are reported — the holder as the holder, the waiter as
    /// the waiter, which is what tells a long transaction from a slow disk.
    #[test]
    fn a_write_blocked_by_another_connection_reports_its_sqlite_wait() {
        let dir = std::env::temp_dir().join(format!("sv10-slow-{}-contended", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("svanbot10.db");
        let holder = Store::open(&path).unwrap();
        let waiter = Store::open(&path).unwrap();
        set_busy_ms(1_200);
        let held = holder.write_lock();
        let tx = held.unchecked_transaction().unwrap();
        tx.execute("INSERT INTO kv (key, value, updated) VALUES ('held', '1', 't')", []).unwrap();
        let waiting = waiter.write_lock();
        let err = waiting.execute("INSERT INTO kv (key, value, updated) VALUES ('waited', '1', 't')", []);
        assert!(err.is_err(), "the write waits out the busy budget while the lock is taken: {err:?}");
        let blocked = waiting.report().unwrap();
        assert!(blocked.contains("waited 1.") && blocked.contains("for sqlite's write lock held by another process"), "{blocked}");
        drop(tx);
        let holding = held.report().unwrap();
        assert!(holding.contains("no wait for sqlite's write lock: this process held it"), "{holding}");
        set_busy_ms(BUSY_MS);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

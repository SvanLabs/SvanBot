//! The bot's status file, kept current while it plays. The dashboard reads it; nothing in the
//! bot's play depends on it, so a write failure is reported and play goes on.

use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use svanbot_runtime::ledger::Played;
use svanbot_runtime::status::{Status, Table};

use crate::now_ms;

/// How often a quiet bot refreshes its status. The dashboard calls a bot stale after three.
pub const HEARTBEAT: Duration = Duration::from_secs(5);

/// What a counted hand won, in thousandths of a big blind, with the luck of our draws taken out.
pub fn mbb(played: &Played) -> i64 {
    ((played.net as f64 - played.luck) / f64::from(played.big_blind.max(1)) * 1000.0).round() as i64
}

struct Inner {
    status: Status,
    /// The last status write failure, so a recurring one is logged once.
    write_error: Option<String>,
}

/// One bot's status, written atomically on every change and on every heartbeat.
pub struct Live {
    path: PathBuf,
    inner: Mutex<Inner>,
    stopped: AtomicBool,
}

impl Live {
    /// Writes the starting status and returns the writer. The heartbeat is started separately
    /// with [`Live::spawn_heartbeat`].
    pub fn start(path: PathBuf, status: Status) -> io::Result<Live> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let live = Live {
            path,
            inner: Mutex::new(Inner {
                status,
                write_error: None,
            }),
            stopped: AtomicBool::new(false),
        };
        live.beat();
        Ok(live)
    }

    /// The venue connection opened.
    pub fn connected(&self) {
        self.update(|status| {
            status.connected = true;
            status.last_error = None;
        });
    }

    /// The connection ended. `error` is what the caller reported, `None` for a clean close. A
    /// disconnected bot does not sit at a table, so the last one is cleared; a reconnect sends it
    /// again with [`Live::table`].
    pub fn disconnected(&self, error: Option<String>) {
        self.update(|status| {
            status.connected = false;
            status.table = None;
            status.last_error = error;
        });
    }

    /// The table changed, or the bot left one (`None`).
    pub fn table(&self, table: Option<Table>) {
        self.update(|status| status.table = table);
    }

    /// A hand finished. `kept` says whether the store added it (the first copy wins).
    pub fn hand(&self, kept: bool, at_ms: i64) {
        self.update(|status| {
            if kept {
                status.hands += 1;
            }
            status.last_hand_ms = Some(at_ms);
        });
    }

    /// The ledger took a hand: what it won, if it was counted, the probe as it now stands, and
    /// the champion's digest.
    pub fn played(&self, mbb: Option<i64>, probe: Option<String>, champion: String) {
        self.update(|status| {
            if let Some(mbb) = mbb {
                status.play.count(mbb);
            }
            status.play.probe = probe;
            status.play.champion = champion;
        });
    }

    /// A fair-play incident was raised.
    pub fn incident(&self) {
        self.update(|status| status.incidents += 1);
    }

    /// Refreshes `updated_ms` so the dashboard knows the bot is alive.
    pub fn beat(&self) {
        self.update(|_| {});
    }

    /// Stops the heartbeat thread at its next turn.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    /// Refreshes the status every `every`, until [`Live::stop`].
    pub fn spawn_heartbeat(live: &Arc<Live>, every: Duration) -> JoinHandle<()> {
        let live = Arc::clone(live);
        thread::spawn(move || {
            while !live.stopped.load(Ordering::Relaxed) {
                thread::sleep(every);
                if live.stopped.load(Ordering::Relaxed) {
                    break;
                }
                live.beat();
            }
        })
    }

    fn update(&self, change: impl FnOnce(&mut Status)) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        change(&mut inner.status);
        inner.status.updated_ms = now_ms();
        if let Err(error) = inner.status.write(&self.path) {
            let text = error.to_string();
            if inner.write_error.as_deref() != Some(text.as_str()) {
                eprintln!("svanbot: status: {text}");
                inner.write_error = Some(text);
            }
        } else {
            inner.write_error = None;
        }
    }
}

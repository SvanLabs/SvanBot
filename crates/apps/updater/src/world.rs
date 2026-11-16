//! The updater's [`Window`]: the real waits, against systemd and the bot's health files.
//!
//! `svanbot_runtime::update` never sleeps and never reads a clock. Every wait it needs is a call
//! that takes a deadline and returns the moment the answer is yes — and the loop that polls until
//! then, along with the clock it reads, lives here, in an app, where the clock is allowed
//! (ADR 0003). The two calls that act, `stop` and `start`, are the one thing a file cannot tell
//! the updater: whether systemd has the bot running, which is [`Systemd`]'s business.
//!
//! Three waits make the window: the bot says it stopped (the `.stopped` file the bot writes when
//! it leaves the table), systemd has stopped it, and systemd has it running again. Each poll is
//! short, so a stop that lands just after one is still seen within one poll of it.

use std::io;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use svanbot_runtime::health::Health;
use svanbot_runtime::update::Window;

use crate::systemd::Systemd;

/// How long to wait between polls of a wait. Short: the bot reads the update request within a read
/// timeout (`svanbot_runtime::client::READ_WAIT_MS`), and systemd starts and stops a unit in well
/// under this.
const POLL: Duration = Duration::from_millis(100);

/// The real world: systemd for the process work, the bot's health files for the answer.
pub(crate) struct BotWorld<'a> {
    systemd: &'a Systemd,
    health: &'a Health,
}

impl<'a> BotWorld<'a> {
    /// A world over `systemd` and the bot's `health` files.
    pub(crate) fn new(systemd: &'a Systemd, health: &'a Health) -> Self {
        Self { systemd, health }
    }

    /// Polls `until` — returning `true`/`false` — every [`POLL`] until it does, or until `deadline`
    /// passes, and returns the last answer. The clock is read here, in an app: this is the one place
    /// the deadline is turned into elapsed time.
    fn poll(&self, deadline_ms: i64, until: impl Fn(&Self) -> io::Result<bool>) -> bool {
        loop {
            match until(self) {
                Ok(true) => return true,
                Ok(false) => {}
                // A file we cannot read is not a bot that has stopped: the wait runs to its
                // deadline and the window reports it, rather than guessing.
                Err(error) => eprintln!("svanbot-release: {error}"),
            }
            if now_ms() >= deadline_ms {
                return false;
            }
            thread::sleep(POLL);
        }
    }
}

impl Window for BotWorld<'_> {
    fn stop(&mut self) -> io::Result<()> {
        self.systemd.stop()
    }

    fn bot_stopped(&mut self, deadline_ms: i64) -> bool {
        // The bot writes this only once it has left the table between hands, so waiting for it is
        // what keeps the swap from landing under a hand in flight.
        self.poll(deadline_ms, |world| Ok(world.health.stopped()))
    }

    fn start(&mut self) -> io::Result<()> {
        self.systemd.start()
    }

    fn restarted(&mut self, deadline_ms: i64) -> bool {
        // Asked of systemd, not of the log: a build systemd cannot `exec` is a build that is not
        // running, and the crash loop is judged afterwards by `watch`, from the start log.
        self.poll(deadline_ms, |world| world.systemd.is_running())
    }
}

/// Milliseconds since the Unix epoch, the updater's clock.
pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
        })
}

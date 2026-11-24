//! The updater's [`Window`]: the real waits, against the bot's supervisor and its health files.
//!
//! `svanbot_runtime::update` never sleeps and never reads a clock. Every wait it needs is a call
//! that takes a deadline and returns the moment the answer is yes — and the loop that polls until
//! then, along with the clock it reads, lives here, in an app, where the clock is allowed
//! (ADR 0003). The two calls that act, `stop` and `start`, are the one thing a file cannot tell
//! the updater: whether the supervisor has the bot running, which is [`Supervisor`]'s business.
//!
//! Three waits make the window: the bot says it stopped (the `.stopped` file the bot writes when
//! it leaves the table), the supervisor has stopped it, and it has it running again. Each poll is
//! short, so a stop that lands just after one is still seen within one poll of it.

use std::fmt;
use std::io;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use svanbot_runtime::health::Health;
use svanbot_runtime::update::Window;

use crate::supervisor::Supervisor;

/// How long to wait between polls of a wait. Short: the bot reads the update request within a read
/// timeout (`svanbot_runtime::client::READ_WAIT_MS`), and the supervisor starts and stops a bot in well
/// under this.
const POLL: Duration = Duration::from_millis(100);

/// The real world: the supervisor for the process work, the bot's health files for the answer.
pub(crate) struct BotWorld<'a> {
    supervisor: &'a Supervisor,
    health: &'a Health,
}

impl<'a> BotWorld<'a> {
    /// A world over the bot's `supervisor` and its `health` files.
    pub(crate) fn new(supervisor: &'a Supervisor, health: &'a Health) -> Self {
        Self { supervisor, health }
    }

    /// Polls `until` — returning `true`/`false` — every [`POLL`] until it does, or until `deadline`
    /// passes, and returns the last answer. The clock is read here, in an app: this is the one place
    /// the deadline is turned into elapsed time.
    ///
    /// The error is whatever the wait's own reader gives — an `io::Error` from the supervisor, a
    /// `HealthError` from a file — and is only reported, so the wait can take either.
    fn poll<E: fmt::Display>(
        &self,
        deadline_ms: i64,
        until: impl Fn(&Self) -> Result<bool, E>,
    ) -> bool {
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
        self.supervisor.stop()
    }

    fn bot_stopped(&mut self, deadline_ms: i64) -> bool {
        // The bot writes this only once it has left the table between hands, so waiting for it is
        // what keeps the swap from landing under a hand in flight.
        self.poll(deadline_ms, |world| {
            Ok::<_, io::Error>(world.health.stopped())
        })
    }

    fn start(&mut self) -> io::Result<()> {
        self.supervisor.start()
    }

    fn restarted(&mut self, deadline_ms: i64) -> bool {
        // Asked of the supervisor, not of the log: a build it cannot `exec` is a build that is not
        // running, and the crash loop is judged by `crash_looped`, from the start log.
        self.poll(deadline_ms, |world| world.supervisor.is_running())
    }

    fn crash_looped(&mut self, since_ms: i64, limit: usize, deadline_ms: i64) -> bool {
        // The count is anchored at `since_ms`, when the build went live, so the starts of the build
        // it replaced are behind the window and cannot be counted as this build looping. The wait
        // runs to the deadline, which is what gives the loop the time it needs to show.
        self.poll(deadline_ms, |world| {
            let now = now_ms();
            world
                .health
                .in_crash_loop(now, now.saturating_sub(since_ms), limit)
        })
    }

    fn down(&mut self) -> bool {
        self.supervisor.is_down()
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

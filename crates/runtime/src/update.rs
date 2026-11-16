//! The update window: what happens between a verified release and the build the fleet runs.
//!
//! The order is fixed and never guessed:
//!
//! 1. Ask the bot to stop between hands, and wait for it to say it has. A bot with a hand in
//!    flight keeps playing; the window does not cut in, and a bot that will not stop is left alone
//!    and the window is repeated rather than forced.
//! 2. Ask systemd to stop the bot, retiring the process that just left the table.
//! 3. Point `current` at the new build, keeping the running one in `previous` ([`crate::slot`]).
//! 4. Ask systemd to start the bot again, and watch its start log ([`crate::health`]).
//! 5. Keep the new build, or roll back to the old one.
//!
//! Two things roll back, and both are checked from outside the process: a crash loop, three starts
//! inside five minutes, which is the build failing to survive; and the harm guard, which a caller
//! reports when the canary's live evidence says the new build is worse. Neither one needs the
//! updater to understand poker, and neither is allowed to leave a half-swapped tree: a rollback is
//! the same atomic pointer move as the update.
//!
//! Nothing here sleeps or reads a clock. Every wait is a poll against [`Window`], whose caller owns
//! the process work, and every deadline is passed in as an input, which keeps the whole window
//! testable without a fleet (ADR 0003).

use std::fmt;
use std::io;

use crate::health::{Health, HealthError};
use crate::slot::{SlotError, Slots};

/// What the window needs done in the world, which is systemd's job and the caller's to carry out.
///
/// `stop` and `start` act; the two polls wait. Each wait gets the deadline, in milliseconds since
/// the Unix epoch, past which the answer is no; the caller polls and returns as soon as it has one,
/// so a wait is the caller's loop and not this module's.
pub trait Window {
    /// Asks systemd to stop the bot.
    fn stop(&mut self) -> io::Result<()>;

    /// Whether the bot has said it stopped, by `deadline_ms`.
    fn bot_stopped(&mut self, deadline_ms: i64) -> bool;

    /// Asks systemd to start the bot, from the build under `current`.
    fn start(&mut self) -> io::Result<()>;

    /// Whether systemd has the bot running, by `deadline_ms`.
    fn restarted(&mut self, deadline_ms: i64) -> bool;
}

/// Why an update did not finish.
#[derive(Debug)]
pub enum WindowError {
    /// The bot never said it stopped: a hand in flight, or a bot wedged mid-turn.
    BotNeverStopped { deadline_ms: i64 },
    /// systemd could not stop the bot, so nothing was swapped under a running process.
    Stop(io::Error),
    /// systemd could not start the bot again.
    Start(io::Error),
    /// The slot tree could not be changed.
    Slot(SlotError),
    /// A health file could not be read or written, so the bot and this window cannot talk.
    Health(HealthError),
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BotNeverStopped { deadline_ms } => write!(
                f,
                "update: the bot did not stop before {deadline_ms}; nothing was swapped"
            ),
            Self::Stop(error) => write!(f, "update: could not stop the bot: {error}"),
            Self::Start(error) => write!(f, "update: could not start the bot: {error}"),
            Self::Slot(error) => write!(f, "update: {error}"),
            Self::Health(error) => write!(f, "update: {error}"),
        }
    }
}

impl std::error::Error for WindowError {}

impl From<SlotError> for WindowError {
    fn from(error: SlotError) -> Self {
        Self::Slot(error)
    }
}

impl From<HealthError> for WindowError {
    fn from(error: HealthError) -> Self {
        Self::Health(error)
    }
}

/// What happened after a build was made `current`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The build `current` was rolled back to, or `None` when the new one was kept.
    pub rolled_back_to: Option<std::path::PathBuf>,
}

/// Puts a verified `build` in `current`: asks the bot to stop, waits for it, asks systemd to stop
/// it, swaps, and asks systemd to start it again.
///
/// The build is kept only if the bot comes up at all: `restarted` says whether it did, and a build
/// systemd cannot start is a build that is not surviving, so `current` goes back to the one that
/// was running and the outcome says so. Whether the build that *did* come up is kept is [`watch`]'s
/// question, asked once it has had its window to fail in.
///
/// Nothing is swapped until systemd has stopped the bot: a bot that is still running under systemd
/// would be started again from the new build while its own process still holds the table, so a stop
/// that systemd refuses ends the window with the slots untouched.
///
/// `now_ms` is the clock, and `window_ms` how long the bot is given to stop and to come up: the
/// deadline is their sum, so the caller owns the time and this module never reads it (ADR 0003).
pub fn apply(
    world: &mut dyn Window,
    slots: &Slots,
    health: &Health,
    build: impl AsRef<std::path::Path>,
    now_ms: i64,
    window_ms: i64,
) -> Result<Outcome, WindowError> {
    // A request left over from an update that did not finish would stop the bot the moment it
    // starts; clearing it first makes the request that matters the only one standing.
    health.clear_stop()?;
    health.request_update()?;
    let deadline = now_ms.saturating_add(window_ms);
    if !world.bot_stopped(deadline) {
        // The request is lifted, so a bot that reads it a moment late does not stop for an update
        // that is no longer happening.
        health.clear_update()?;
        return Err(WindowError::BotNeverStopped {
            deadline_ms: deadline,
        });
    }
    // The bot has left the table, but its process is still up and systemd would still start *it*
    // from the old build. Retire it before anything moves, so the start below is the new build's
    // and only the new build's. A stop systemd refuses ends the window here, with the slots
    // untouched: a request is easier to repeat than a swap under a running process is to undo.
    if let Err(error) = world.stop() {
        health.clear_update()?;
        return Err(WindowError::Stop(error));
    }
    // The bot is down from here, and every path out of the swap must leave a build running again.
    let swapped = swap_and_start(world, slots, build, deadline);
    health.clear_update()?;
    swapped
}

/// Takes the bot from stopped to running the build `current` holds, and reports what it settled on.
///
/// The caller has already stopped the bot. Three things can go wrong, and each one ends with a
/// build running: the install, which leaves the old build in `current` untouched; the start, which
/// rolls back to that old build; and the bot not coming up, which rolls back the same way. There is
/// nothing running only if the *last* fallback, starting again after a rollback, fails too.
fn swap_and_start(
    world: &mut dyn Window,
    slots: &Slots,
    build: impl AsRef<std::path::Path>,
    deadline_ms: i64,
) -> Result<Outcome, WindowError> {
    if let Err(error) = slots.install(build) {
        // Nothing moved: `current` still holds the build that was running, so starting it again
        // restores the fleet. The install's failure is the one worth reporting.
        let _ = world.start();
        return Err(error.into());
    }
    if let Err(error) = world.start() {
        // systemd would not start the build. Put the one that was running back and start that.
        roll_back(slots)?;
        let _ = world.start();
        return Err(WindowError::Start(error));
    }
    if !world.restarted(deadline_ms) {
        // systemd started something, but the bot never came up: the same judgement as a crash
        // loop, made sooner. The old build goes back and is started.
        let rolled_back_to = roll_back(slots)?;
        let _ = world.start();
        return Ok(Outcome { rolled_back_to });
    }
    Ok(Outcome {
        rolled_back_to: None,
    })
}

/// Whether the build `current` holds may be kept, once the bot has had its window.
///
/// `limit` starts inside `window_ms` is a crash loop: the build is not surviving, so `current`
/// goes back to the previous build. Nothing is rolled back when there is nothing to go back to.
pub fn watch(
    slots: &Slots,
    health: &Health,
    now_ms: i64,
    window_ms: i64,
    limit: usize,
) -> Result<Outcome, WindowError> {
    if health.in_crash_loop(now_ms, window_ms, limit)? {
        return Ok(Outcome {
            rolled_back_to: roll_back(slots)?,
        });
    }
    Ok(Outcome {
        rolled_back_to: None,
    })
}

/// Rolls back when the harm guard trips: the canary's live evidence says the new build is worse
/// than the one it replaced. Returns the build `current` went back to.
pub fn roll_back_for_harm(slots: &Slots) -> Result<Option<std::path::PathBuf>, WindowError> {
    roll_back(slots)
}

/// Points `current` back at `previous`, or reports that there is nothing to go back to.
fn roll_back(slots: &Slots) -> Result<Option<std::path::PathBuf>, WindowError> {
    if slots.previous().is_none() {
        return Ok(None);
    }
    slots.roll_back()?;
    Ok(slots.current())
}

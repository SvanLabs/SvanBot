//! The update window: what happens between a verified release and the build the fleet runs.
//!
//! The order is fixed and never guessed:
//!
//! 1. Ask the bot to stop between hands, and wait for it to say it has. A bot with a hand in
//!    flight keeps playing; the window does not cut in, and a bot that will not stop is left alone
//!    and the window is repeated rather than forced.
//! 2. Ask the supervisor to stop the bot, retiring the process that just left the table.
//! 3. Point `current` at the new build, keeping the running one in `previous` ([`crate::slot`]).
//! 4. Ask the supervisor to start the bot again, and watch its start log ([`crate::health`]).
//! 5. Keep the new build, or roll back to the old one.
//!
//! A bot that the supervisor has **down** (stopped or failed) skips the first two steps: nothing is
//! playing, so there is no hand to wait out and no process to retire. That is the **first start**
//! on a fresh box, where `current` has never been set, and a bot the operator stopped. Its window
//! points `current` and starts the bot. If the start fails, the bot is left down, as the window
//! found it, with nothing started in its place and `current` back on the build it held. On a
//! fresh box it held none, so it keeps the new build for the next try.
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

/// What the window needs done in the world, which is the supervisor's job and the caller's to carry
/// out.
///
/// `stop` and `start` act; the two polls wait. Each wait gets the deadline, in milliseconds since
/// the Unix epoch, past which the answer is no; the caller polls and returns as soon as it has one,
/// so a wait is the caller's loop and not this module's.
pub trait Window {
    /// Asks the supervisor to stop the bot.
    fn stop(&mut self) -> io::Result<()>;

    /// Whether the bot has said it stopped, by `deadline_ms`.
    fn bot_stopped(&mut self, deadline_ms: i64) -> bool;

    /// Asks the supervisor to start the bot, from the build under `current`.
    fn start(&mut self) -> io::Result<()>;

    /// Whether the supervisor has the bot running, by `deadline_ms`.
    fn restarted(&mut self, deadline_ms: i64) -> bool;

    /// Whether the build that went live at `since_ms` has come up `limit` times by `deadline_ms` —
    /// a crash loop.
    ///
    /// Polled until it has, or until the deadline passes, and counted over `[since_ms, now]`:
    /// a crash loop is not visible the moment a build comes up, so it has to be watched for, and
    /// the count has to start when the build did or the build it replaced is counted as this one.
    fn crash_looped(&mut self, since_ms: i64, limit: usize, deadline_ms: i64) -> bool;

    /// Whether the supervisor has the bot down right now: stopped or failed, with no process of it
    /// running. `false` when the supervisor cannot say, because a bot that might be playing is the
    /// one the window must wait for.
    fn down(&mut self) -> bool;
}

/// Why an update did not finish.
#[derive(Debug)]
pub enum WindowError {
    /// The bot never said it stopped: a hand in flight, or a bot wedged mid-turn.
    BotNeverStopped { deadline_ms: i64 },
    /// The supervisor could not stop the bot, so nothing was swapped under a running process.
    Stop(io::Error),
    /// The supervisor could not start the bot again.
    Start(io::Error),
    /// A bot that was down was started on the new build, and the supervisor never had it running.
    NeverCameUp { deadline_ms: i64 },
    /// The build crash-loops and there is no build in `previous` to roll back to.
    CrashLoop,
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
            Self::NeverCameUp { deadline_ms } => write!(
                f,
                "update: the bot was down and did not come up on the new build before \
                 {deadline_ms}; it is stopped again, and `current` is back on the build it held, \
                 if it held one"
            ),
            Self::CrashLoop => write!(
                f,
                "update: the build crash-loops and there is no previous build to roll back to; \
                 the supervisor stops retrying it after five starts in five minutes"
            ),
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

/// Puts a verified `build` in `current`: asks the bot to stop, waits for it, asks the supervisor to
/// stop it, swaps, and asks the supervisor to start it again.
///
/// The build is kept only if the bot comes up at all: `restarted` says whether it did, and a build
/// the supervisor cannot start is a build that is not surviving, so `current` goes back to the one
/// that was running and the outcome says so. Whether the build that *did* come up is kept is
/// [`watch`]'s question, asked once it has had its window to fail in.
///
/// Nothing is swapped until the supervisor has stopped the bot: a bot that is still running under
/// the supervisor would be started again from the new build while its own process still holds the
/// table, so a stop that the supervisor refuses ends the window with the slots untouched.
///
/// A bot the supervisor has down is never asked to stop: there is no bot to answer, and waiting for
/// one would spend the whole window and then report that it never stopped. Its window starts it on
/// the new build instead (see the module's first start).
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
    let deadline = now_ms.saturating_add(window_ms);
    if world.down() {
        // Nothing is playing, so there is no hand to wait out and no process to retire.
        health.clear_update()?;
        return start_from_down(world, slots, build, deadline);
    }
    health.request_update()?;
    if !world.bot_stopped(deadline) {
        // The request is lifted, so a bot that reads it a moment late does not stop for an update
        // that is no longer happening.
        health.clear_update()?;
        return Err(WindowError::BotNeverStopped {
            deadline_ms: deadline,
        });
    }
    // The bot has left the table, but its process is still up and the supervisor would still start
    // *it* from the old build. Retire it before anything moves, so the start below is the new
    // build's and only the new build's. A stop the supervisor refuses ends the window here, with
    // the slots untouched: a request is easier to repeat than a swap under a running process is to
    // undo.
    if let Err(error) = world.stop() {
        health.clear_update()?;
        return Err(WindowError::Stop(error));
    }
    // The bot is down from here, and every path out of the swap must leave a build running again.
    let swapped = swap_and_start(world, slots, build, deadline);
    health.clear_update()?;
    swapped
}

/// Starts a bot that the supervisor has down on `build`: the first start on a fresh box, or a bot
/// the operator stopped.
///
/// Every way it can fail leaves the bot as it was found, down, and starts nothing in its place,
/// because nothing was running to be restored. `current` goes back to the build it held, which the
/// rest of the fleet execs on its next start; on a fresh box it held none, and keeps the new build
/// for the next try.
fn start_from_down(
    world: &mut dyn Window,
    slots: &Slots,
    build: impl AsRef<std::path::Path>,
    deadline_ms: i64,
) -> Result<Outcome, WindowError> {
    let before = slots.current();
    // An install that fails has moved nothing and started nothing.
    slots.install(build)?;
    if let Err(error) = world.start() {
        put_back(slots, before)?;
        return Err(WindowError::Start(error));
    }
    if !world.restarted(deadline_ms) {
        // The supervisor took the start, so its own restarts would keep trying the build. Stop it
        // first, or a restart could land on whichever build `current` names next.
        let _ = world.stop();
        put_back(slots, before)?;
        return Err(WindowError::NeverCameUp { deadline_ms });
    }
    Ok(Outcome {
        rolled_back_to: None,
    })
}

/// Points `current` back at `before`, the build it held when the window opened, if it held one
/// and the install moved it.
fn put_back(slots: &Slots, before: Option<std::path::PathBuf>) -> Result<(), WindowError> {
    if before.is_some() && before != slots.current() {
        slots.roll_back()?;
    }
    Ok(())
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
        // The supervisor would not start the build. Put the one that was running back and start
        // that.
        roll_back(slots)?;
        let _ = world.start();
        return Err(WindowError::Start(error));
    }
    if !world.restarted(deadline_ms) {
        // The supervisor started something, but the bot never came up: the same judgement as a
        // crash loop, made sooner. The old build goes back and is started.
        let rolled_back_to = roll_back(slots)?;
        let _ = world.start();
        return Ok(Outcome { rolled_back_to });
    }
    Ok(Outcome {
        rolled_back_to: None,
    })
}

/// Whether the build `current` holds may be kept, once it has had its window to fail in.
///
/// The window is watched for its whole length, not read once at the start: the build has only just
/// come up, and a crash loop needs time to show. `limit` starts inside it is a crash loop, and the
/// build is not surviving: the supervisor is sitting there restarting it, so this stops it, points
/// `current` back at the build it replaced, and starts that. Nothing is rolled back when there is
/// nothing to go back to, as after a first start: the build stays and its own restart is the one
/// that runs, but the window reports [`WindowError::CrashLoop`] rather than a build that is kept.
pub fn watch(
    world: &mut dyn Window,
    slots: &Slots,
    now_ms: i64,
    window_ms: i64,
    limit: usize,
) -> Result<Outcome, WindowError> {
    let deadline = now_ms.saturating_add(window_ms);
    if !world.crash_looped(now_ms, limit, deadline) {
        return Ok(Outcome {
            rolled_back_to: None,
        });
    }
    if slots.previous().is_none() {
        // A first build has nothing to go back to. It stays, and the supervisor's own restarts are
        // the ones that run, but the window says it loops rather than that it is running.
        return Err(WindowError::CrashLoop);
    }
    // Stop the loop before the swap, or the supervisor restarts the build being retired the moment
    // it starts the one the rollback lands on. A stop it refuses leaves the build as it was.
    world.stop().map_err(WindowError::Stop)?;
    let rolled_back_to = roll_back(slots)?;
    world.start().map_err(WindowError::Start)?;
    Ok(Outcome { rolled_back_to })
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

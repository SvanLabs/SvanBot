//! The update window: what the updater does between a verified release and the build the fleet
//! runs, and when it takes the new build back out.
//!
//! The window has a fixed order and never guesses. It asks the bot to stop, and waits for the bot
//! to say it has; it asks the supervisor to stop the process that left the table; it installs the new
//! build and points `current` at it; it asks the supervisor to start the bot again. The build is kept only
//! if the bot comes up and stays up; if it never comes up, crash-loops, or the canary's harm guard
//! trips first, the slot goes back to the previous build — and a build always comes back up:
//! whatever fails, the window leaves the slot it settled on running.
//!
//! Nothing here sleeps: every wait is a short poll against a world the caller supplies, and the
//! clock is passed in, so a test drives both the crash and the deadline without waiting.
// A test update is a set of files in a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use svanbot_runtime::health::{Health, WINDOW_MS};
use svanbot_runtime::slot::Slots;
use svanbot_runtime::update::{Outcome, Window, WindowError};

/// A fresh, empty directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-update-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// An installed build: its own directory with a program in it.
fn build(root: &Path, version: &str) -> PathBuf {
    let dir = root.join(version);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("svanbot"), format!("{version}\n")).unwrap();
    fs::canonicalize(dir).unwrap()
}

fn health_for(root: &Path, name: &str) -> Health {
    Health::in_dir(root.join("health"), name)
}

/// A world the test scripts: what the supervisor answers, whether the bot comes back up, and a log of
/// every call the window made, in order.
struct Scripted {
    /// What `svanbot-bot stop` answers.
    stop_ok: bool,
    /// What `svanbot-bot start` answers.
    start_ok: bool,
    /// What the start poll finds at its deadline.
    up: bool,
    /// What the stop poll finds at its deadline.
    stopped: bool,
    /// What the crash-loop watch finds by its deadline.
    looped: bool,
    /// Whether the supervisor has the bot down when the window opens: stopped or failed.
    down: bool,
    /// Every call the window made: `"stop"`, `"stop_poll"`, `"start"`, `"up_poll"`, `"loop_watch"`.
    calls: Rc<RefCell<Vec<&'static str>>>,
}

impl Scripted {
    /// A world that concedes every wait and answers every supervisor call, the happy path.
    fn ready() -> Self {
        Self {
            stop_ok: true,
            start_ok: true,
            up: true,
            stopped: true,
            looped: false,
            down: false,
            calls: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// A world where the supervisor has the bot down: a fresh box, or a bot the operator stopped. Nothing
    /// is playing, so no bot is there to say it stopped.
    fn down() -> Self {
        Self {
            down: true,
            stopped: false,
            ..Self::ready()
        }
    }

    /// A world where the build crashes and comes back up: the watch sees a crash loop.
    fn crash_loops() -> Self {
        Self {
            looped: true,
            ..Self::ready()
        }
    }

    /// A world whose supervisor refuses `stop`.
    fn refuses_to_stop() -> Self {
        Self {
            stop_ok: false,
            ..Self::ready()
        }
    }

    /// A world whose supervisor refuses `start`.
    fn refuses_to_start() -> Self {
        Self {
            start_ok: false,
            ..Self::ready()
        }
    }

    /// A world where the supervisor starts the bot but it never comes up.
    fn never_comes_up() -> Self {
        Self {
            up: false,
            ..Self::ready()
        }
    }

    /// A world where the bot never says it stopped.
    fn bot_never_stops() -> Self {
        Self {
            stopped: false,
            ..Self::ready()
        }
    }

    fn log(&self) -> Vec<&'static str> {
        self.calls.borrow().clone()
    }

    fn note(&self, what: &'static str) {
        self.calls.borrow_mut().push(what);
    }
}

impl Window for Scripted {
    fn stop(&mut self) -> io::Result<()> {
        self.note("stop");
        if self.stop_ok {
            Ok(())
        } else {
            Err(io::Error::other("svanbot-bot: First has no configuration"))
        }
    }

    fn bot_stopped(&mut self, _deadline_ms: i64) -> bool {
        self.note("stop_poll");
        self.stopped
    }

    fn start(&mut self) -> io::Result<()> {
        self.note("start");
        if self.start_ok {
            Ok(())
        } else {
            Err(io::Error::other("svanbot-bot: First did not start"))
        }
    }

    fn restarted(&mut self, _deadline_ms: i64) -> bool {
        self.note("up_poll");
        self.up
    }

    fn crash_looped(&mut self, _since_ms: i64, _limit: usize, _deadline_ms: i64) -> bool {
        self.note("loop_watch");
        self.looped
    }

    fn down(&mut self) -> bool {
        self.down
    }
}

#[test]
fn a_fresh_box_points_current_at_the_build_and_starts_the_bot_without_a_stop_wait() {
    let root = scratch("fresh");
    let slots = Slots::in_dir(root.join("lib"));
    let health = health_for(&root, "SurSvan");
    let new = build(&root, "11.0.0");
    // Nothing has ever run here: no `current`, and the supervisor has the bot stopped. There is no bot
    // to say it stopped, so a window that waits for one waits out its whole deadline for nothing.
    let mut world = Scripted::down();
    let outcome =
        svanbot_runtime::update::apply(&mut world, &slots, &health, &new, 1_000, 60_000).unwrap();
    assert_eq!(
        outcome,
        Outcome {
            rolled_back_to: None
        }
    );
    // No stop wait and no stop: started once, and watched until the supervisor has it running.
    assert_eq!(world.log(), ["start", "up_poll"]);
    assert_eq!(slots.current(), Some(new));
    assert_eq!(slots.previous(), None);
    // Nothing is left asking the bot that just came up to stop.
    assert!(!health.update_requested());
    assert!(!health.stopped());
}

#[test]
fn a_stopped_bot_whose_new_build_never_comes_up_is_left_stopped_on_the_old_build() {
    let root = scratch("downnever");
    let slots = Slots::in_dir(root.join("lib"));
    let health = health_for(&root, "SurSvan");
    let old = build(&root, "11.0.0");
    let new = build(&root, "11.0.1");
    slots.install(&old).unwrap();
    // The operator stopped the bot, and the new build the supervisor starts never comes up.
    let mut world = Scripted {
        up: false,
        ..Scripted::down()
    };
    let error = svanbot_runtime::update::apply(&mut world, &slots, &health, &new, 1_000, 60_000)
        .unwrap_err();
    assert!(
        matches!(error, WindowError::NeverCameUp { .. }),
        "{error:?}"
    );
    // The window leaves the bot as it found it: stopped, so the supervisor's own restarts do not keep
    // retrying the build, and `current` back on the build the rest of the fleet runs from.
    assert_eq!(world.log(), ["start", "up_poll", "stop"]);
    assert_eq!(slots.current(), Some(old));
    assert_eq!(slots.previous(), Some(new));
}

#[test]
fn a_stopped_bot_the_supervisor_will_not_start_keeps_the_old_build_in_current() {
    let root = scratch("downrefused");
    let slots = Slots::in_dir(root.join("lib"));
    let health = health_for(&root, "SurSvan");
    let old = build(&root, "11.0.0");
    let new = build(&root, "11.0.1");
    slots.install(&old).unwrap();
    let mut world = Scripted {
        start_ok: false,
        ..Scripted::down()
    };
    let error = svanbot_runtime::update::apply(&mut world, &slots, &health, &new, 1_000, 60_000)
        .unwrap_err();
    assert!(matches!(error, WindowError::Start(_)), "{error:?}");
    // Nothing was running, so nothing is started in its place: the bot stays down, and `current`
    // goes back to the build it held.
    assert_eq!(world.log(), ["start"]);
    assert_eq!(slots.current(), Some(old));
}

#[test]
fn an_update_stops_the_bot_installs_the_release_and_starts_it_again() {
    let root = scratch("happy");
    let slots = Slots::in_dir(root.join("lib"));
    let health = health_for(&root, "SurSvan");
    let old = build(&root, "2026.11.16-1");
    let new = build(&root, "2026.11.16-2");
    slots.install(&old).unwrap();
    health.clear_stop().unwrap();
    let mut world = Scripted::ready();
    let outcome =
        svanbot_runtime::update::apply(&mut world, &slots, &health, &new, 1_000, 60_000).unwrap();
    // The build came up and nothing is being rolled back: whether it is *kept* is `watch`'s
    // question, asked once it has had its window.
    assert_eq!(
        outcome,
        Outcome {
            rolled_back_to: None
        }
    );
    // Asked to stop, asked to start once: the swap happened while the bot was down.
    assert_eq!(world.log(), ["stop_poll", "stop", "start", "up_poll"]);
    assert_eq!(slots.current(), Some(new));
    assert_eq!(slots.previous(), Some(old));
    // The request is lifted and the stop cleared, so the bot that just came up is not asked again.
    assert!(!health.update_requested());
    assert!(!health.stopped());
}

#[test]
fn a_bot_that_does_not_stop_inside_the_deadline_is_never_swapped_under() {
    let root = scratch("nosstop");
    let slots = Slots::in_dir(root.join("lib"));
    let health = health_for(&root, "SurSVan");
    let old = build(&root, "2026.11.16-1");
    let new = build(&root, "2026.11.16-2");
    slots.install(&old).unwrap();
    // A world where the bot never says it stopped: a hand in progress, or a bot wedged mid-turn.
    let mut world = Scripted::bot_never_stops();
    let error = svanbot_runtime::update::apply(&mut world, &slots, &health, &new, 1_000, 60_000)
        .unwrap_err();
    assert!(
        matches!(error, WindowError::BotNeverStopped { .. }),
        "{error:?}"
    );
    // The bot was never asked to stop and the supervisor was never touched: there is nothing to stop a
    // bot that is still playing for, and the swap waits for the next window.
    assert_eq!(world.log(), ["stop_poll"]);
    assert_eq!(slots.current(), Some(old));
    assert!(!health.update_requested());
}

#[test]
fn a_build_the_supervisor_cannot_start_is_rolled_back_before_it_can_take_anything_down() {
    let root = scratch("deadonarrival");
    let slots = Slots::in_dir(root.join("lib"));
    let health = health_for(&root, "SurSvan");
    let old = build(&root, "2026.11.16-1");
    let new = build(&root, "2026.11.16-2");
    slots.install(&old).unwrap();
    // The swap happened, but the supervisor never reported the bot back up: the build is not running, so
    // the one that was running goes straight back. This is a different failure from a crash loop:
    // there is not one start to read.
    let mut world = Scripted::never_comes_up();
    let outcome =
        svanbot_runtime::update::apply(&mut world, &slots, &health, &new, 1_000, 60_000).unwrap();
    assert_eq!(outcome.rolled_back_to, Some(old.clone()));
    assert_eq!(slots.current(), Some(old));
    // The old build was started again after the rollback: the fleet is left running.
    assert_eq!(
        world.log(),
        ["stop_poll", "stop", "start", "up_poll", "start"]
    );
}

#[test]
fn a_start_the_supervisor_refuses_rolls_the_slot_back_and_starts_the_old_build() {
    let root = scratch("badstart");
    let slots = Slots::in_dir(root.join("lib"));
    let health = health_for(&root, "SurSVan");
    let old = build(&root, "2026.11.16-1");
    let new = build(&root, "2026.11.16-2");
    slots.install(&old).unwrap();
    let mut world = Scripted::refuses_to_start();
    let error = svanbot_runtime::update::apply(&mut world, &slots, &health, &new, 1_000, 60_000)
        .unwrap_err();
    assert!(matches!(error, WindowError::Start(_)), "{error:?}");
    // The build the supervisor would not start is not left in `current`: the old one is back and started.
    assert_eq!(slots.current(), Some(old));
    assert_eq!(world.log(), ["stop_poll", "stop", "start", "start"]);
    assert!(!health.update_requested());
}

#[test]
fn a_stop_the_supervisor_refuses_leaves_the_slots_where_they_were() {
    let root = scratch("badstop");
    let slots = Slots::in_dir(root.join("lib"));
    let health = health_for(&root, "SurSVan");
    let old = build(&root, "2026.11.16-1");
    let new = build(&root, "2026.11.16-2");
    slots.install(&old).unwrap();
    let mut world = Scripted::refuses_to_stop();
    let error = svanbot_runtime::update::apply(&mut world, &slots, &health, &new, 1_000, 60_000)
        .unwrap_err();
    assert!(matches!(error, WindowError::Stop(_)), "{error:?}");
    // Nothing moved and nothing was started: the supervisor never stopped the bot, so the running build
    // is still the one it is running and the window is repeated later.
    assert_eq!(slots.current(), Some(old));
    assert_eq!(slots.previous(), None);
    assert_eq!(world.log(), ["stop_poll", "stop"]);
    assert!(!health.update_requested());
}

#[test]
fn a_build_that_crash_loops_is_rolled_back_to_the_one_that_was_running() {
    let root = scratch("crash");
    let slots = Slots::in_dir(root.join("lib"));
    let health = health_for(&root, "SurSVan");
    let old = build(&root, "2026.11.16-1");
    let new = build(&root, "2026.11.16-2");
    slots.install(&old).unwrap();
    // The build comes up and `apply` keeps it; the window then watches it, and the watch sees it
    // come up three times inside the window. The world answers "looped" where the real one counts
    // the start log (`in_crash_loop`, tested in `health.rs`); here the window's response is what
    // is under test.
    let mut world = Scripted::crash_loops();
    let outcome =
        svanbot_runtime::update::apply(&mut world, &slots, &health, &new, 0, 60_000).unwrap();
    assert_eq!(outcome.rolled_back_to, None);
    assert_eq!(slots.current(), Some(new.clone()));
    let outcome = svanbot_runtime::update::watch(&mut world, &slots, 0, WINDOW_MS, 3).unwrap();
    assert_eq!(outcome.rolled_back_to, Some(old.clone()));
    assert_eq!(slots.current(), Some(old));
    // The build that crash-looped is now `previous`, so a later rollback is still possible.
    assert_eq!(slots.previous(), Some(new));
    // The loop is stopped before the swap and the build rolled back to is started: the supervisor is
    // restarting the broken build, so a swap under it would race the start of the old one.
    assert_eq!(
        world.log(),
        [
            "stop_poll",
            "stop",
            "start",
            "up_poll",
            "loop_watch",
            "stop",
            "start"
        ]
    );
}

#[test]
fn a_build_that_comes_up_and_stays_up_is_kept() {
    let root = scratch("keep");
    let slots = Slots::in_dir(root.join("lib"));
    let new = build(&root, "2026.11.16-2");
    slots.install(build(&root, "2026.11.16-1")).unwrap();
    slots.install(&new).unwrap();
    // The watch runs its whole window and finds no loop: the build is kept, and nothing is stopped
    // or started, because it is already the one running.
    let mut world = Scripted::ready();
    let outcome = svanbot_runtime::update::watch(&mut world, &slots, 0, WINDOW_MS, 3).unwrap();
    assert_eq!(outcome.rolled_back_to, None);
    assert_eq!(slots.current(), Some(new));
    assert_eq!(world.log(), ["loop_watch"]);
}

#[test]
fn a_first_build_that_crash_loops_is_reported_and_not_kept_as_running() {
    let root = scratch("firstloop");
    let slots = Slots::in_dir(root.join("lib"));
    let first = build(&root, "11.0.0");
    slots.install(&first).unwrap();
    // A fresh box's first build has nothing in `previous`, so there is nothing to roll back to.
    // It stays in `current` and the supervisor's own restarts keep trying it, but a window that says it
    // is running would send the operator away from a bot that is not.
    let mut world = Scripted::crash_loops();
    let error = svanbot_runtime::update::watch(&mut world, &slots, 0, WINDOW_MS, 3).unwrap_err();
    assert!(matches!(error, WindowError::CrashLoop), "{error:?}");
    assert_eq!(world.log(), ["loop_watch"]);
    assert_eq!(slots.current(), Some(first));
}

#[test]
fn the_harm_guard_trips_after_the_swap_and_rolls_back() {
    let root = scratch("harm");
    let slots = Slots::in_dir(root.join("lib"));
    let old = build(&root, "2026.11.16-1");
    let new = build(&root, "2026.11.16-2");
    slots.install(&old).unwrap();
    slots.install(&new).unwrap();
    // The build comes up and stays up, so the watch keeps it.
    let mut world = Scripted::ready();
    let outcome = svanbot_runtime::update::watch(&mut world, &slots, 0, WINDOW_MS, 3).unwrap();
    assert_eq!(outcome.rolled_back_to, None);
    // The guard then trips while the build is running: the same rollback the crash loop gets, done
    // by hand as an operator runs it.
    let tripped = svanbot_runtime::update::roll_back_for_harm(&slots).unwrap();
    assert_eq!(tripped, Some(old.clone()));
    assert_eq!(slots.current(), Some(old));
}

#[test]
fn a_window_with_no_previous_build_can_only_hear_that_there_is_nothing_to_do() {
    let root = scratch("rollbackless");
    let slots = Slots::in_dir(root.join("lib"));
    slots.install(build(&root, "2026.11.16-1")).unwrap();
    assert_eq!(
        svanbot_runtime::update::roll_back_for_harm(&slots).unwrap(),
        None
    );
}

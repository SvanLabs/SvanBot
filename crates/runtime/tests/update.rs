//! The update window: what the updater does between a verified release and the build the fleet
//! runs, and when it takes the new build back out.
//!
//! The window has a fixed order and never guesses. It asks the bot to stop, and waits for the bot
//! to say it has; it asks systemd to stop the process that left the table; it installs the new
//! build and points `current` at it; it asks systemd to start the bot again. The build is kept only
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

const MINUTE_MS: i64 = 60_000;

/// A world the test scripts: what systemd answers, whether the bot comes back up, and a log of
/// every call the window made, in order.
struct Scripted {
    /// What `systemd stop` answers.
    stop_ok: bool,
    /// What `systemd start` answers.
    start_ok: bool,
    /// What the start poll finds at its deadline.
    up: bool,
    /// What the stop poll finds at its deadline.
    stopped: bool,
    /// Every call the window made: `"stop"`, `"stop_poll"`, `"start"`, `"up_poll"`.
    calls: Rc<RefCell<Vec<&'static str>>>,
}

impl Scripted {
    /// A world that concedes every wait and answers every systemd call, the happy path.
    fn ready() -> Self {
        Self {
            stop_ok: true,
            start_ok: true,
            up: true,
            stopped: true,
            calls: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// A world whose systemd refuses `stop`.
    fn refuses_to_stop() -> Self {
        Self {
            stop_ok: false,
            ..Self::ready()
        }
    }

    /// A world whose systemd refuses `start`.
    fn refuses_to_start() -> Self {
        Self {
            start_ok: false,
            ..Self::ready()
        }
    }

    /// A world where systemd starts the bot but it never comes up.
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
            Err(io::Error::other("systemd: unit is not loaded"))
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
            Err(io::Error::other("systemd: unit failed to start"))
        }
    }

    fn restarted(&mut self, _deadline_ms: i64) -> bool {
        self.note("up_poll");
        self.up
    }
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
    // The bot was never asked to stop and systemd was never touched: there is nothing to stop a
    // bot that is still playing for, and the swap waits for the next window.
    assert_eq!(world.log(), ["stop_poll"]);
    assert_eq!(slots.current(), Some(old));
    assert!(!health.update_requested());
}

#[test]
fn a_build_systemd_cannot_start_is_rolled_back_before_it_can_take_anything_down() {
    let root = scratch("deadonarrival");
    let slots = Slots::in_dir(root.join("lib"));
    let health = health_for(&root, "SurSvan");
    let old = build(&root, "2026.11.16-1");
    let new = build(&root, "2026.11.16-2");
    slots.install(&old).unwrap();
    // The swap happened, but systemd never reported the bot back up: the build is not running, so
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
fn a_start_systemd_refuses_rolls_the_slot_back_and_starts_the_old_build() {
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
    // The build systemd would not start is not left in `current`: the old one is back and started.
    assert_eq!(slots.current(), Some(old));
    assert_eq!(world.log(), ["stop_poll", "stop", "start", "start"]);
    assert!(!health.update_requested());
}

#[test]
fn a_stop_systemd_refuses_leaves_the_slots_where_they_were() {
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
    // Nothing moved and nothing was started: systemd never stopped the bot, so the running build
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
    // The world says the bot came back up, and it did. But the start log says it came up three
    // times inside the window: the build is not surviving, and the window reads the log rather
    // than trusting the one start.
    let mut world = Scripted::ready();
    let outcome =
        svanbot_runtime::update::apply(&mut world, &slots, &health, &new, 0, 60_000).unwrap();
    assert_eq!(outcome.rolled_back_to, None);
    assert_eq!(slots.current(), Some(new.clone()));
    // The bot comes up, crashes, and comes up again: three starts inside the window.
    for minute in 0..3 {
        health
            .note_start(minute * MINUTE_MS, "2026.11.16-2")
            .unwrap();
    }
    let outcome =
        svanbot_runtime::update::watch(&slots, &health, 2 * MINUTE_MS, WINDOW_MS, 3).unwrap();
    assert_eq!(outcome.rolled_back_to, Some(old.clone()));
    assert_eq!(slots.current(), Some(old));
    // The build that crash-looped is now `previous`, so a later rollback is still possible.
    assert_eq!(slots.previous(), Some(new));
}

#[test]
fn a_build_that_comes_up_and_stays_up_is_kept() {
    let root = scratch("keep");
    let slots = Slots::in_dir(root.join("lib"));
    let health = health_for(&root, "SurSVan");
    let new = build(&root, "2026.11.16-2");
    slots.install(build(&root, "2026.11.16-1")).unwrap();
    slots.install(&new).unwrap();
    // One start, well inside the window: nothing to roll back for.
    health.note_start(1_000, "2026.11.16-2").unwrap();
    let outcome = svanbot_runtime::update::watch(&slots, &health, 1_000, 60_000, 3).unwrap();
    assert_eq!(outcome.rolled_back_to, None);
    assert_eq!(slots.current(), Some(new));
}

#[test]
fn the_harm_guard_trips_after_the_swap_and_rolls_back() {
    let root = scratch("harm");
    let slots = Slots::in_dir(root.join("lib"));
    let health = health_for(&root, "SurSVan");
    let old = build(&root, "2026.11.16-1");
    let new = build(&root, "2026.11.16-2");
    slots.install(&old).unwrap();
    slots.install(&new).unwrap();
    let outcome = svanbot_runtime::update::watch(&slots, &health, 1_000, 60_000, 3).unwrap();
    assert_eq!(outcome.rolled_back_to, None);
    // The guard trips while the build is running: the same rollback the crash loop gets.
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

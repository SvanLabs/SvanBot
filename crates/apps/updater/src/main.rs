//! `svanbot-release`: puts a verified release where the fleet can run it, canary first.
//!
//! Four subcommands, each one step of an update that a person or CI can run and read:
//!
//! - `status`: what is installed and what is running — the [`status`] panel that replaces v10's
//!   `verify-install` skill and its `rollback.sh`.
//! - `install <dir>`: verifies a staged release ([`svanbot_runtime::release`]) and promotes it into
//!   `lib/<version>` ([`promote`]), without touching the slot the fleet runs. Both of those are
//!   read-only with respect to `current`, so a release can be staged long before a window is found.
//! - `update [<version>]`: swaps `current` to an installed build between hands, canary first, and
//!   rolls back if it does not come up ([`svanbot_runtime::update`]). The version defaults to the
//!   one `install` last promoted.
//! - `rollback`: points `current` back at `previous`, for the harm guard and for an operator.
//!
//! The canary rule lives here, in the app, because it is about who is on the box, not about poker:
//! a build is swapped into `current` for the canary's slot and then, once it has been watched, for
//! the rest. `SVANBOT_CANARY` names the canary (`docs/research/2026-11/04-learner-experiments.md`);
//! `--fleet` is the operator saying the canary passed its window.
//!
//! An app does I/O, so it turns off the `clippy.toml` bans here, once (ADR 0003).
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

mod lock;
mod promote;
mod status;
mod supervisor;
mod world;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use svanbot_runtime::health::Health;
use svanbot_runtime::release::{self, Release};
use svanbot_runtime::slot::Slots;
use svanbot_runtime::update;

use crate::lock::Lock;
use crate::supervisor::Supervisor;
use crate::world::{BotWorld, now_ms};

/// How long the bot is given to stop between hands and to come back up, in the plainest possible
/// terms. A window is a gap between hands, not a season: a bot that has not come up in two minutes
/// is not going to, and the action clock it is waiting on is 45 s.
const WINDOW_MS: i64 = 2 * 60 * 1_000;

/// The starts inside `health::WINDOW_MS` that mean a crash loop: three, the number the ticket and
/// the glossary name, which is inside the supervisor's own five so the updater sees it first.
const CRASH_STARTS: usize = 3;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("svanbot-release: {why}");
            ExitCode::from(1)
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let subcommand = args.first().map(String::as_str).unwrap_or("status");
    match subcommand {
        "status" => status::print(&tree()?),
        "install" => install(positional(args, &[])?),
        "update" => {
            let fleet = args.iter().any(|arg| arg == "--fleet");
            update_now(positional(args, &["--fleet"])?, fleet)
        }
        "rollback" => rollback(),
        "help" | "-h" | "--help" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown subcommand `{other}`\n\n{USAGE}")),
    }
}

/// The one argument that is not a flag, for the subcommands that take one.
///
/// A flag is never a version: `update --fleet` names no build, and reading `--fleet` as the version
/// made the documented fleet rollout fail with "`--fleet` is not installed". Anything starting with
/// `-` that `known` does not name is refused by name rather than taken as a positional, so a
/// mistyped flag is answered as a typo and not as a missing build.
fn positional<'a>(args: &'a [String], known: &[&str]) -> Result<Option<&'a str>, String> {
    let mut positional = None;
    for arg in args.iter().skip(1) {
        if arg.starts_with('-') {
            if !known.contains(&arg.as_str()) {
                return Err(format!("unknown flag `{arg}`\n\n{USAGE}"));
            }
            continue;
        }
        if positional.replace(arg.as_str()).is_some() {
            return Err(format!("unexpected extra argument `{arg}`\n\n{USAGE}"));
        }
    }
    Ok(positional)
}

const USAGE: &str = "\
usage: svanbot-release <status|install|update|rollback>

  status             what is installed and what is running
  install <dir>      verify a staged release and install it into lib/<version>
  update [<version>] swap current to that build between hands (SVANBOT_CANARY unless --fleet)
  rollback           point current back at the build it replaced
";

/// The install tree the updater manages: `SVANBOT_LIB`, or `~/.local/lib/svanbot`.
///
/// It is one path for the whole fleet because the builds are shared; what differs per bot is which
/// bot a window is run for, which is `SVANBOT_CANARY` and `SVANBOT_NAME`, not this.
fn tree() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("SVANBOT_LIB") {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").ok_or("set SVANBOT_LIB or HOME for the install tree")?;
    Ok(PathBuf::from(home).join(".local/lib/svanbot"))
}

/// Verifies a staged release in `dir` and copies it into `lib/<version>`. The slot the fleet runs
/// is not touched, so this is safe to run at any time, including while the fleet plays.
fn install(dir: Option<&str>) -> Result<(), String> {
    let dir = dir.ok_or("install needs the directory a release was unpacked into")?;
    let staged = Release::at(Path::new(dir)).map_err(|e| format!("{e}"))?;
    staged.verify().map_err(|e| format!("{e}"))?;
    let target = promote::promote(&staged, &tree()?).map_err(|e| format!("{e}"))?;
    println!("svanbot-release: installed {}", version_of(&target));
    Ok(())
}

/// Swaps `current` to an installed build between hands, for the canary first.
fn update_now(version: Option<&str>, fleet: bool) -> Result<(), String> {
    let tree = tree()?;
    let slots = Slots::in_dir(&tree);
    let build = installed(&tree, version)?;
    // The canary takes every update first, and the fleet waits for it to be watched. The operator
    // says the word that means the window is over; there is nothing on the box that can. So the
    // canary is named only when a canary is what runs: `--fleet` is the operator saying that window
    // has passed, and demanding a canary there would refuse the command the usage text offers.
    let bot = if fleet {
        // Which bot is swapped still has to be named, one at a time.
        std::env::var("SVANBOT_NAME").map_err(|_| "set SVANBOT_NAME: --fleet swaps one bot")?
    } else {
        canary()?
    };
    let _lock = Lock::take(&tree)?;
    let health = health_for(&bot)?;
    let supervisor = Supervisor::for_bot(&bot);
    let mut world = BotWorld::new(&supervisor, &health);
    let outcome = update::apply(&mut world, &slots, &health, &build, now_ms(), WINDOW_MS)
        .map_err(|e| format!("{e}"))?;
    match outcome.rolled_back_to {
        // The build did not come up: it is out, and the one that was running is back. Say so
        // loudly, because a release that fails this is a release that is not shipped.
        Some(back) => {
            println!(
                "svanbot-release: {} did not come up on {bot}; rolled back to {}",
                version_of(&build),
                version_of(&back)
            );
            Ok(())
        }
        None => {
            // The swap is done; whether the build stays is `watch`'s question, asked by watching
            // its start log for the whole window. A crash loop rolls back and is restarted there.
            let watched = update::watch(
                &mut world,
                &slots,
                now_ms(),
                svanbot_runtime::health::WINDOW_MS,
                CRASH_STARTS,
            )
            .map_err(|e| format!("{e}"))?;
            match watched.rolled_back_to {
                Some(back) => println!(
                    "svanbot-release: {} crash-looped on {bot}; rolled back to {}",
                    version_of(&build),
                    version_of(&back)
                ),
                None => println!("svanbot-release: {bot} now runs {}", version_of(&build)),
            }
            Ok(())
        }
    }
}

/// Points `current` back at the build it replaced, for the harm guard and for an operator.
fn rollback() -> Result<(), String> {
    let tree = tree()?;
    let _lock = Lock::take(&tree)?;
    let slots = Slots::in_dir(&tree);
    match update::roll_back_for_harm(&slots).map_err(|e| format!("{e}"))? {
        Some(back) => {
            println!("svanbot-release: rolled back to {}", version_of(&back));
            Ok(())
        }
        None => Err("nothing to roll back to: only one build has been installed".into()),
    }
}

/// The directory `deploy/install.sh` stages a bundle under before it is verified, inside the tree
/// so the whole release travels in one path. It is not a build: no window points at it, and no
/// rollback lands on it.
const STAGED: &str = "staged";

/// The build `install` last promoted, or the version named on the command line.
fn installed(tree: &Path, version: Option<&str>) -> Result<PathBuf, String> {
    let dir = tree.join(version.unwrap_or_default());
    if let Some(version) = version {
        return if dir.is_dir() {
            Ok(dir)
        } else {
            Err(format!(
                "{version} is not installed in {}: run `svanbot-release install`",
                tree.display()
            ))
        };
    }
    newest(tree)
}

/// The newest installed build: the one the versions name, not the one a text sort would put last.
///
/// `1.9.0` sorts after `1.10.0` as text, and `2026.11.9` after `2026.11.16`, so a text sort makes a
/// version-less `update` a silent downgrade. The comparison is [`release::version_cmp`]'s.
fn newest(tree: &Path) -> Result<PathBuf, String> {
    let builds = installs(tree);
    let versions = builds.iter().map(|path| version_of(path));
    let newest = release::newest(versions)
        .ok_or_else(|| format!("no build is installed in {}", tree.display()))?;
    Ok(tree.join(newest))
}

/// The build directories in `tree`, in no particular order.
///
/// A slot is a symlink and a staged release is a half-installed bundle, so neither is a version a
/// window could be pointed at: only the directories whose names are plain versions count.
pub(crate) fn installs(tree: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(tree) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        // A slot is a symlink and a staged release is not a build yet: only a real directory
        // named for a version is one a window can be pointed at.
        .filter(|path| path.is_dir() && !path.is_symlink())
        .filter(|path| version_of(path) != STAGED)
        .collect()
}

/// The canary's name: the one bot that takes every update first.
fn canary() -> Result<String, String> {
    std::env::var("SVANBOT_CANARY").map_err(|_| {
        "set SVANBOT_CANARY: the canary takes every update first (or pass --fleet for a bot that \
         already has it)"
            .into()
    })
}

/// The bot's health files, derived from its store the way the bot derives them: a `health`
/// directory beside `~/.local/share/svanbot/<name>.db`.
fn health_for(bot: &str) -> Result<Health, String> {
    let dir = match std::env::var_os("SVANBOT_HEALTH") {
        Some(path) => PathBuf::from(path),
        None => {
            let store = store_path(bot)?;
            svanbot_runtime::health::beside_store(store)
        }
    };
    Ok(Health::in_dir(dir, bot))
}

/// The bot's store path, by the same rule the bot and its supervisor (`scripts/bot.sh`) use.
fn store_path(bot: &str) -> Result<PathBuf, String> {
    match std::env::var_os("SVANBOT_STORE") {
        Some(path) => Ok(PathBuf::from(path)),
        None => {
            let home = std::env::var_os("HOME").ok_or("set SVANBOT_STORE or HOME")?;
            Ok(PathBuf::from(home).join(format!(".local/share/svanbot/{bot}.db")))
        }
    }
}

/// The version a build directory is named for.
///
/// A build lives at `lib/<version>`, so the directory's own name is the version — except at the
/// root of the filesystem, where it has no name, and there is no version there to report.
fn version_of(dir: &Path) -> String {
    dir.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "-".into())
}

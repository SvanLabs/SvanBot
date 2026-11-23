//! `svanbot release-check <tag> [fleet]`: holds a release to the ship bar (V11-22), names every
//! clause it fails, and exits 1 if there is one, 0 if there is none. It reads the fleet's files
//! and the stores, and writes nothing.
//!
//! - `svanbot release-check <tag>`: the Arena stage, before the tag is pushed. The tag, or any
//!   name git has for a commit, is looked up in the clone the command runs in. The newest run of
//!   each league from that commit must be under the fleet's champion, play again as recorded,
//!   beat v10.0.2 heads up, and show no regression (`svanbot_runtime::ship::arena`). The bouts it
//!   plays again are played here, by the same code `svanbot arena` played them with
//!   ([`play_bout`]), so run it from a build of the tag.
//! - `svanbot release-check <tag> fleet`: the Arena stage and then the canary stage, before
//!   `svanbot-release update --fleet`. The canary (`SVANBOT_CANARY`) must have played the release
//!   over 20 000 hands since it first took it, and not lose to the rest of the fleet over the same
//!   window (`svanbot_runtime::ship::canary`). The fleet's side is each other bot's export in its
//!   store, so run `svanbot download` for each one first (`deploy/README.md`).
//!
//! The stores are in `SVANBOT_DATA`, `~/.local/share/svanbot` unless set, one `<name>.db` a bot.

use std::path::PathBuf;
use std::process::{Command, ExitCode};

use svanbot_runtime::fleet::{ArenaRow, ArenaRun, Fleet};
use svanbot_runtime::ship::{self, window};
use svanbot_runtime::store::Store;

use crate::arena::{Opponent, play_bout};
use crate::deciders;

pub(crate) fn run(args: &[String]) -> ExitCode {
    let (tag, fleet_stage) = match args {
        [tag] => (tag.as_str(), false),
        [tag, stage] if stage == "fleet" => (tag.as_str(), true),
        _ => {
            eprintln!("svanbot release-check: usage: svanbot release-check <tag> [fleet]");
            return ExitCode::from(2);
        }
    };
    let (commit, champion, runs) = match read(tag) {
        Ok(read) => read,
        Err(why) => {
            eprintln!("svanbot release-check: {why}");
            return ExitCode::from(2);
        }
    };
    let mut replay = |run: &ArenaRun, row: &ArenaRow, bout: u64| {
        let Some(opponent) = Opponent::named(&row.opponent) else {
            eprintln!(
                "svanbot release-check: no opponent is named {}",
                row.opponent
            );
            return f64::NAN;
        };
        eprintln!(
            "svanbot release-check: plays the {} {} row's bout {bout} again",
            run.league.name(),
            row.opponent
        );
        let game = deciders::game(run.seed);
        play_bout(&game, run.league, opponent, bout, &champion, run.iterations)
            .iter()
            .sum()
    };
    let digest = champion.digest().to_string();
    let mut failed = match ship::arena(&runs, &commit, &digest, &mut replay) {
        Ok(()) => {
            println!("the Arena stage clears {commit} under champion {digest}");
            Vec::new()
        }
        Err(failed) => failed,
    };
    if fleet_stage {
        match canary_stage(tag) {
            Ok(line) => println!("{line}"),
            Err(why) => failed.push(why),
        }
    }
    for why in &failed {
        println!("refused: {why}");
    }
    if failed.is_empty() {
        let stage = if fleet_stage { "the fleet" } else { "the tag" };
        println!("{tag} clears the ship bar for {stage}");
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// The tag's commit, the fleet's champion and the Arena's runs.
fn read(tag: &str) -> Result<(String, svanbot_strategy::params::Params, Vec<ArenaRun>), String> {
    let output = Command::new("git")
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{tag}^{{commit}}"),
        ])
        .output()
        .map_err(|e| format!("git did not run: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git has no commit named {tag}: run it in a clone that has the tag"
        ));
    }
    let commit = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let fleet = Fleet::in_dir(crate::fleet_dir());
    let champion = fleet
        .champion()
        .map_err(|why| format!("the champion file is refused: {why}"))?;
    let runs = fleet
        .arena_runs()
        .map_err(|why| format!("the arena file is refused: {why}"))?;
    Ok((commit, champion, runs))
}

/// The canary stage: the canary's hands since it took the release against the rest of the
/// fleet's export hands over the same window.
fn canary_stage(tag: &str) -> Result<String, String> {
    let version = ship::version(tag).ok_or_else(|| {
        format!("{tag} names no release: the canary stage needs a tag like v11.0.0")
    })?;
    let canary = std::env::var("SVANBOT_CANARY")
        .map_err(|_| "the canary stage needs SVANBOT_CANARY, the canary's name".to_owned())?;
    let data = data_dir()?;
    let open = |name: &str| {
        Store::open_reader(&data.join(format!("{name}.db")))
            .map_err(|why| format!("{name}'s store is refused: {why}"))
    };
    let ours = open(&canary)?;
    let from = ours
        .released(version)
        .map_err(|why| format!("{canary}'s store is refused: {why}"))?
        .ok_or_else(|| format!("{canary} has not taken {version}"))?;
    let played = ours
        .results_since(from)
        .map_err(|why| format!("{canary}'s store is refused: {why}"))?;
    let until = played.last().map_or(from, |&(at, _)| at);
    let results: Vec<f64> = played.iter().map(|&(_, result)| result).collect();
    let mut fleet = Vec::new();
    for name in stablemates(&data, &canary)? {
        let lines = open(&name)?
            .export_lines()
            .map_err(|why| format!("{name}'s store is refused: {why}"))?;
        let side = window::fleet(&lines, from, until);
        eprintln!(
            "svanbot release-check: {name}: {} export hands in the window, {} unread",
            side.results.len(),
            side.unread
        );
        fleet.extend(side.results);
    }
    ship::canary(&results, &fleet)
}

/// Every bot with a store in `data` but the canary, by name.
fn stablemates(data: &PathBuf, canary: &str) -> Result<Vec<String>, String> {
    let entries =
        std::fs::read_dir(data).map_err(|e| format!("{} is not readable: {e}", data.display()))?;
    let mut names: Vec<String> = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_stem()?.to_str()?.to_owned();
            (path.extension()? == "db" && name != canary).then_some(name)
        })
        .collect();
    names.sort();
    Ok(names)
}

/// The stores' directory: `SVANBOT_DATA`, or `~/.local/share/svanbot`.
fn data_dir() -> Result<PathBuf, String> {
    std::env::var_os("SVANBOT_DATA")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share/svanbot"))
        })
        .ok_or_else(|| "set SVANBOT_DATA or HOME for the stores".to_owned())
}

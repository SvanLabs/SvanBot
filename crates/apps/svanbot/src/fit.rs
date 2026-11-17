//! `svanbot fit`: fit the action model on the hands in the bot's store, then exit.
//!
//! It reads `SVANBOT_NAME` and `SVANBOT_STORE` like the bot. Every opponent's side of every
//! stored hand goes into the fit, live hands and downloaded history alike. The last fifth of the
//! hands is held out, and the fit is kept only if it passes the gate on them against the model in
//! use: the one fitted before, or the baseline. A kept model is written beside the store as
//! `<name>.model`, and the bot reads it when it starts. Reads are per bot (ADR 0004): a bot fits
//! on its own store alone.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use svanbot_learn::fit::{MARGIN, Prepared, fit, gate, prepare};
use svanbot_runtime::history::observed;
use svanbot_strategy::action::ActionModel;

use crate::{open_store, store_path};

/// Rounds of EM a fit runs.
const ROUNDS: usize = 25;

/// The Dirichlet prior on each move's count.
const ALPHA: f64 = 1.0;

/// Held-out hands below which no fit is tried: the gate could not tell.
const MIN_HELD_OUT: usize = 200;

/// Where a bot's fitted action model is kept: beside its store.
pub(crate) fn model_path(me: &str) -> Result<PathBuf, String> {
    Ok(store_path(me)?.with_extension("model"))
}

/// The bot's fitted action model, if one was kept and reads whole. A half-written or unreadable
/// file is no model at all: the bot falls back to the baseline rather than guess.
pub(crate) fn load_model(me: &str) -> Option<ActionModel> {
    read_model(&model_path(me).ok()?)
}

fn read_model(path: &Path) -> Option<ActionModel> {
    ActionModel::from_text(&std::fs::read_to_string(path).ok()?)
}

pub(crate) fn run() -> ExitCode {
    let Ok(me) = std::env::var("SVANBOT_NAME") else {
        eprintln!("svanbot fit: set SVANBOT_NAME");
        return ExitCode::from(2);
    };
    let hands = match open_store(&me).and_then(|store| store.hands().map_err(|e| e.to_string())) {
        Ok(hands) => hands,
        Err(why) => {
            eprintln!("svanbot fit: {why}");
            return ExitCode::from(2);
        }
    };
    let cut = hands.len() * 4 / 5;
    if hands.len() - cut < MIN_HELD_OUT {
        println!(
            "svanbot fit: {} hands in the store, too few to fit and gate (needs {})",
            hands.len(),
            MIN_HELD_OUT * 5
        );
        return ExitCode::SUCCESS;
    }
    let ready = |range: std::ops::Range<usize>| -> Vec<Prepared> {
        range
            .flat_map(|number| observed(&hands[number], number as u64))
            .map(|side| prepare(&side))
            .collect()
    };
    let (train, held_out) = (ready(0..cut), ready(cut..hands.len()));
    let fitted = fit(&train, ROUNDS, ALPHA);
    let incumbent = load_model(&me).unwrap_or_else(ActionModel::baseline);
    let verdict = gate(&fitted.model, &incumbent, &held_out, MARGIN);
    println!(
        "svanbot fit: {} hands, {} held out: gain {:+.4} nats a hand, lower bound {:+.4}, {}",
        hands.len(),
        verdict.hands,
        verdict.gain,
        verdict.lower,
        if verdict.passed {
            "passed the gate"
        } else {
            "did not pass the gate"
        }
    );
    if !verdict.passed {
        return ExitCode::SUCCESS;
    }
    match model_path(&me).and_then(|path| write_whole(&path, &fitted.model.to_text())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("svanbot fit: the model was not kept: {why}");
            ExitCode::from(1)
        }
    }
}

/// Writes `text` to `path` all or not at all: to a file beside it, then renamed over it, so a
/// bot reading the model while a fit runs sees the old one whole or the new one whole.
fn write_whole(path: &Path, text: &str) -> Result<(), String> {
    let partial = path.with_extension("model.partial");
    std::fs::write(&partial, text)
        .and_then(|()| std::fs::rename(&partial, path))
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("svanbot-fit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_kept_model_reads_back_as_it_was_written_and_leaves_no_partial() {
        let dir = scratch("kept");
        let path = dir.join("Alpha.model");
        let text = ActionModel::baseline().to_text();
        write_whole(&path, &text).unwrap();
        assert_eq!(read_model(&path), Some(ActionModel::baseline()));
        // The temporary file is renamed over the target, not left beside it.
        assert!(!dir.join("Alpha.model.partial").exists());
    }

    #[test]
    fn no_model_or_a_broken_one_is_none_rather_than_a_guess() {
        let dir = scratch("broken");
        assert_eq!(read_model(&dir.join("missing.model")), None);
        let path = dir.join("broken.model");
        std::fs::write(&path, "not a model").unwrap();
        assert_eq!(read_model(&path), None);
    }
}

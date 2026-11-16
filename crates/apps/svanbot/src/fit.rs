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

/// The bot's fitted action model, if one was kept and reads whole.
pub(crate) fn load_model(me: &str) -> Option<ActionModel> {
    let text = std::fs::read_to_string(model_path(me).ok()?).ok()?;
    ActionModel::from_text(&text)
}

pub(crate) fn run() -> ExitCode {
    let Ok(me) = std::env::var("SVANBOT_NAME") else {
        eprintln!("svanbot fit: set SVANBOT_NAME");
        return ExitCode::from(2);
    };
    let hands =
        match open_store(&me).and_then(|(_, store)| store.hands().map_err(|e| e.to_string())) {
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

/// Writes `text` to `path` all or not at all: to a file beside it, then renamed over it.
fn write_whole(path: &Path, text: &str) -> Result<(), String> {
    let partial = path.with_extension("model.partial");
    std::fs::write(&partial, text)
        .and_then(|()| std::fs::rename(&partial, path))
        .map_err(|e| format!("{}: {e}", path.display()))
}

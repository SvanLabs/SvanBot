//! Compares the corrected hands of each assigned policy with the unassigned control.

use std::path::Path;

use svanbot_learn::paired::verdict;
use svanbot_runtime::{fleet::Fleet, store::Store};

#[derive(Debug, PartialEq)]
pub enum Guarded {
    NotYet(String),
    Holds(String),
    Trips(String),
}

pub fn judge(styled: &[f64], control: &[f64], hands: usize, margin_bb100: f64) -> Guarded {
    let hands = hands.max(1);
    if styled.len() < hands || control.len() < hands {
        return Guarded::NotYet(format!(
            "waiting for {hands} hands each (styled {}, control {})",
            styled.len(),
            control.len(),
        ));
    }
    let ours = verdict(styled);
    let theirs = verdict(control);
    let gap = ours.mbb_per_100 - theirs.mbb_per_100;
    let ci = ours.ci95.hypot(theirs.ci95);
    let evidence = format!(
        "{gap:+.0} mbb/100 ±{ci:.0} over {} styled and {} control hands",
        styled.len(),
        control.len(),
    );
    if gap + ci < -(margin_bb100 * 1_000.0) {
        Guarded::Trips(evidence)
    } else {
        Guarded::Holds(evidence)
    }
}

pub fn guard(
    fleet: &Fleet,
    data_dir: &Path,
    hands: usize,
    margin_bb100: f64,
) -> Result<Vec<(String, Guarded)>, String> {
    let assignments = fleet.assignments()?;
    if assignments.is_empty() {
        return Ok(Vec::new());
    }
    let entries =
        std::fs::read_dir(data_dir).map_err(|error| format!("{}: {error}", data_dir.display()))?;
    let mut control_stores = Vec::new();
    for entry in entries {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().is_some_and(|extension| extension == "db")
            && path.is_file()
            && let Some(name) = path.file_stem().and_then(|name| name.to_str())
            && !assignments.iter().any(|(bot, _, _)| bot == name)
        {
            control_stores.push(path);
        }
    }
    control_stores.sort_unstable();

    let mut results = Vec::with_capacity(assignments.len());
    for (bot, assignment, since) in assignments {
        if Path::new(&bot).file_name().and_then(|name| name.to_str()) != Some(&bot) {
            return Err(format!("invalid bot store name {bot:?}"));
        }
        let styled = results_since(&data_dir.join(format!("{bot}.db")), since)?;
        let mut control = Vec::new();
        for path in &control_stores {
            control.extend(results_since(path, since)?);
        }
        let mut result = judge(&styled, &control, hands, margin_bb100);
        if let Guarded::Trips(evidence) = &result {
            if fleet.revert_if(&bot, assignment, since)? {
                eprintln!("<2>svanbot: CRIT: {bot} reverted to the reference policy, {evidence}");
            } else {
                result = Guarded::NotYet("assignment changed during the guard check".into());
            }
        }
        results.push((bot, result));
    }
    Ok(results)
}

fn results_since(path: &Path, since: i64) -> Result<Vec<f64>, String> {
    Store::open_reader(path)
        .and_then(|store| store.corrected_since(since))
        .map_err(|error| format!("{}: {error}", path.display()))
}

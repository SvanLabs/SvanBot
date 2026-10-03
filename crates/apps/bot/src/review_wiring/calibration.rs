//! The self-calibration reach count (0332): how many decisions the installed bias decided.
//!
//! Split out of `review_wiring.rs` (0320: the 500-line rule).

use anyhow::Result;
use sv10_store::store::Store;

/// Hours of the decision log the calibration count reads.
pub const CALIBRATION_HOURS: i64 = 24;

/// On one street, how many decisions the self-calibration bias decided (0332).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CalibrationFlips {
    /// `preflop`, `flop`, `turn` or `river`.
    pub street: String,
    /// Decisions with two or more priced options.
    pub decisions: usize,
    /// Of those, the ones whose best action changes when each candidate's applied bias is removed.
    pub flipped: usize,
    /// `flipped` as a share of `decisions`, percent.
    pub share_pct: f64,
    /// The commonest change, `without -> with` (e.g. `fold -> call`), and how many.
    pub main: String,
    /// See [`CalibrationFlips::main`].
    pub main_count: usize,
}

/// Count, per street, the decisions whose best action (by family: every raise size is `raise`) differs
/// with and without the calibration bias recorded on each candidate. Pure over `(street, detail JSON)`.
pub fn calibration_flips(rows: &[(String, String)]) -> Vec<CalibrationFlips> {
    // Per street: decisions with a choice, and each (without, with) change of best action.
    type Changes = std::collections::BTreeMap<(String, String), usize>;
    let mut by: std::collections::BTreeMap<&str, (usize, Changes)> = Default::default();
    for (street, detail) in rows {
        let Ok(d) = serde_json::from_str::<serde_json::Value>(detail) else { continue };
        let cands: Vec<(&str, f64, f64)> = d["candidates"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|c| Some((c["action"].as_str()?, c["ev"].as_f64()?, c["bias"].as_f64().unwrap_or(0.0))))
            .collect();
        if cands.len() < 2 {
            continue;
        }
        let best = |f: &dyn Fn(&(&str, f64, f64)) -> f64| cands.iter().max_by(|a, b| f(a).total_cmp(&f(b))).map(|c| c.0).unwrap_or("");
        let (with, without) = (best(&|c| c.1), best(&|c| c.1 - c.2));
        let entry = by.entry(street.as_str()).or_default();
        entry.0 += 1;
        if with != without {
            *entry.1.entry((without.to_string(), with.to_string())).or_default() += 1;
        }
    }
    let order = |s: &str| ["preflop", "flop", "turn", "river"].iter().position(|x| *x == s).unwrap_or(4);
    let mut out: Vec<CalibrationFlips> = by
        .into_iter()
        .map(|(street, (n, flips))| {
            let flipped = flips.values().sum();
            let (main, main_count) =
                flips.iter().max_by_key(|(_, c)| **c).map(|((a, b), c)| (format!("{a} -> {b}"), *c)).unwrap_or_default();
            CalibrationFlips {
                street: street.to_string(),
                decisions: n,
                flipped,
                share_pct: flipped as f64 * 100.0 / n.max(1) as f64,
                main,
                main_count,
            }
        })
        .collect();
    out.sort_by_key(|f| order(&f.street));
    out
}

/// [`calibration_flips`] over the store's last [`CALIBRATION_HOURS`] of decisions.
pub(super) fn calibration_from(store: &Store) -> Result<Vec<CalibrationFlips>> {
    let since = (chrono::Utc::now() - chrono::Duration::hours(CALIBRATION_HOURS)).to_rfc3339();
    Ok(calibration_flips(&store.decision_details_since(&since)?))
}

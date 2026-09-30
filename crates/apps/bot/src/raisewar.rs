//! Raise-war equity study (0158): is the live equity estimate too high when a postflop pot has
//! been raised several times? Three season-13 losses over 1,000 bb were flop re-raise wars at an
//! estimated 0.66–0.86 against sets or better.
//!
//! Sample: our heads-up postflop decisions after which nobody acts again and the hand is shown
//! down (all-in), so nothing after the decision selects which hands reach showdown. Each is
//! scored against our exact equity versus the opponent's shown cards on the board at that moment
//! (every runout enumerated), which removes runout luck. Calls of an all-in are the clean test:
//! the estimate there is against the range that put the chips in. Our own shoves are reported
//! apart, since a called shove meets a calling range stronger than the range the estimate used.

use sv10_core::cards::{Card, CardMask, mask_of, parse_cards};
use sv10_core::eval::eval;

// One module per fitted band (0262); the names below are re-exported so every caller keeps its
// path (`sv10_bot::raisewar::fit_deep_call` and friends are unchanged).
pub mod deep;
pub mod multiway;
pub mod overbet;
pub mod river_jam;

pub use deep::{DEEP_CALL_KEY, DeepCallFit, fit_deep_call, installed_deep_call_shift};
pub use multiway::fit_multiway_call;
pub use overbet::{OVERBET_CALL_KEY, OVERBET_SLOPE_KEY, OverbetSlopeFit, fit_overbet_call, fit_overbet_slope, installed_overbet_slope};
pub use river_jam::{RIVER_JAM_KEY, RiverJamFit, fit_river_jam_call, installed_river_jam_shift};

/// One committed decision with its estimate and the exact outcome equity.
#[derive(Clone, Debug, PartialEq)]
pub struct Commit {
    /// 0 flop, 1 turn, 2 river.
    pub street: usize,
    /// Bets and raises, including short all-in raises, on the street before our action.
    pub raises_before: usize,
    /// Whether our action was a call (of an all-in) rather than our own raise or shove.
    pub call: bool,
    /// The live model's equity estimate.
    pub estimate: f64,
    /// Exact equity against the opponent's shown cards.
    pub exact: f64,
    /// Pot before our action and chips to call (0 when unknown).
    pub pot: i64,
    /// Chips we had to call.
    pub to_call: i64,
    /// Size of the last bet or raise an opponent made before our action on this street, as a
    /// multiple of the pot before it (0 when none): a 180x-pot shove reads very differently from a
    /// half-pot one (2026-09-23).
    pub bet_to_pot: f64,
}

impl Commit {
    /// Share of the final pot a call needs: to_call / (pot + to_call).
    pub fn price(&self) -> Option<f64> {
        (self.to_call > 0 && self.pot > 0).then(|| self.to_call as f64 / (self.pot + self.to_call) as f64)
    }

    /// Realized EV of the call in chips against the shown hand: exact × (pot + to_call) − to_call.
    pub fn call_ev(&self) -> Option<f64> {
        self.price()?;
        Some(self.exact * (self.pot + self.to_call) as f64 - self.to_call as f64)
    }
}

pub(crate) const STREETS: [&str; 3] = ["Flop", "Turn", "River"];

pub(crate) fn is_raise(a: &serde_json::Value) -> bool {
    match a["kind"].as_str() {
        Some("Raise" | "Bet") => true,
        Some("AllIn") => match (a["to"].as_i64(), a["bet_before"].as_i64(), a["to_call_before"].as_i64()) {
            (Some(to), Some(before), Some(call)) => to.saturating_sub(before) > call,
            // Legacy summaries lack chip amounts; preserve their full-raise convention.
            _ => a["full_raise"].as_bool().unwrap_or(true),
        },
        _ => false,
    }
}

/// Exact showdown equity (win + half tie) of `hero` against `villain` over every completion of
/// `board`.
pub fn exact_equity(hero: [Card; 2], villain: [Card; 2], board: &[Card]) -> f64 {
    let dead = mask_of(&hero) | mask_of(&villain) | mask_of(board);
    let rest: Vec<CardMask> = (0..52u8).map(|c| Card(c).bit()).filter(|b| dead & b == 0).collect();
    let (h, v, b) = (mask_of(&hero), mask_of(&villain), mask_of(board));
    let mut score = 0.0;
    let mut n = 0.0;
    let mut settle = |extra: CardMask| {
        let (hv, vv) = (eval(h | b | extra), eval(v | b | extra));
        score += if hv > vv {
            1.0
        } else if hv == vv {
            0.5
        } else {
            0.0
        };
        n += 1.0;
    };
    match 5 - board.len() {
        0 => settle(0),
        1 => rest.iter().for_each(|&x| settle(x)),
        _ => {
            for i in 0..rest.len() {
                for j in i + 1..rest.len() {
                    settle(rest[i] | rest[j]);
                }
            }
        }
    }
    score / n
}

/// Find our `ordinal`-th action on `street` in a stored hand summary and, when it commits the
/// hand heads-up to a showdown, score it. `None` when the decision cannot be matched or the hand
/// was not decided by that action.
pub fn commit_from_summary(
    summary: &serde_json::Value,
    hero: &str,
    street: usize,
    ordinal: usize,
    action: &str,
    estimate: f64,
    (pot, to_call): (i64, i64),
) -> Option<Commit> {
    let seat = summary["players"].as_array()?.iter().find(|p| p[1].as_str() == Some(hero))?[0].as_i64()?;
    let history = summary["history"].as_array()?;
    let name = STREETS[street];
    let at =
        history.iter().enumerate().filter(|(_, a)| a["seat"].as_i64() == Some(seat) && a["street"].as_str() == Some(name)).nth(ordinal)?.0;
    let ours = &history[at];
    let call = match (action, ours["kind"].as_str()?) {
        ("call", "Call") => true,
        ("call", "AllIn") if !is_raise(ours) => true,
        ("all_in", "AllIn") => !is_raise(ours),
        ("raise", "Raise") => false,
        _ => return None,
    };
    // Nothing after us but the opponent calling on this street: the chips are in.
    if !history[at + 1..]
        .iter()
        .all(|a| a["street"].as_str() == Some(name) && matches!(a["kind"].as_str(), Some("Call" | "AllIn")) && !is_raise(a))
    {
        return None;
    }
    let shown = summary["shown"].as_array()?;
    if shown.len() != 2 {
        return None;
    }
    let cards = |s: i64| -> Option<[Card; 2]> {
        let e = shown.iter().find(|e| e[0].as_i64() == Some(s))?;
        let v: Vec<&str> = e[1].as_array()?.iter().filter_map(|c| c.as_str()).collect();
        let c = parse_cards(&v)?;
        (c.len() == 2).then(|| [c[0], c[1]])
    };
    let hero_cards = cards(seat)?;
    let villain_seat = shown.iter().filter_map(|e| e[0].as_i64()).find(|&s| s != seat)?;
    let villain = cards(villain_seat)?;
    let board_all: Vec<&str> = summary["board"].as_array()?.iter().filter_map(|c| c.as_str()).collect();
    let board = parse_cards(&board_all[..(3 + street).min(board_all.len())])?;
    if board.len() != 3 + street {
        return None;
    }
    let raises_before = history[..at].iter().filter(|a| a["street"].as_str() == Some(name) && is_raise(a)).count();
    let bet_to_pot = history[..at]
        .iter()
        .rfind(|a| a["street"].as_str() == Some(name) && a["seat"].as_i64() != Some(seat) && is_raise(a))
        .and_then(|a| {
            let added = (a["to"].as_i64()? - a["bet_before"].as_i64().unwrap_or(0)) as f64;
            let before = a["pot_before"].as_i64()? as f64;
            (before > 0.0).then(|| added / before)
        })
        .unwrap_or(0.0);
    Some(Commit { street, raises_before, call, estimate, exact: exact_equity(hero_cards, villain, &board), pot, to_call, bet_to_pot })
}

/// Every committed heads-up postflop decision in the store, oldest first, with the number of
/// postflop decisions read.
pub fn commits_from_store(store: &sv10_store::store::Store) -> anyhow::Result<(usize, Vec<Commit>)> {
    use std::collections::HashMap;
    let mut ordinal: HashMap<(String, String, String), usize> = HashMap::new();
    let mut summaries: HashMap<(String, String), Option<serde_json::Value>> = HashMap::new();
    let mut commits = Vec::new();
    let decisions = store.postflop_decisions()?;
    for d in &decisions {
        let key = (d.bot.clone(), d.hand_id.clone(), d.street.clone());
        let k = *ordinal.entry(key).and_modify(|n| *n += 1).or_insert(0);
        if d.opponents != Some(1) || d.equity < 0.0 || !matches!(d.action.as_str(), "call" | "all_in" | "raise") {
            continue;
        }
        let Some(street) = ["flop", "turn", "river"].iter().position(|s| *s == d.street) else { continue };
        let summary = summaries
            .entry((d.bot.clone(), d.hand_id.clone()))
            .or_insert_with(|| store.hand(&d.bot, &d.hand_id).ok().flatten().and_then(|h| serde_json::from_str(&h.summary).ok()));
        let spot = (d.pot.unwrap_or(0), d.to_call.unwrap_or(0));
        if let Some(c) = summary.as_ref().and_then(|s| commit_from_summary(s, &d.bot, street, k, &d.action, d.equity, spot)) {
            commits.push(c);
        }
    }
    Ok((decisions.len(), commits))
}

/// Seconds between refits of the call fits while the learner waits for its next search (2026-09-23):
/// they take seconds, and waiting for the next search cycle (up to a day under the operator's
/// pacing) left a fit that had cleared its held-out test uninstalled.
pub const CALLS_REFIT_SECS: f64 = 3_600.0;

/// Whether the call fits (river and deep-pot) are due for a refit, `last` being the previous refit.
pub fn calls_refit_due(last: f64, now: f64) -> bool {
    now - last >= CALLS_REFIT_SECS
}

/// Shared band fit for [`fit_deep_call`] and [`fit_overbet_call`].
/// Largest equity shift any band fit may install (the river fit's own cap, reused as the band
/// clamp). Shared here because the band fitter and the river fit both clamp on it.
const JAM_MAX_SHIFT: f64 = 0.15;
/// Held-out calls the band fits need before they may install a shift (they are rare: ~90 deep
/// calls in a season). Shared here because the band fitter and the overbet slope both gate on it.
const DEEP_MIN_HELD_OUT: usize = 40;

fn fit_call_band(commits: &[Commit], keep: impl Fn(&Commit) -> bool) -> DeepCallFit {
    let calls: Vec<&Commit> = commits.iter().filter(|c| c.call && c.price().is_some() && keep(c)).collect();
    let mut fit = DeepCallFit { n: calls.len(), ..DeepCallFit::default() };
    let (train, test) = calls.split_at(calls.len() / 2);
    if test.len() < DEEP_MIN_HELD_OUT || train.is_empty() {
        return fit;
    }
    let mean_gap = |set: &[&Commit]| set.iter().map(|c| c.estimate - c.exact).sum::<f64>() / set.len() as f64;
    fit.train_shift = mean_gap(train).clamp(0.0, JAM_MAX_SHIFT * 2.0);
    let n = test.len() as f64;
    let gm = mean_gap(test);
    let gv = test.iter().map(|c| (c.estimate - c.exact - gm).powi(2)).sum::<f64>() / (n - 1.0);
    fit.held_out_gap = gm;
    fit.held_out_gap_lower = gm - 1.96 * (gv / n).sqrt();
    let saved: Vec<f64> = test
        .iter()
        .map(|c| match (c.price(), c.call_ev()) {
            (Some(price), Some(ev)) if c.estimate - fit.train_shift < price => -ev,
            _ => 0.0,
        })
        .collect();
    let sm = saved.iter().sum::<f64>() / n;
    let sv = saved.iter().map(|x| (x - sm).powi(2)).sum::<f64>() / (n - 1.0);
    fit.held_out_saved = sm;
    fit.held_out_saved_lower = sm - 1.96 * (sv / n).sqrt();
    let calibration_win = fit.held_out_gap_lower > fit.train_shift / 2.0;
    fit.active = fit.train_shift > 0.0 && (calibration_win || fit.held_out_saved_lower > 0.0);
    if fit.active {
        fit.shift = mean_gap(&calls).clamp(0.0, JAM_MAX_SHIFT * 2.0);
    } else if fit.train_shift > 0.0 && fit.held_out_gap_lower > 0.0 {
        // The full shift is not confirmed, but the held-out over-estimate is positive at 95%:
        // install only what that bound supports (0208, LESSONS 29).
        fit.active = true;
        fit.shift = fit.held_out_gap_lower.min(JAM_MAX_SHIFT * 2.0);
    }
    fit
}

/// Mean estimate, mean exact equity and the paired gap with its 95% half-width.
#[derive(Clone, Debug, PartialEq)]
pub struct Bin {
    /// Decisions in the bin.
    pub n: usize,
    /// Mean live estimate.
    pub estimate: f64,
    /// Mean exact equity.
    pub exact: f64,
    /// Mean of estimate minus exact (positive: over-estimate).
    pub gap: f64,
    /// 95% half-width of the gap.
    pub half_width: f64,
}

/// Summarize a set of commits.
pub fn bin(commits: &[&Commit]) -> Bin {
    let n = commits.len();
    if n == 0 {
        return Bin { n, estimate: 0.0, exact: 0.0, gap: 0.0, half_width: 0.0 };
    }
    let mean = |f: &dyn Fn(&Commit) -> f64| commits.iter().map(|c| f(c)).sum::<f64>() / n as f64;
    let gap = mean(&|c| c.estimate - c.exact);
    let var = if n > 1 { commits.iter().map(|c| (c.estimate - c.exact - gap).powi(2)).sum::<f64>() / (n - 1) as f64 } else { 0.0 };
    Bin { n, estimate: mean(&|c| c.estimate), exact: mean(&|c| c.exact), gap, half_width: 1.96 * (var / n as f64).sqrt() }
}

#[cfg(test)]
mod tests;

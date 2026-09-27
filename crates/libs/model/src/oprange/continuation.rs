//! Which part of a range continues against a bet: strength histograms, equity tables, continue-by-score.

use super::*;
use std::sync::OnceLock;
use sv10_stats::exp_memo::ExpMemo;

/// Histogram bins for strength distributions.
pub const STRENGTH_BINS: usize = 40;

/// Streets, as counted by [`Street::index`].
const STREETS: usize = 4;

/// Bin pairs per street: one hand's bin against one opponent bin.
const BIN_PAIRS: usize = STRENGTH_BINS * STRENGTH_BINS;

/// Strength-gap scale of the pairwise logistic, per street: strength gaps translate to smaller
/// equity gaps with more cards to come.
const TAU: [f32; STREETS] = [0.30, 0.12, 0.09, 0.05];

/// Equity floor per street: nobody is ever drawing completely dead before the river.
const EQ_FLOOR: [f32; STREETS] = [0.25, 0.12, 0.07, 0.02];

/// Center of strength bin `bin` on the 0..1 strength scale.
fn bin_center(bin: usize) -> f32 {
    (bin as f32 + 0.5) / STRENGTH_BINS as f32
}

/// `1 + exp((center(b) - center(v)) / tau)` for every bin pair, one table per street (0353).
///
/// The argument depends only on the two bin centers and the street's `tau`, never on the histogram,
/// so [`equity_vs_histogram`] was evaluating the same 1,600 logarithms on every call (measured over
/// a 120-hand suite: 202 of its 1,519 arguments per call distinct, and every argument of one call
/// repeats in the next). Each entry is built by the expression the inner loop used to evaluate in
/// place, so a lookup returns the bits that expression produced.
static LOGISTIC_DEN: [OnceLock<[f32; BIN_PAIRS]>; STREETS] = [const { OnceLock::new() }; STREETS];

/// The logistic denominators of `street`: `[v * STRENGTH_BINS + b]` is
/// `1 + exp((center(b) - center(v)) / tau)`.
fn logistic_den(street: Street) -> &'static [f32; BIN_PAIRS] {
    LOGISTIC_DEN[street.index()].get_or_init(|| {
        let tau = TAU[street.index()];
        let mut den = [0f32; BIN_PAIRS];
        for v in 0..STRENGTH_BINS {
            let sv = bin_center(v);
            for b in 0..STRENGTH_BINS {
                den[v * STRENGTH_BINS + b] = 1.0 + ((bin_center(b) - sv) / tau).exp();
            }
        }
        den
    })
}

/// Weighted histogram of range strength (bins over 0..1).
pub fn strength_histogram(range: &Range, strength: &[f32]) -> [f32; STRENGTH_BINS] {
    let mut h = [0f32; STRENGTH_BINS];
    let mut total = 0.0;
    for i in 0..NUM_COMBOS {
        let w = range.w[i];
        if w > 0.0 {
            let b = ((strength[i] * STRENGTH_BINS as f32) as usize).min(STRENGTH_BINS - 1);
            h[b] += w;
            total += w;
        }
    }
    if total > 0.0 {
        for x in h.iter_mut() {
            *x /= total;
        }
    }
    h
}

/// Approximate equity of a hand of strength-bin `v` against a range histogram:
/// pairwise win chance modeled as a logistic in the strength difference.
pub fn equity_vs_histogram(hist: &[f32; STRENGTH_BINS], street: Street) -> [f32; STRENGTH_BINS] {
    let den = logistic_den(street);
    let mut out = [0f32; STRENGTH_BINS];
    for v in 0..STRENGTH_BINS {
        let mut e = 0.0;
        for (b, &w) in hist.iter().enumerate() {
            if w > 0.0 {
                e += w / den[v * STRENGTH_BINS + b];
            }
        }
        let floor = EQ_FLOOR[street.index()];
        out[v] = floor + (1.0 - floor) * e;
    }
    out
}

/// A villain's response to hero's bet: they continue with the combos whose
/// equity against hero's perceived betting range clears the price they are
/// offered, shifted by how light they are known to call. Returns the
/// continuing fraction of their range and the continuing (unnormalized) range.
pub fn villain_continue(villain: &Range, strength: &[f32], hero_eq_table: &[f32; STRENGTH_BINS], need: f32, max_cont: f32) -> (f32, Range) {
    let scores: Vec<f32> =
        (0..NUM_COMBOS).map(|i| hero_eq_table[((strength[i] * STRENGTH_BINS as f32) as usize).min(STRENGTH_BINS - 1)]).collect();
    continue_by_score(villain, &scores, need, max_cont)
}

/// Keep combos whose score clears `need`; if that keeps more than `max_cont`
/// of the range, raise the bar until it doesn't.
pub fn continue_by_score(villain: &Range, scores: &[f32], need: f32, max_cont: f32) -> (f32, Range) {
    continue_by_order(villain, scores, None, need, max_cont)
}

/// `continue_by_score` with an optional precomputed strongest-first `order` of all combos by `scores`,
/// which replaces the per-call sort when the bar has to rise.
fn continue_by_order(villain: &Range, scores: &[f32], order: Option<&[u16; NUM_COMBOS]>, need: f32, max_cont: f32) -> (f32, Range) {
    let total: f32 = villain.w.iter().sum();
    if total <= 0.0 {
        return (0.0, villain.clone());
    }
    // The loop below evaluates one `exp` per live combo — 1,180 per call measured, and only ~120 of
    // those arguments are distinct (the scores come from a 40-bin equity table or the 169-rung
    // preflop ladder, and the bar is fixed for the call), so a memo removes ~90% of them.
    let mut memo = ExpMemo::new();
    let soft = |memo: &mut ExpMemo, bar: f32, score: f32| {
        let c = 1.0 / (1.0 + memo.exp((bar - score) / 0.012));
        if c < 0.03 { 0.0 } else { c }
    };
    let mut bar = need;
    // If the pot-odds threshold keeps more than the learned frequency allows, raise the
    // bar to the score at which the (hard) cumulative weight reaches `max_cont`.
    let above: f32 = (0..NUM_COMBOS).filter(|&i| villain.w[i] > 0.0 && scores[i] >= need).map(|i| villain.w[i]).sum();
    if above / total > max_cont {
        let target = max_cont * total;
        let mut acc = 0.0;
        let mut cross = |i: usize| {
            acc += villain.w[i];
            (acc >= target).then(|| scores[i].max(need))
        };
        let found = match order {
            Some(order) => order.iter().map(|&i| i as usize).filter(|&i| villain.w[i] > 0.0).find_map(&mut cross),
            None => by_descending_score(villain, scores).into_iter().find_map(&mut cross),
        };
        if let Some(b) = found {
            bar = b;
        }
    }
    let mut cont = villain.clone();
    let mut kept = 0.0;
    for i in 0..NUM_COMBOS {
        let w = villain.w[i];
        if w <= 0.0 {
            continue;
        }
        let c = soft(&mut memo, bar, scores[i]);
        cont.w[i] = w * c;
        kept += w * c;
    }
    (kept / total, cont)
}

/// The bar-rise index: `villain`'s live combos, strongest score first.
///
/// The sort must be a total order even when a fitted model hands it a NaN (0362). `partial_cmp(..)
/// .unwrap_or(Equal)` was not one — a NaN compares `Equal` to everything, so the relation stops
/// being transitive — and `sort_unstable_by` answers that with a debug-assertion panic on some NaN
/// inputs and silence on others, while the release build that plays returns merely whatever order
/// the sort happened to produce.
///
/// A NaN sorts **last**, and that is not cosmetic. The walk accumulates from the strongest score
/// down and sets the bar to the score it crosses on, so a NaN at the front would cross first — and
/// `f32::max(NaN, need)` is `need`, so the bar would come back unraised and the cap this branch
/// exists to enforce (`above / total > max_cont`) would be off for every real score in the range
/// (the bar decides the composition of the continuing range the caller prices against). Last leaves
/// the bar to the strongest real scores. `above` already reads a NaN the same way, since
/// `scores[i] >= need` is false for it, so this is the branch's existing reading of an unknown
/// score, not a new one. Real scores order exactly as they always did, sign of zero included.
fn by_descending_score(villain: &Range, scores: &[f32]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..NUM_COMBOS).filter(|&i| villain.w[i] > 0.0).collect();
    idx.sort_unstable_by(|&a, &b| bar_order(scores[a], scores[b]));
    idx
}

/// Ordering of two bar-rise scores: strongest first, NaN last (see [`by_descending_score`]).
///
/// Total by construction — the `is_nan` early return answers every comparison a NaN takes part in,
/// so `partial_cmp` is only ever asked about two numbers. `true > false`, so a NaN goes after a
/// real score, and two NaNs tie.
fn bar_order(a: f32, b: f32) -> std::cmp::Ordering {
    if a.is_nan() || b.is_nan() {
        return a.is_nan().cmp(&b.is_nan());
    }
    if a > b {
        std::cmp::Ordering::Less
    } else if a < b {
        std::cmp::Ordering::Greater
    } else {
        std::cmp::Ordering::Equal
    }
}

/// Fraction of hands (top-x) an opponent attributes to hero raising preflop to `to`.
pub fn preflop_raise_fraction(sit: &Situation, prof: &Profile, to: i64) -> f32 {
    let cur = sit.current_bet();
    let raises = sit.history.iter().filter(|r| r.street == Street::Preflop && aggressive(r)).count();
    let pos = sit.position_of(sit.hero_seat);
    let bb = sit.bb as f64;
    let mut frac = match raises {
        0 => prof.open_raise * position_open_factor(pos),
        1 => prof.three_bet,
        _ => (prof.four_bet * prof.three_bet * 4.0).max(0.02),
    };
    let normal = match raises {
        0 => 2.5 * bb,
        1 => cur as f64 * 3.3,
        _ => cur as f64 * 2.3,
    };
    if to as f64 > normal * 1.5 {
        frac *= ((normal * 1.5) / to as f64).powf(0.5) as f32;
    }
    frac.clamp(0.012, 0.95)
}

/// Preflop villain response: continue with classes whose equity against
/// hero's perceived top-`hero_frac` range clears `need`.
pub fn villain_continue_preflop(villain: &Range, hero_frac: f32, need: f32, max_cont: f32) -> (f32, Range) {
    let t = preflop::combo_scores_vs_top(hero_frac);
    continue_by_order(villain, &t.score, Some(&t.order), need, max_cont)
}

#[cfg(test)]
mod tests;

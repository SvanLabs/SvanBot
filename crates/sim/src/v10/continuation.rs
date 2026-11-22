//! v10.0.2's read of who goes on against a bet: a seat's hands, binned by strength, each scored
//! by its equity against hero's binned range, and kept above a bar the price sets.
//!
//! v10 memoised `exp` here for speed; its memo only ever returned `f32::exp`'s own bits, so the
//! port calls `exp` and gives the same numbers.

use std::cmp::Ordering;
use std::sync::OnceLock;

use super::cards::{NUM_COMBOS, Range};
use super::preflop;
use super::profile::Profile;
use super::ranges::position_open_factor;
use super::situation::{Situation, Street};

pub const STRENGTH_BINS: usize = 40;
const STREETS: usize = 4;
const BIN_PAIRS: usize = STRENGTH_BINS * STRENGTH_BINS;
const TAU: [f32; STREETS] = [0.30, 0.12, 0.09, 0.05];
const EQ_FLOOR: [f32; STREETS] = [0.25, 0.12, 0.07, 0.02];

fn bin_center(bin: usize) -> f32 {
    (bin as f32 + 0.5) / STRENGTH_BINS as f32
}

fn bin_of(strength: f32) -> usize {
    ((strength * STRENGTH_BINS as f32) as usize).min(STRENGTH_BINS - 1)
}

fn logistic_den(street: Street) -> &'static [f32; BIN_PAIRS] {
    static DEN: [OnceLock<[f32; BIN_PAIRS]>; STREETS] = [const { OnceLock::new() }; STREETS];
    DEN[street.index()].get_or_init(|| {
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

/// The share of `range` in each strength bin.
pub fn strength_histogram(range: &Range, strength: &[f32]) -> [f32; STRENGTH_BINS] {
    let mut h = [0f32; STRENGTH_BINS];
    let mut total = 0.0;
    for i in 0..NUM_COMBOS {
        let w = range.w[i];
        if w > 0.0 {
            h[bin_of(strength[i])] += w;
            total += w;
        }
    }
    if total > 0.0 {
        h.iter_mut().for_each(|x| *x /= total);
    }
    h
}

/// A hand's equity in each strength bin against a range binned as `hist`.
pub fn equity_vs_histogram(hist: &[f32; STRENGTH_BINS], street: Street) -> [f32; STRENGTH_BINS] {
    let den = logistic_den(street);
    let floor = EQ_FLOOR[street.index()];
    let mut out = [0f32; STRENGTH_BINS];
    for (v, slot) in out.iter_mut().enumerate() {
        let mut e = 0.0;
        for (b, &w) in hist.iter().enumerate() {
            if w > 0.0 {
                e += w / den[v * STRENGTH_BINS + b];
            }
        }
        *slot = floor + (1.0 - floor) * e;
    }
    out
}

/// The share of `villain` that goes on, and what goes on, scoring each combo by its bin's equity.
pub fn villain_continue(
    villain: &Range,
    strength: &[f32],
    hero_eq_table: &[f32; STRENGTH_BINS],
    need: f32,
    max_cont: f32,
) -> (f32, Range) {
    let scores: Vec<f32> = (0..NUM_COMBOS)
        .map(|i| hero_eq_table[bin_of(strength[i])])
        .collect();
    continue_by_order(villain, &scores, None, need, max_cont)
}

/// The same, preflop: each combo scored against the top `hero_frac` of hands.
pub fn villain_continue_preflop(
    villain: &Range,
    hero_frac: f32,
    need: f32,
    max_cont: f32,
) -> (f32, Range) {
    let t = preflop::combo_scores_vs_top(hero_frac);
    continue_by_order(villain, &t.score, Some(&t.order), need, max_cont)
}

fn continue_by_order(
    villain: &Range,
    scores: &[f32],
    order: Option<&[u16; NUM_COMBOS]>,
    need: f32,
    max_cont: f32,
) -> (f32, Range) {
    let total: f32 = villain.w.iter().sum();
    if total <= 0.0 {
        return (0.0, villain.clone());
    }
    let soft = |bar: f32, score: f32| {
        let c = 1.0 / (1.0 + ((bar - score) / 0.012).exp());
        if c < 0.03 { 0.0 } else { c }
    };
    let mut bar = need;
    let above: f32 = (0..NUM_COMBOS)
        .filter(|&i| villain.w[i] > 0.0 && scores[i] >= need)
        .map(|i| villain.w[i])
        .sum();
    if above / total > max_cont {
        let target = max_cont * total;
        let mut acc = 0.0;
        let mut cross = |i: usize| {
            acc += villain.w[i];
            (acc >= target).then(|| scores[i].max(need))
        };
        let found = match order {
            Some(order) => order
                .iter()
                .map(|&i| usize::from(i))
                .filter(|&i| villain.w[i] > 0.0)
                .find_map(&mut cross),
            None => by_descending_score(villain, scores)
                .into_iter()
                .find_map(&mut cross),
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
        let c = soft(bar, scores[i]);
        cont.w[i] = w * c;
        kept += w * c;
    }
    (kept / total, cont)
}

fn by_descending_score(villain: &Range, scores: &[f32]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..NUM_COMBOS).filter(|&i| villain.w[i] > 0.0).collect();
    idx.sort_unstable_by(|&a, &b| bar_order(scores[a], scores[b]));
    idx
}

fn bar_order(a: f32, b: f32) -> Ordering {
    if a.is_nan() || b.is_nan() {
        return a.is_nan().cmp(&b.is_nan());
    }
    if a > b {
        Ordering::Less
    } else if a < b {
        Ordering::Greater
    } else {
        Ordering::Equal
    }
}

/// The share of hands hero would raise to `to` with, as the table reads it.
pub fn preflop_raise_fraction(sit: &Situation, prof: &Profile, to: i64) -> f32 {
    let cur = sit.current_bet();
    let raises = sit
        .history
        .iter()
        .filter(|r| r.street == Street::Preflop && r.aggressive())
        .count();
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

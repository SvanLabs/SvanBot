//! Overbet call fits: the flat band (0203) and the size-scaled slope (0242), split from the raise-war module (0262).

use super::deep::DeepCallFit;
use super::{Commit, DEEP_MIN_HELD_OUT, fit_call_band};

/// Kv key of the stored overbet call fit (a [`DeepCallFit`], 2026-09-23).
pub const OVERBET_CALL_KEY: &str = "overbet_call.v1";

/// Fit the overbet call shift: calls of an all-in whose bet was at least
/// `sv10_core::policy::OVERBET_CALL_MIN_RATIO` times the pot before it (against 4x+ pot shoves our
/// estimate was 0.623 vs 0.297 exact, n 36), gated exactly like [`fit_deep_call`].
pub fn fit_overbet_call(commits: &[Commit]) -> DeepCallFit {
    fit_call_band(commits, |c| c.bet_to_pot >= sv10_core::policy::OVERBET_CALL_MIN_RATIO)
}
/// Kv key of the stored size-scaled overbet call fit (an [`OverbetSlopeFit`], 0233).
pub const OVERBET_SLOPE_KEY: &str = "overbet_slope.v1";
/// Largest slope the size-scaled overbet fit may install (with the policy's 0.40 cap on the shift).
const OVERBET_MAX_SLOPE: f64 = 0.2;

/// Live fit of the size-scaled overbet call shift (0233): against an all-in of `r` times the pot the
/// estimate over-estimated equity by more the bigger the shove (1.5–4x: +0.115, n 63; 4x+: +0.291,
/// n 42 on 2026-09-26), which one flat shift over both bands (0203) could only install as −0.028. The
/// gap is fitted as `slope × x(r)`, `x = sv10_core::policy::overbet_size_x`, by least squares through
/// the origin on the older half and gated on the newer half exactly like [`fit_deep_call`].
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct OverbetSlopeFit {
    /// Calls of an overbet all-in with a known price.
    pub n: usize,
    /// Slope fitted on the older half (clamped to 0..0.2).
    pub train_slope: f64,
    /// Slope fitted on the newer (held-out) half and its 95% lower bound.
    pub held_out_slope: f64,
    /// 95% lower bound of `held_out_slope`.
    pub held_out_slope_lower: f64,
    /// Mean held-out over-estimate before and after the older-half slope.
    pub held_out_gap_before: f64,
    /// Mean held-out over-estimate left after applying `train_slope`.
    pub held_out_gap_after: f64,
    /// Chips per held-out call the older-half slope saves (calls it flips to folds, realized EV
    /// negated) and its 95% lower bound.
    pub held_out_saved: f64,
    /// 95% lower bound of `held_out_saved`.
    pub held_out_saved_lower: f64,
    /// Installed: the held-out slope clears half the older-half slope at 95% or the saved chips are
    /// positive at 95% (then the slope over all calls); failing both, a held-out slope positive at 95%
    /// installs that lower bound (0208, LESSONS 29).
    pub active: bool,
    /// Installed slope (0 when inactive).
    pub slope: f64,
}

/// Fit the size-scaled overbet call shift on `commits` (oldest first).
pub fn fit_overbet_slope(commits: &[Commit]) -> OverbetSlopeFit {
    use sv10_core::policy::{OVERBET_CALL_MIN_RATIO, OVERBET_SLOPE_CAP, overbet_size_x};
    let calls: Vec<&Commit> = commits.iter().filter(|c| c.call && c.price().is_some() && c.bet_to_pot >= OVERBET_CALL_MIN_RATIO).collect();
    let mut fit = OverbetSlopeFit { n: calls.len(), ..OverbetSlopeFit::default() };
    let (train, test) = calls.split_at(calls.len() / 2);
    if test.len() < DEEP_MIN_HELD_OUT || train.is_empty() {
        return fit;
    }
    let gap = |c: &Commit| c.estimate - c.exact;
    // Least squares through the origin: slope = Σ x·gap / Σ x², with its standard error.
    let slope_of = |set: &[&Commit]| {
        let (sxy, sxx) = set.iter().fold((0.0, 0.0), |(a, b), c| {
            let x = overbet_size_x(c.bet_to_pot);
            (a + x * gap(c), b + x * x)
        });
        let k = if sxx > 0.0 { sxy / sxx } else { 0.0 };
        let n = set.len() as f64;
        let rss = set.iter().map(|c| (gap(c) - k * overbet_size_x(c.bet_to_pot)).powi(2)).sum::<f64>();
        let se = if n > 1.0 && sxx > 0.0 { (rss / (n - 1.0) / sxx).sqrt() } else { f64::INFINITY };
        (k, se)
    };
    fit.train_slope = slope_of(train).0.clamp(0.0, OVERBET_MAX_SLOPE);
    let (k, se) = slope_of(test);
    fit.held_out_slope = k;
    fit.held_out_slope_lower = k - 1.96 * se;
    let n = test.len() as f64;
    let shift = |c: &Commit, slope: f64| (slope * overbet_size_x(c.bet_to_pot)).min(OVERBET_SLOPE_CAP);
    fit.held_out_gap_before = test.iter().map(|c| gap(c)).sum::<f64>() / n;
    fit.held_out_gap_after = test.iter().map(|c| gap(c) - shift(c, fit.train_slope)).sum::<f64>() / n;
    let saved: Vec<f64> = test
        .iter()
        .map(|c| match (c.price(), c.call_ev()) {
            (Some(price), Some(ev)) if c.estimate - shift(c, fit.train_slope) < price => -ev,
            _ => 0.0,
        })
        .collect();
    let sm = saved.iter().sum::<f64>() / n;
    let sv = saved.iter().map(|x| (x - sm).powi(2)).sum::<f64>() / (n - 1.0);
    fit.held_out_saved = sm;
    fit.held_out_saved_lower = sm - 1.96 * (sv / n).sqrt();
    let calibration_win = fit.held_out_slope_lower > fit.train_slope / 2.0;
    fit.active = fit.train_slope > 0.0 && (calibration_win || fit.held_out_saved_lower > 0.0);
    if fit.active {
        fit.slope = slope_of(&calls).0.clamp(0.0, OVERBET_MAX_SLOPE);
    } else if fit.train_slope > 0.0 && fit.held_out_slope_lower > 0.0 {
        fit.active = true;
        fit.slope = fit.held_out_slope_lower.min(OVERBET_MAX_SLOPE);
    }
    fit
}

/// The size-scaled overbet slope to play with, from the stored fit (0 when absent, unreadable or
/// inactive).
pub fn installed_overbet_slope(stored: Option<&str>) -> f64 {
    stored.and_then(|j| serde_json::from_str::<OverbetSlopeFit>(j).ok()).filter(|f| f.active).map(|f| f.slope).unwrap_or(0.0)
}

//! River all-in call equity shift fit (0159), split from the raise-war module (0262).

use super::{Commit, JAM_MAX_SHIFT};

/// Kv key of the stored [`RiverJamFit`] (0159).
pub const RIVER_JAM_KEY: &str = "river_jam_call.v1";
/// Held-out river calls the fit needs before it may install a shift.
const JAM_MIN_HELD_OUT: usize = 150;

/// Live fit of the river all-in call equity shift, with its held-out evidence (0159).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RiverJamFit {
    /// River calls of an all-in with a known price.
    pub n: usize,
    /// Mean over-estimate (estimate minus exact equity) on the older half.
    pub train_shift: f64,
    /// Chips per held-out call the older-half shift saves: calls it would have folded, with their
    /// realized EV against the shown hand negated.
    pub held_out_saved: f64,
    /// 95% lower bound of `held_out_saved`.
    pub held_out_lower: f64,
    /// Mean over-estimate on the newer (held-out) half, and its 95% lower bound. Applying the
    /// older-half shift lowers held-out squared calibration error exactly when this gap exceeds
    /// half the shift, so `held_out_gap_lower > train_shift / 2` is a held-out win at 95% (2026-09-23).
    #[serde(default)]
    pub held_out_gap: f64,
    /// 95% lower bound of `held_out_gap`.
    #[serde(default)]
    pub held_out_gap_lower: f64,
    /// Whether the shift is installed.
    pub active: bool,
    /// Installed shift (mean over-estimate on every call; 0 when inactive).
    pub shift: f64,
}

/// Fit the river all-in call shift on `commits` (oldest first): the mean over-estimate on the older
/// half, kept when it wins on the newer half at 95%, by either test: folding the calls it flips
/// would have saved chips, or it lowers held-out calibration error (the newer half's over-estimate
/// clears half the shift). Chips alone proved too noisy: a few huge pots kept a steady +0.08 bias
/// uninstalled for days (2026-09-23). Selection note: only calls we made are observed, so the
/// shift may only lower the call's value; folding is the side the evidence covers.
pub fn fit_river_jam_call(commits: &[Commit]) -> RiverJamFit {
    let calls: Vec<&Commit> = commits.iter().filter(|c| c.call && c.street == 2 && c.price().is_some()).collect();
    let mut fit = RiverJamFit { n: calls.len(), ..RiverJamFit::default() };
    let (train, test) = calls.split_at(calls.len() / 2);
    if test.len() < JAM_MIN_HELD_OUT || train.is_empty() {
        return fit;
    }
    let gap = |set: &[&Commit]| (set.iter().map(|c| c.estimate - c.exact).sum::<f64>() / set.len() as f64).clamp(0.0, JAM_MAX_SHIFT);
    fit.train_shift = gap(train);
    let saved: Vec<f64> = test
        .iter()
        .map(|c| match (c.price(), c.call_ev()) {
            (Some(price), Some(ev)) if c.estimate - fit.train_shift < price => -ev,
            _ => 0.0,
        })
        .collect();
    let n = saved.len() as f64;
    let mean = saved.iter().sum::<f64>() / n;
    let var = saved.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    fit.held_out_saved = mean;
    fit.held_out_lower = mean - 1.96 * (var / n).sqrt();
    let gaps: Vec<f64> = test.iter().map(|c| c.estimate - c.exact).collect();
    let gm = gaps.iter().sum::<f64>() / n;
    let gv = gaps.iter().map(|x| (x - gm).powi(2)).sum::<f64>() / (n - 1.0);
    fit.held_out_gap = gm;
    fit.held_out_gap_lower = gm - 1.96 * (gv / n).sqrt();
    let calibration_win = fit.held_out_gap_lower > fit.train_shift / 2.0;
    fit.active = fit.train_shift > 0.0 && (fit.held_out_lower > 0.0 || calibration_win);
    if fit.active {
        fit.shift = gap(&calls);
    }
    fit
}

/// The shift to play with, from the stored fit (0 when absent, unreadable or inactive).
pub fn installed_river_jam_shift(stored: Option<&str>) -> f64 {
    stored.and_then(|j| serde_json::from_str::<RiverJamFit>(j).ok()).map(|f| f.shift).unwrap_or(0.0)
}

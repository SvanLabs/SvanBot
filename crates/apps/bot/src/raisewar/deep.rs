//! Deep-pot all-in call shift fit (2026-09-23), split from the raise-war module (0262).

use super::{Commit, fit_call_band};

/// Kv key of the stored [`DeepCallFit`] (2026-09-23).
pub const DEEP_CALL_KEY: &str = "deep_call.v1";

/// Live fit of the deep-pot all-in call shift: calls of an all-in in pots of at least
/// `sv10_core::policy::DEEP_CALL_MIN_POT_BB` over-estimated equity by 0.154 ± 0.080 (90 calls, the
/// 2026-09-23 study) while smaller pots were calibrated.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct DeepCallFit {
    /// Deep calls of an all-in (any street) with a known price.
    pub n: usize,
    /// Mean over-estimate on the older half.
    pub train_shift: f64,
    /// Mean over-estimate on the newer (held-out) half and its 95% lower bound.
    pub held_out_gap: f64,
    /// 95% lower bound of `held_out_gap`.
    pub held_out_gap_lower: f64,
    /// Chips per held-out deep call the older-half shift saves (calls it flips, realized EV negated).
    #[serde(default)]
    pub held_out_saved: f64,
    /// 95% lower bound of `held_out_saved`.
    #[serde(default)]
    pub held_out_saved_lower: f64,
    /// Whether the shift is installed: on the held-out half it lowers calibration error at 95%
    /// (the over-estimate clears half the shift) or saves chips at 95%, the river fit's two tests;
    /// failing both, a held-out over-estimate positive at 95% still installs its lower bound (0208).
    pub active: bool,
    /// Installed shift (the mean over-estimate on every call in the band, or the held-out 95% lower
    /// bound when only that is confirmed; 0 when inactive).
    pub shift: f64,
}

/// Fit the deep-pot call shift on `commits` (oldest first) with the big blind `bb`.
pub fn fit_deep_call(commits: &[Commit], bb: f64) -> DeepCallFit {
    let min_pot = sv10_core::policy::DEEP_CALL_MIN_POT_BB * bb.max(1.0);
    fit_call_band(commits, |c| c.pot as f64 >= min_pot)
}
/// The deep-call shift to play with, from the stored fit (0 when absent, unreadable or inactive).
pub fn installed_deep_call_shift(stored: Option<&str>) -> f64 {
    stored.and_then(|j| serde_json::from_str::<DeepCallFit>(j).ok()).filter(|f| f.active).map(|f| f.shift).unwrap_or(0.0)
}

//! Calibration at the decision margin (0222): is a category's predicted EV right where a small
//! error would flip the decision? Self-calibration (`sv10_bot::tasks`) corrects each category by one
//! capped bias; its residual is dominated by big-EV spots, where the action is clear anyway. This
//! splits the residual by predicted-EV bin and subtracts the installed correction, so the question
//! "does the policy fold or continue too much near the margin" has a direct answer.
//! `review margins` prints it.
//!
//! This is the *pricing* instrument (0346), and the numbers say exactly what they are:
//! - population: every settled decision the bot priced and got a stack result for, at every pot size
//!   (the `calibration` table). The deep audit's rows are the analyst's big spots only — a different
//!   population, never the same rows.
//! - basis: `residual` is realized minus the **uncorrected** predicted EV. Live play records each
//!   sample with the bias taken back out (`client/decide.rs`: `candidate.ev - candidate.bias`), so
//!   the bias is what self-calibration is *fitted from*, not part of the price this residual is
//!   measured against.
//! - `after_correction` subtracts the installed correction at its largest size. In play a penalty is
//!   capped at its supported per-pot residual (`policy::applied_bias`, 0207), so for a capped
//!   category this is the most, not the exact amount, play corrects a decision by.

use std::collections::HashMap;

/// Predicted-EV bins (big blinds): the margin is the bins around zero.
pub const BINS: [(f64, f64); 8] =
    [(f64::NEG_INFINITY, -1.0), (-1.0, 0.0), (0.0, 0.5), (0.5, 1.0), (1.0, 2.0), (2.0, 5.0), (5.0, 15.0), (15.0, f64::INFINITY)];

/// Predicted-EV range (bb) that counts as the decision margin for self-calibration's upward bound
/// (0222): where a correction of a few big blinds can flip the action.
pub const MARGIN: (f64, f64) = (-1.0, 1.0);

/// Fewest samples a bin needs to be reported.
pub const MIN_BIN: usize = 30;

/// One predicted-EV bin of one category.
#[derive(Clone, Debug, PartialEq)]
pub struct MarginBin {
    /// Lower edge of the bin (bb).
    pub lo: f64,
    /// Upper edge of the bin (bb).
    pub hi: f64,
    /// Samples in the bin.
    pub n: usize,
    /// Mean realized minus predicted (bb), before the live correction.
    pub residual: f64,
    /// 95% half-width of `residual`.
    pub half_width: f64,
    /// `residual` minus the installed correction for this category, at its largest size (see the
    /// module docs: a penalty is capped per pot in play, so this is the most it corrects by).
    pub after_correction: f64,
}

impl MarginBin {
    /// Whether the bin straddles or touches zero predicted EV, where decisions are close.
    pub fn at_margin(&self) -> bool {
        self.lo < 1.0 && self.hi > -1.0
    }

    /// Whether the corrected residual is significantly away from zero at 95%.
    pub fn miscalibrated(&self) -> bool {
        self.after_correction.abs() > self.half_width
    }
}

/// Bin `samples` (category, predicted bb, realized bb) by predicted EV for each category in
/// `categories` (all categories when empty), subtracting each category's installed correction in
/// `bias` (bb, at its largest size; categories without one get 0).
pub fn margins(samples: &[(String, f64, f64)], categories: &[&str], bias: &HashMap<String, f64>) -> Vec<(String, Vec<MarginBin>)> {
    let mut by: HashMap<&str, Vec<(f64, f64)>> = HashMap::new();
    for (cat, predicted, realized) in samples {
        if (categories.is_empty() || categories.contains(&cat.as_str())) && predicted.is_finite() && realized.is_finite() {
            by.entry(cat.as_str()).or_default().push((*predicted, realized - predicted));
        }
    }
    let mut out: Vec<(String, Vec<MarginBin>)> = by
        .into_iter()
        .map(|(cat, rows)| {
            let correction = bias.get(cat).copied().unwrap_or(0.0);
            let bins = BINS
                .iter()
                .filter_map(|&(lo, hi)| {
                    let r: Vec<f64> = rows.iter().filter(|(p, _)| *p >= lo && *p < hi).map(|(_, r)| *r).collect();
                    let n = r.len();
                    if n < MIN_BIN {
                        return None;
                    }
                    let (mean, half_width) = crate::moments::mean_half_width(&r, 1.96);
                    Some(MarginBin { lo, hi, n, residual: mean, half_width, after_correction: mean - correction })
                })
                .collect();
            (cat.to_string(), bins)
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Each category's installed EV correction in big blinds, from the stored self-calibration
/// (`sv10_bot::CALIBRATION_KEY`); empty when absent or unreadable. A penalty is also bounded per pot
/// in play (`policy::applied_bias`), so for a negative correction this is its largest size.
pub fn installed_bias(stored: Option<&str>) -> HashMap<String, f64> {
    let Some(v) = stored.and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok()) else { return HashMap::new() };
    v.as_object().map(|m| m.iter().filter_map(|(k, e)| Some((k.clone(), e["bias_bb"].as_f64()?))).collect()).unwrap_or_default()
}

/// The rule that set each installed correction's size (`bound_by` in the same stored table, 0330):
/// the margin evidence, a cap, the 95% band, or the evidence in full.
pub fn bound_by(stored: Option<&str>) -> HashMap<String, String> {
    let Some(v) = stored.and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok()) else { return HashMap::new() };
    v.as_object()
        .map(|m| m.iter().filter_map(|(k, e)| Some((k.clone(), e["bound_by"].as_str()?.to_string()))).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(cat: &str, predicted: f64, residuals: &[f64]) -> Vec<(String, f64, f64)> {
        residuals.iter().map(|r| (cat.to_string(), predicted, predicted + r)).collect()
    }

    #[test]
    fn a_margin_residual_that_the_correction_covers_is_calibrated() {
        // Marginal calls realize +3 bb over prediction; live play already adds +3.
        let mut s = rows("preflop:call", -0.5, &[2.0, 4.0].repeat(50));
        s.extend(rows("preflop:call", 20.0, &[30.0, 50.0].repeat(50)));
        s.extend(rows("flop:call", 0.2, &[1.0].repeat(10)));
        let bias: HashMap<String, f64> = [("preflop:call".to_string(), 3.0)].into_iter().collect();
        let m = margins(&s, &["preflop:call"], &bias);
        assert_eq!(m.len(), 1, "only the asked category");
        let bins = &m[0].1;
        assert_eq!(bins.len(), 2);
        let margin = bins.iter().find(|b| b.at_margin()).unwrap();
        assert_eq!(margin.n, 100);
        assert!((margin.residual - 3.0).abs() < 1e-9 && margin.after_correction.abs() < 1e-9);
        assert!(!margin.miscalibrated());
        let big = bins.iter().find(|b| !b.at_margin()).unwrap();
        assert!((big.after_correction - 37.0).abs() < 1e-9 && big.miscalibrated());
    }

    #[test]
    fn thin_bins_are_left_out_and_bias_parses_from_the_stored_fit() {
        let s = rows("river:call", 0.1, &[1.0; 5]);
        assert!(margins(&s, &[], &HashMap::new())[0].1.is_empty());
        let stored = r#"{"river:call":{"bias_bb":-8.67,"n":1554},"flop:check":{"n":3}}"#;
        let b = installed_bias(Some(stored));
        assert_eq!(b.len(), 1);
        assert_eq!(b["river:call"], -8.67);
        assert!(installed_bias(Some("junk")).is_empty() && installed_bias(None).is_empty());
    }
}

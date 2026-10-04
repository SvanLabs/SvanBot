//! Per-street fold calibration (0156): the live policy's final fold estimate for our heads-up
//! postflop bets, checked against what opponents actually did. Before the fit, on 2026-09-22, flop
//! folds were under-predicted (held-out logit shift +0.21, +7.4 mnats) and river folds
//! over-predicted (-0.74, +14.8 mnats). Each street gets a logit shift fitted on the older half of
//! the samples and kept only when it improves log-loss on the newer half with a positive 95% lower
//! bound; the installed shift is then refitted on every sample. Predictions are un-shifted with the
//! shift recorded on each decision, so refits never compound earlier corrections.

use serde::{Deserialize, Serialize};
use sv10_store::store::Store;

/// Kv key of the stored [`FoldCalibration`].
pub const FOLD_CAL_KEY: &str = "fold_calibration.v1";

/// Held-out samples a street needs before its shift can be installed.
const MIN_HELD_OUT: usize = 200;
/// Largest logit shift the fit may choose (about a 2.7x change in fold odds).
const MAX_SHIFT: f64 = 1.0;
/// Largest preflop logit shift (2026-09-23: the measured gap is about -1.3 to -1.7).
const PREFLOP_MAX_SHIFT: f64 = 2.0;
/// Sample street index for preflop raises (postflop streets are 0..=2).
pub const PREFLOP: usize = 3;
/// Most recent samples used per fit.
const MAX_SAMPLES: usize = 60_000;

/// One heads-up postflop bet: the fold estimate before any calibration shift and whether the
/// opponent folded.
#[derive(Clone, Debug)]
pub struct FoldSample {
    /// Decision time (RFC 3339), for the chronological split.
    pub ts: String,
    /// 0 flop, 1 turn, 2 river, [`PREFLOP`] for our preflop raises.
    pub street: usize,
    /// Fold probability before the calibration shift (for preflop: that everyone folds).
    pub raw: f64,
    /// Whether the opponent folded to the bet (for preflop: whether everyone folded).
    pub folded: bool,
    /// The opponent who answered a postflop bet (none for preflop raises), for per-opponent
    /// calibration (0214).
    pub opponent: Option<String>,
}

/// Fit report for one street.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct StreetFit {
    /// Samples used.
    pub n: usize,
    /// Mean predicted fold rate before the shift.
    pub predicted: f64,
    /// Realized fold rate.
    pub actual: f64,
    /// Shift fitted on the older half.
    pub train_shift: f64,
    /// Held-out log-loss gain of that shift, nats per sample.
    pub held_out_gain: f64,
    /// 95% lower bound of the held-out gain.
    pub held_out_lower: f64,
    /// Whether the street's shift is installed.
    pub active: bool,
    /// Installed shift (refitted on all samples; 0 when inactive).
    pub shift: f64,
}

/// Stored fold calibration: installed shifts plus the per-street evidence.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FoldCalibration {
    /// Installed logit shift per postflop street (flop, turn, river).
    pub shift: [f64; 3],
    /// Evidence per street.
    pub streets: [StreetFit; 3],
    /// Unix time of the fit.
    pub fitted_at: f64,
    /// Installed logit shift for the everyone-folds estimate of our preflop raises (2026-09-23:
    /// 36,080 raises predicted ~2x the folds that happened, e.g. 0.55 predicted vs 0.24 realized).
    #[serde(default)]
    pub preflop_shift: f64,
    /// Evidence for the preflop shift.
    #[serde(default)]
    pub preflop: StreetFit,
}

/// The shifts to play with, from the stored calibration (zeros when absent or unreadable).
pub fn installed_shift(stored: Option<&str>) -> [f64; 3] {
    stored.and_then(|j| serde_json::from_str::<FoldCalibration>(j).ok()).map(|c| c.shift).unwrap_or([0.0; 3])
}

/// The preflop shift to play with, from the stored calibration (0 when absent or unreadable).
pub fn installed_preflop_shift(stored: Option<&str>) -> f64 {
    stored.and_then(|j| serde_json::from_str::<FoldCalibration>(j).ok()).map(|c| c.preflop_shift).unwrap_or(0.0)
}

/// Stored fits older than this are refitted when the learner starts.
const STALE_AFTER_SECS: f64 = 3_600.0;

/// Whether the learner should refit at start (unix `now`): nothing stored, unreadable, or older
/// than an hour. Otherwise a release or restart would play uncalibrated until the next cycle — and
/// a quiet table, where hands stopped arriving, until the next hand (#314).
pub fn refit_due(stored: Option<&str>, now: f64) -> bool {
    match stored.and_then(|j| serde_json::from_str::<FoldCalibration>(j).ok()) {
        // A fit stored before the preflop street existed (2026-09-23) is refitted at once.
        Some(cal) => now - cal.fitted_at > STALE_AFTER_SECS || cal.preflop.n == 0,
        None => true,
    }
}

/// Log-odds with the fits' clamp (`sv10_stats::logistic`, 0231).
pub(crate) fn logit(p: f64) -> f64 {
    sv10_stats::logistic::logit(p, 1e-4)
}

fn loss(raw: f64, shift: f64, folded: bool) -> f64 {
    sv10_stats::logistic::log_loss(sv10_stats::logistic::sigmoid(logit(raw) + shift), folded, 1e-9)
}

/// Maximum-likelihood logit shift (golden-section search on the convex mean log-loss).
fn best_shift(samples: &[&FoldSample], max: f64) -> f64 {
    let total = |s: f64| samples.iter().map(|x| loss(x.raw, s, x.folded)).sum::<f64>();
    sv10_stats::optimize::golden_section_min(total, -max, max, 60)
}

/// Fit and gate every street; samples need not be sorted.
pub fn fit(samples: &[FoldSample], now: f64) -> FoldCalibration {
    let mut out = FoldCalibration { fitted_at: now, ..FoldCalibration::default() };
    out.preflop = fit_street(samples, PREFLOP, PREFLOP_MAX_SHIFT);
    out.preflop_shift = out.preflop.shift;
    for street in 0..3 {
        out.streets[street] = fit_street(samples, street, MAX_SHIFT);
        out.shift[street] = out.streets[street].shift;
    }
    out
}

/// Fit and gate one street: the shift fitted on the older half is kept only when it improves
/// log-loss on the newer half with a positive 95% lower bound, then refitted on every sample.
fn fit_street(samples: &[FoldSample], street: usize, max_shift: f64) -> StreetFit {
    let mut fit = StreetFit::default();
    {
        let mut v: Vec<&FoldSample> = samples.iter().filter(|s| s.street == street && s.raw.is_finite()).collect();
        v.sort_by(|a, b| a.ts.cmp(&b.ts));
        if v.len() > MAX_SAMPLES {
            v.drain(..v.len() - MAX_SAMPLES);
        }
        let n = v.len();
        fit.n = n;
        if n == 0 {
            return fit;
        }
        fit.predicted = v.iter().map(|s| s.raw).sum::<f64>() / n as f64;
        fit.actual = v.iter().filter(|s| s.folded).count() as f64 / n as f64;
        let (train, test) = v.split_at(n / 2);
        if test.len() < MIN_HELD_OUT {
            return fit;
        }
        fit.train_shift = best_shift(train, max_shift);
        let gains: Vec<f64> = test.iter().map(|s| loss(s.raw, 0.0, s.folded) - loss(s.raw, fit.train_shift, s.folded)).collect();
        let m = gains.iter().sum::<f64>() / gains.len() as f64;
        let var = gains.iter().map(|g| (g - m).powi(2)).sum::<f64>() / (gains.len() - 1) as f64;
        fit.held_out_gain = m;
        fit.held_out_lower = m - 1.96 * (var / gains.len() as f64).sqrt();
        fit.active = fit.held_out_lower > 0.0;
        if fit.active {
            fit.shift = best_shift(&v, max_shift);
        }
    }
    fit
}

/// One time slice of a street, scored with the shift fitted on everything before it.
#[derive(Clone, Debug, PartialEq)]
pub struct DriftSlice {
    /// Samples in the slice.
    pub n: usize,
    /// Mean predicted fold rate before any shift.
    pub predicted: f64,
    /// Realized fold rate.
    pub actual: f64,
    /// Shift fitted on the slices before this one.
    pub shift: f64,
    /// Log-loss gain of that shift on this slice, nats per sample.
    pub gain: f64,
    /// 95% half-width of the gain.
    pub half_width: f64,
    /// Gain of the shift fitted on the previous slice alone: what a recency-weighted fit would have earned.
    pub recent_gain: f64,
}

/// A street's samples in time order, cut into `slices` equal parts; each part after the first is scored with the shift
/// fitted on all earlier parts (#767). A street whose gain is negative in every slice is mis-shaped (a constant shift
/// does not describe it); one that is positive in some and negative in others drifted between the halves the gate
/// compares. Read-only, for `review fold-cal --split`; nothing here is installed.
pub fn drift(samples: &[FoldSample], street: usize, slices: usize) -> Vec<DriftSlice> {
    let max_shift = if street == PREFLOP { PREFLOP_MAX_SHIFT } else { MAX_SHIFT };
    let mut v: Vec<&FoldSample> = samples.iter().filter(|s| s.street == street && s.raw.is_finite()).collect();
    v.sort_by(|a, b| a.ts.cmp(&b.ts));
    if v.len() > MAX_SAMPLES {
        v.drain(..v.len() - MAX_SAMPLES);
    }
    let size = v.len() / slices.max(2);
    let mut out = Vec::new();
    for k in 1..slices.max(2) {
        let (before, this) = (&v[..k * size], &v[k * size..((k + 1) * size).min(v.len())]);
        if before.is_empty() || this.len() < 2 {
            continue;
        }
        let shift = best_shift(before, max_shift);
        let recent = best_shift(&before[before.len().saturating_sub(size)..], max_shift);
        let mean_gain =
            |sh: f64| this.iter().map(|s| loss(s.raw, 0.0, s.folded) - loss(s.raw, sh, s.folded)).sum::<f64>() / this.len() as f64;
        let gains: Vec<f64> = this.iter().map(|s| loss(s.raw, 0.0, s.folded) - loss(s.raw, shift, s.folded)).collect();
        let n = this.len() as f64;
        let gain = gains.iter().sum::<f64>() / n;
        let var = gains.iter().map(|g| (g - gain).powi(2)).sum::<f64>() / (n - 1.0);
        out.push(DriftSlice {
            n: this.len(),
            predicted: this.iter().map(|s| s.raw).sum::<f64>() / n,
            actual: this.iter().filter(|s| s.folded).count() as f64 / n,
            shift,
            gain,
            half_width: 1.96 * (var / n).sqrt(),
            recent_gain: mean_gain(recent),
        });
    }
    out
}

/// The `review fold-cal --split` report: four time slices per street, each scored with the shift fitted before it.
pub fn drift_lines(samples: &[FoldSample]) -> Vec<String> {
    let mut out = Vec::new();
    for (street, name) in [(0, "flop"), (1, "turn"), (2, "river"), (PREFLOP, "preflop")] {
        for (k, d) in drift(samples, street, 4).iter().enumerate() {
            out.push(format!(
                "{name:7} slice {} n {:6}  predicted {:.3}  actual {:.3}  earlier-fit shift {:+.2}  gain {:+6.1} ± {:4.1} mnats  (previous slice alone: {:+6.1})",
                k + 2,
                d.n,
                d.predicted,
                d.actual,
                d.shift,
                d.gain * 1000.0,
                d.half_width * 1000.0,
                d.recent_gain * 1000.0
            ));
        }
    }
    out
}

/// Heads-up postflop bets from the store: each recorded bet or raise into no bet with one
/// opponent, the chosen candidate's fold estimate un-shifted by the shift recorded with the
/// decision, and whether that opponent folded next on the same street.
pub fn samples_from_store(store: &Store) -> anyhow::Result<Vec<FoldSample>> {
    let mut out = Vec::new();
    let mut hands: std::collections::HashMap<(String, String), Option<serde_json::Value>> = std::collections::HashMap::new();
    for bet in store.heads_up_postflop_bets(MAX_SAMPLES * 3)? {
        let Some(street) = ["flop", "turn", "river"].iter().position(|s| *s == bet.street) else { continue };
        let Ok(detail) = serde_json::from_str::<serde_json::Value>(&bet.detail) else { continue };
        if detail["opponents"].as_u64() != Some(1) {
            continue;
        }
        let Some(fp) = detail["candidates"].as_array().and_then(|cs| {
            cs.iter()
                .find(|c| matches!(c["action"].as_str(), Some("raise" | "all_in")) && c["amount"].as_i64() == bet.amount)
                .and_then(|c| c["fold_prob"].as_f64())
        }) else {
            continue;
        };
        let recorded = detail["fold_shift"].as_array().and_then(|a| a.get(street)).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let raw = if recorded == 0.0 || fp <= 0.0 { fp } else { 1.0 / (1.0 + (-(logit(fp) - recorded)).exp()) };
        let key = (bet.bot.clone(), bet.hand_id.clone());
        let summary = hands
            .entry(key)
            .or_insert_with(|| store.hand(&bet.bot, &bet.hand_id).ok().flatten().and_then(|h| serde_json::from_str(&h.summary).ok()));
        let Some((opponent, folded)) = summary.as_ref().and_then(|s| opponent_response(s, &bet.bot, street)) else { continue };
        out.push(FoldSample { ts: bet.ts, street, raw, folded, opponent: Some(opponent) });
    }
    Ok(out)
}

/// Our preflop raises from the store: the chosen candidate's everyone-folds estimate un-shifted by
/// the preflop shift recorded with the decision, and whether every opponent folded to it.
pub fn preflop_samples_from_store(store: &Store) -> anyhow::Result<Vec<FoldSample>> {
    let mut out = Vec::new();
    let mut hands: std::collections::HashMap<(String, String), Option<serde_json::Value>> = std::collections::HashMap::new();
    for bet in store.preflop_raises(MAX_SAMPLES * 2)? {
        let Ok(detail) = serde_json::from_str::<serde_json::Value>(&bet.detail) else { continue };
        let is_all_in = bet.action == "all_in";
        let Some(fp) = detail["candidates"].as_array().and_then(|cs| {
            cs.iter()
                .find(|c| c["action"].as_str() == Some(bet.action.as_str()) && (is_all_in || c["amount"].as_i64() == bet.amount))
                .and_then(|c| c["fold_prob"].as_f64())
        }) else {
            continue;
        };
        let recorded = detail["preflop_fold_shift"].as_f64().unwrap_or(0.0);
        let raw = if recorded == 0.0 || fp <= 0.0 { fp } else { 1.0 / (1.0 + (-(logit(fp) - recorded)).exp()) };
        let key = (bet.bot.clone(), bet.hand_id.clone());
        let summary = hands
            .entry(key)
            .or_insert_with(|| store.hand(&bet.bot, &bet.hand_id).ok().flatten().and_then(|h| serde_json::from_str(&h.summary).ok()));
        let Some(folded) = summary.as_ref().and_then(|s| everyone_folded_preflop(s, &bet.bot, bet.amount, is_all_in)) else { continue };
        out.push(FoldSample { ts: bet.ts, street: PREFLOP, raw, folded, opponent: None });
    }
    Ok(out)
}

/// Whether every opponent who acted after our preflop raise to `to` folded before our next action.
fn everyone_folded_preflop(summary: &serde_json::Value, hero: &str, to: Option<i64>, all_in: bool) -> Option<bool> {
    let seat = summary["players"].as_array()?.iter().find(|p| p[1].as_str() == Some(hero))?[0].as_i64()?;
    let history = summary["history"].as_array()?;
    let at = history.iter().position(|a| {
        a["seat"].as_i64() == Some(seat)
            && a["street"].as_str() == Some("Preflop")
            && matches!(a["kind"].as_str(), Some("Raise" | "AllIn"))
            && (all_in || a["to"].as_i64() == to)
    })?;
    let after: Vec<&serde_json::Value> =
        history[at + 1..].iter().take_while(|a| a["seat"].as_i64() != Some(seat) && a["street"].as_str() == Some("Preflop")).collect();
    if after.is_empty() {
        return None;
    }
    Some(after.iter().all(|a| a["kind"].as_str() == Some("Fold")))
}

/// Whether the first opponent to act after our first bet into no bet on `street` folded.
#[cfg(test)]
fn opponent_folded(summary: &serde_json::Value, hero: &str, street: usize) -> Option<bool> {
    opponent_response(summary, hero, street).map(|(_, folded)| folded)
}

/// Who answered our first bet into no bet on `street` (0 flop .. 2 river), and whether they folded.
fn opponent_response(summary: &serde_json::Value, hero: &str, street: usize) -> Option<(String, bool)> {
    let seat = summary["players"].as_array()?.iter().find(|p| p[1].as_str() == Some(hero))?[0].as_i64()?;
    let name = ["Flop", "Turn", "River"][street];
    let history = summary["history"].as_array()?;
    let at = history.iter().position(|a| {
        a["seat"].as_i64() == Some(seat)
            && a["street"].as_str() == Some(name)
            && matches!(a["kind"].as_str(), Some("Raise" | "AllIn"))
            && a["to_call_before"].as_i64() == Some(0)
    })?;
    let next = history[at + 1..].iter().find(|a| a["seat"].as_i64() != Some(seat) && a["street"].as_str() == Some(name))?;
    let who = next["seat"].as_i64()?;
    let opponent = summary["players"].as_array()?.iter().find(|p| p[0].as_i64() == Some(who))?[1].as_str()?.to_string();
    Some((opponent, next["kind"].as_str() == Some("Fold")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn samples(street: usize, n: usize, raw: f64, fold_rate: f64) -> Vec<FoldSample> {
        // Deterministic outcomes at the requested rate, interleaved over time.
        let every = (1.0 / fold_rate.max(1e-9)).round() as usize;
        (0..n)
            .map(|i| FoldSample {
                ts: format!("2026-09-{:02}T{:05}", 14 + i * 7 / n, i),
                street,
                raw,
                folded: i % every == 0,
                opponent: None,
            })
            .collect()
    }

    #[test]
    fn a_consistent_over_prediction_is_fitted_and_installed() {
        // Predicted 30%, realized 20% on the river: the shift must pull folds down toward 20%.
        let cal = fit(&samples(2, 2_000, 0.30, 0.20), 1.0);
        let river = &cal.streets[2];
        assert!(river.active, "{river:?}");
        assert!(cal.shift[2] < -0.4 && cal.shift[2] > -0.7, "shift {}", cal.shift[2]);
        let shifted = 1.0 / (1.0 + (-(logit(0.30) + cal.shift[2])).exp());
        assert!((shifted - 0.20).abs() < 0.01, "shifted prediction {shifted}");
        assert_eq!((cal.shift[0], cal.shift[1]), (0.0, 0.0), "streets without data stay unshifted");
    }

    #[test]
    fn a_calibrated_or_thin_street_is_left_alone() {
        let calibrated = fit(&samples(0, 4_000, 0.25, 0.25), 1.0);
        assert!(!calibrated.streets[0].active, "{:?}", calibrated.streets[0]);
        assert_eq!(calibrated.shift, [0.0; 3]);
        let thin = fit(&samples(1, 300, 0.30, 0.10), 1.0);
        assert!(!thin.streets[1].active, "150 held-out samples are below the minimum");
    }

    #[test]
    fn preflop_over_prediction_is_fitted_on_its_own_street() {
        // 2026-09-23: predicted 0.55 that everyone folds, realized 0.25.
        let cal = fit(&samples(PREFLOP, 4_000, 0.55, 0.25), 1.0);
        assert!(cal.preflop.active, "{:?}", cal.preflop);
        let shifted = 1.0 / (1.0 + (-(logit(0.55) + cal.preflop_shift)).exp());
        assert!((shifted - 0.25).abs() < 0.01, "shifted {shifted}");
        assert!(cal.preflop_shift < -1.0, "wider than the postflop cap: {}", cal.preflop_shift);
        assert_eq!(cal.shift, [0.0; 3], "postflop streets untouched");
        assert!((installed_preflop_shift(Some(&serde_json::to_string(&cal).unwrap())) - cal.preflop_shift).abs() < 1e-12);
        // Older stored calibrations without the field read as no shift.
        assert_eq!(installed_preflop_shift(Some(r#"{"shift":[0,0,0],"streets":[{},{},{}],"fitted_at":1}"#)), 0.0);
    }

    #[test]
    fn everyone_folding_to_our_preflop_raise_is_the_outcome() {
        let hand = json!({"players": [[0, "Hero"], [1, "A"], [2, "B"]], "history": [
            {"seat": 0, "street": "Preflop", "kind": "Raise", "to": 60},
            {"seat": 1, "street": "Preflop", "kind": "Fold", "to": 0},
            {"seat": 2, "street": "Preflop", "kind": "Fold", "to": 0}]});
        assert_eq!(everyone_folded_preflop(&hand, "Hero", Some(60), false), Some(true));
        let called = json!({"players": [[0, "Hero"], [1, "A"], [2, "B"]], "history": [
            {"seat": 0, "street": "Preflop", "kind": "Raise", "to": 60},
            {"seat": 1, "street": "Preflop", "kind": "Fold", "to": 0},
            {"seat": 2, "street": "Preflop", "kind": "Call", "to": 60},
            {"seat": 0, "street": "Flop", "kind": "Check", "to": 0}]});
        assert_eq!(everyone_folded_preflop(&called, "Hero", Some(60), false), Some(false));
        assert_eq!(everyone_folded_preflop(&called, "Hero", Some(90), false), None, "no raise to 90");
    }

    #[test]
    fn drift_separates_a_street_that_changed_from_one_a_shift_cannot_describe() {
        // Folds were under-predicted early and over-predicted late: each slice is scored with the earlier shift.
        let mut early = samples(1, 2_000, 0.20, 0.35);
        let mut late = samples(1, 2_000, 0.20, 0.08);
        for (i, s) in early.iter_mut().enumerate() {
            s.ts = format!("2026-09-01T00:00:{:02}.{i:06}Z", i % 60);
        }
        for (i, s) in late.iter_mut().enumerate() {
            s.ts = format!("2026-10-01T00:00:{:02}.{i:06}Z", i % 60);
        }
        early.extend(late);
        let slices = drift(&early, 1, 2);
        assert_eq!(slices.len(), 1, "two slices score one: the second against the first");
        assert!(slices[0].gain < 0.0, "a shift fitted on the early half hurts the late half: {:?}", slices[0]);
        assert!(slices[0].shift > 0.0, "the early half wanted more folds: {:?}", slices[0]);
        assert!((slices[0].actual - 0.08).abs() < 0.01);
        assert!(drift(&early, 0, 4).is_empty(), "a street with no samples has no slices");
        let steady = drift(&samples(2, 4_000, 0.30, 0.10), 2, 4);
        assert_eq!(steady.len(), 3);
        assert!(steady.iter().all(|d| d.gain > 0.0 && d.shift < 0.0), "a stationary miss is fixed by one shift: {steady:?}");
    }

    #[test]
    fn installed_shift_reads_the_store_value_or_zero() {
        let cal = FoldCalibration { shift: [0.2, 0.0, -0.7], ..FoldCalibration::default() };
        assert_eq!(installed_shift(Some(&serde_json::to_string(&cal).unwrap())), [0.2, 0.0, -0.7]);
        assert_eq!(installed_shift(None), [0.0; 3]);
        assert_eq!(installed_shift(Some("not json")), [0.0; 3]);
    }

    #[test]
    fn a_missing_stale_or_unreadable_fit_is_due_at_learner_start() {
        let fitted = |at: f64| serde_json::to_string(&FoldCalibration { fitted_at: at, ..FoldCalibration::default() }).unwrap();
        assert!(refit_due(None, 10_000.0), "nothing stored: fit now instead of a cycle later");
        assert!(refit_due(Some("not json"), 10_000.0));
        assert!(refit_due(Some(&fitted(10_000.0 - 3_601.0)), 10_000.0), "older than an hour");
        let fresh = |at: f64| {
            serde_json::to_string(&FoldCalibration {
                fitted_at: at,
                preflop: StreetFit { n: 5, ..StreetFit::default() },
                ..FoldCalibration::default()
            })
            .unwrap()
        };
        assert!(!refit_due(Some(&fresh(10_000.0 - 600.0)), 10_000.0), "a fresh fit is kept");
        assert!(refit_due(Some(&fitted(10_000.0 - 600.0)), 10_000.0), "a fresh fit without preflop samples is refitted");
    }

    #[test]
    fn the_opponents_next_action_on_the_street_is_the_outcome() {
        let hand = json!({"players": [[1, "Hero"], [3, "V"]], "history": [
            {"seat": 1, "street": "Flop", "kind": "Raise", "to_call_before": 0},
            {"seat": 3, "street": "Flop", "kind": "Call", "to_call_before": 50},
            {"seat": 1, "street": "River", "kind": "Raise", "to_call_before": 0},
            {"seat": 3, "street": "River", "kind": "Fold", "to_call_before": 90}]});
        assert_eq!(opponent_folded(&hand, "Hero", 0), Some(false));
        assert_eq!(opponent_folded(&hand, "Hero", 2), Some(true));
        assert_eq!(opponent_folded(&hand, "Hero", 1), None, "no turn bet");
        assert_eq!(opponent_folded(&hand, "Other", 0), None);
        assert_eq!(opponent_response(&hand, "Hero", 2), Some(("V".to_string(), true)), "names who answered");
    }
}

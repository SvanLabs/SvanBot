//! Per-opponent fold calibration (0214): how often each opponent folds to our heads-up postflop
//! bets compared with the (street-calibrated) fold estimate the decision priced them with.
//!
//! The fold estimate drives the EV of every bet and raise. The street calibration ([`crate::foldcal`])
//! corrects it for everyone at once; an opponent who folds more or less than the model predicts is
//! still mispriced hand after hand. Each opponent gets a logit offset, one Newton step of a
//! logistic fit with a Gaussian prior: `Σ(folded − p) / (Σ p(1 − p) + prior)`, so a player with
//! few samples barely moves. It is installed only while it lowers held-out log-loss at 95%, and the
//! policy applies it to heads-up postflop bets only, the spots it was measured in.

use crate::foldcal::{FoldSample, PREFLOP, logit};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use sv10_store::store::Store;

/// Kv key of the stored fit ([`PlayerFoldFit`]).
pub const PLAYER_FOLD_KEY: &str = "player_fold.v1";
/// Largest offset in logits: extrapolation guard, not the model (LESSONS 29).
pub const MAX_OFFSET: f64 = 1.5;
/// Fewest held-out samples the gate judges on.
const MIN_HELD_OUT: usize = 1_000;

use sv10_stats::logistic::sigmoid;

fn log_loss(p: f64, y: bool) -> f64 {
    sv10_stats::logistic::log_loss(p, y, 1e-6)
}

/// Running residuals of one opponent.
#[derive(Clone, Copy, Debug, Default)]
struct Tally {
    residual: f64,
    weight: f64,
}

impl Tally {
    fn offset(&self, prior: f64) -> f64 {
        (self.residual / (self.weight + prior)).clamp(-MAX_OFFSET, MAX_OFFSET)
    }
    fn add(&mut self, p: f64, folded: bool) {
        self.residual += f64::from(u8::from(folded)) - p;
        self.weight += p * (1.0 - p);
    }
}

/// Held-out result of one prior: mean log-loss gain per sample (nats) and its 95% half-width.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerFoldScore {
    /// Prior weight (pseudo-samples at the model's prediction).
    pub prior: f64,
    /// Held-out samples.
    pub n: usize,
    /// Mean held-out log-loss of the street-calibrated estimate.
    pub base_loss: f64,
    /// Mean gain of adding the per-opponent offset.
    pub gain: f64,
    /// 95% half-width of `gain`.
    pub half_width: f64,
}

/// The street-calibrated fold estimate of a sample.
fn base(s: &FoldSample, street_shift: &[f64; 3]) -> f64 {
    sigmoid(logit(s.raw) + street_shift[s.street])
}

/// Score each prior on postflop `samples` (oldest first): street shifts fitted on the older half,
/// offsets learned online from each opponent's earlier samples, scored on the newer half.
pub fn score(samples: &[FoldSample], priors: &[f64]) -> Vec<PlayerFoldScore> {
    let post: Vec<&FoldSample> = samples.iter().filter(|s| s.street != PREFLOP && s.opponent.is_some()).collect();
    let split = post.len() / 2;
    let train: Vec<FoldSample> = post[..split].iter().map(|s| (*s).clone()).collect();
    let street_shift = crate::foldcal::fit(&train, 0.0).shift;
    let mut tallies: Vec<HashMap<&str, Tally>> = vec![HashMap::new(); priors.len()];
    let mut gains: Vec<Vec<f64>> = vec![Vec::new(); priors.len()];
    let mut base_losses = Vec::new();
    for (i, s) in post.iter().enumerate() {
        let who = s.opponent.as_deref().unwrap_or_default();
        let p0 = base(s, &street_shift);
        if i >= split {
            let b = log_loss(p0, s.folded);
            base_losses.push(b);
            for (k, &prior) in priors.iter().enumerate() {
                let off = tallies[k].get(who).map_or(0.0, |t| t.offset(prior));
                gains[k].push(b - log_loss(sigmoid(logit(p0) + off), s.folded));
            }
        }
        for t in tallies.iter_mut() {
            t.entry(who).or_default().add(p0, s.folded);
        }
    }
    let base_loss = base_losses.iter().sum::<f64>() / base_losses.len().max(1) as f64;
    priors
        .iter()
        .zip(gains)
        .map(|(&prior, g)| {
            let n = g.len();
            let m = g.iter().sum::<f64>() / n.max(1) as f64;
            let v = if n > 1 { g.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1) as f64 } else { f64::INFINITY };
            PlayerFoldScore { prior, n, base_loss, gain: m, half_width: 1.96 * (v / n.max(1) as f64).sqrt() }
        })
        .collect()
}

/// Prior of the installed offsets.
pub const PRIOR: f64 = 20.0;

/// The stored fit: every opponent's offset against the installed street shifts, and its evidence.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlayerFoldFit {
    /// Held-out gain (nats per sample) and its 95% half-width at [`PRIOR`].
    pub gain: f64,
    /// 95% half-width of `gain`.
    pub half_width: f64,
    /// Held-out samples.
    pub n: usize,
    /// Whether the offsets are installed.
    pub active: bool,
    /// Logit offset per opponent.
    pub offsets: HashMap<String, f64>,
}

/// Fit from `samples` against the street shifts in play (`street_shift`).
pub fn fit(samples: &[FoldSample], street_shift: &[f64; 3]) -> PlayerFoldFit {
    let s = score(samples, &[PRIOR]).remove(0);
    let mut tallies: HashMap<&str, Tally> = HashMap::new();
    for x in samples.iter().filter(|x| x.street != PREFLOP) {
        if let Some(who) = x.opponent.as_deref() {
            tallies.entry(who).or_default().add(base(x, street_shift), x.folded);
        }
    }
    let active = s.n >= MIN_HELD_OUT && s.gain - s.half_width > 0.0;
    PlayerFoldFit {
        gain: s.gain,
        half_width: s.half_width,
        n: s.n,
        active,
        offsets: tallies.into_iter().map(|(k, t)| (k.to_string(), t.offset(PRIOR))).filter(|(_, o)| o.abs() > 1e-3).collect(),
    }
}

/// The offsets to play with from the stored fit (none unless active).
pub fn installed(stored: Option<&str>) -> std::sync::Arc<HashMap<String, f32>> {
    let fit = stored.and_then(|j| serde_json::from_str::<PlayerFoldFit>(j).ok()).filter(|f| f.active);
    std::sync::Arc::new(fit.map(|f| f.offsets.into_iter().map(|(k, v)| (k, v as f32)).collect()).unwrap_or_default())
}

/// Refit and store (the learner, next to the street calibration).
pub fn refit(store: &Store) {
    let t0 = std::time::Instant::now();
    let samples = match crate::foldcal::samples_from_store(store) {
        Ok(mut s) => {
            s.sort_by(|a, b| a.ts.cmp(&b.ts));
            s
        }
        Err(e) => return tracing::warn!("per-opponent fold samples unreadable: {e}"),
    };
    let shift = crate::foldcal::installed_shift(store.get_kv(crate::foldcal::FOLD_CAL_KEY).ok().flatten().as_deref());
    let f = fit(&samples, &shift);
    tracing::info!(
        "per-opponent fold calibration: {} opponents; held-out gain {:+.2} ± {:.2} mnats on {} bets -> {} ({:.1}s)",
        f.offsets.len(),
        f.gain * 1000.0,
        f.half_width * 1000.0,
        f.n,
        if f.active { "installed" } else { "not installed" },
        t0.elapsed().as_secs_f64()
    );
    match serde_json::to_string(&f) {
        Ok(j) => {
            if let Err(e) = store.put_kv(PLAYER_FOLD_KEY, &j) {
                tracing::warn!("per-opponent fold calibration not stored: {e}");
            }
        }
        Err(e) => tracing::warn!("per-opponent fold calibration not serialized: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(i: usize, who: &str, raw: f64, folded: bool) -> FoldSample {
        FoldSample { ts: format!("2026-09-24T{i:06}"), street: i % 3, raw, folded, opponent: Some(who.to_string()) }
    }

    #[test]
    fn an_opponent_who_folds_more_than_predicted_gets_a_positive_offset_that_helps() {
        // The model says 40% for both; the nit folds 80%, the station 5%.
        let samples: Vec<FoldSample> = (0..4_000)
            .map(|i| if i % 2 == 0 { sample(i, "nit", 0.4, i % 10 != 0) } else { sample(i, "station", 0.4, i % 20 == 1) })
            .collect();
        let scores = score(&samples, &[5.0, 20.0, 100.0]);
        for s in &scores {
            assert_eq!(s.n, 2_000);
            assert!(s.gain > 0.05 && s.gain - s.half_width > 0.0, "{s:?}");
        }
        let f = fit(&samples, &[0.0; 3]);
        assert!(f.active);
        assert!(f.offsets["nit"] > 0.8 && f.offsets["station"] < -0.8, "{:?}", f.offsets);
        let json = serde_json::to_string(&f).unwrap();
        assert!((installed(Some(&json))["nit"] as f64 - f.offsets["nit"]).abs() < 1e-6);
        assert!(installed(Some(&serde_json::to_string(&PlayerFoldFit { active: false, ..f }).unwrap())).is_empty());
        assert!(installed(None).is_empty() && installed(Some("junk")).is_empty());
    }

    #[test]
    fn calibrated_opponents_get_no_offset_and_no_install() {
        // Everyone folds at the predicted 50%: nothing to learn.
        let samples: Vec<FoldSample> = (0..4_000).map(|i| sample(i, ["a", "b", "c"][i % 3], 0.5, (i / 3) % 2 == 0)).collect();
        let f = fit(&samples, &[0.0; 3]);
        assert!(!f.active, "{f:?}");
        assert!(f.offsets.values().all(|o| o.abs() < 0.1), "{:?}", f.offsets);
    }

    #[test]
    fn a_few_samples_barely_move_an_offset_and_it_is_bounded() {
        let mut t = Tally::default();
        for _ in 0..3 {
            t.add(0.5, true);
        }
        assert!(t.offset(PRIOR) > 0.0 && t.offset(PRIOR) < 0.2);
        let mut big = Tally::default();
        for _ in 0..100_000 {
            big.add(0.01, true);
        }
        assert_eq!(big.offset(PRIOR), MAX_OFFSET);
    }
}

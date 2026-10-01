//! Per-opponent residual study for the response model (0210): does correcting the live network's
//! predictions by each opponent's own observed/expected ratios predict held-out decisions better?
//!
//! Live hands are replayed in time order exactly as training builds its samples (profiles known
//! before each hand, warmed by past-season hands). Every decision is predicted by the network, then
//! by the network corrected with a [`ResidualTable`] fed only earlier decisions, and both are scored
//! by log-loss on the validation hands (the newest 15%, as the training gate uses).

use crate::neural::{VALIDATION_SPLIT_PERCENT, live_hands, warm_history};
use sv10_core::features::{ResponseFeatureSet, named_samples_from_hand_for};
use sv10_core::model::ModelStore;
use sv10_core::nn::{Mlp, Sample, log_loss};
use sv10_core::residual::ResidualTable;
use sv10_store::store::Store;

/// Held-out gain of one shrinkage prior, nats per decision (positive = the correction predicts better).
#[derive(Clone, Debug, PartialEq)]
pub struct ResidualScore {
    /// Pseudo-observations per class the ratios are shrunk with.
    pub prior: f32,
    /// Validation decisions scored, and those facing a bet (where the fleet reads the network).
    pub n: usize,
    /// Decisions facing a bet among `n`.
    pub n_facing: usize,
    /// Mean network log-loss on all validation decisions.
    pub base_loss: f64,
    /// Mean log-loss gain on all validation decisions and its 95% half-width.
    pub gain: f64,
    /// 95% half-width of `gain`.
    pub half_width: f64,
    /// Mean gain on decisions facing a bet and its 95% half-width.
    pub gain_facing: f64,
    /// 95% half-width of `gain_facing`.
    pub half_width_facing: f64,
    /// Mean gain on the older and newer halves of the validation hands (stability check).
    pub gain_halves: [f64; 2],
}

fn mean_hw(xs: &[f64]) -> (f64, f64) {
    sv10_stats::moments::mean_half_width(xs, 1.96)
}

/// Score each prior on `hands` (per hand, oldest first: responder name and sample), validating on
/// hands from index `split` on. The tables learn from every hand, validation included, but only
/// after the hand is scored.
pub fn score(hands: &[Vec<(String, Sample)>], net: &Mlp, split: usize, priors: &[f32]) -> Vec<ResidualScore> {
    let mut tables: Vec<ResidualTable> = priors.iter().map(|&k| ResidualTable::new(k)).collect();
    let mut base = Vec::new();
    let mut gains: Vec<Vec<(f64, bool, usize)>> = vec![Vec::new(); priors.len()];
    let mid = split + (hands.len().saturating_sub(split)) / 2;
    for (i, hand) in hands.iter().enumerate() {
        let predicted: Vec<Vec<f32>> = hand.iter().map(|(_, s)| net.predict(&s.x, &s.mask)).collect();
        if i >= split {
            for ((name, s), p) in hand.iter().zip(&predicted) {
                let facing = s.x[4] > 0.5;
                let b = log_loss(p, s.label);
                base.push(b);
                for (t, g) in tables.iter().zip(gains.iter_mut()) {
                    g.push((b - log_loss(&t.adjust(name, facing, p, &s.mask), s.label), facing, usize::from(i >= mid)));
                }
            }
        }
        for ((name, s), p) in hand.iter().zip(&predicted) {
            for t in tables.iter_mut() {
                t.observe(name, s.x[4] > 0.5, p, &s.mask, s.label);
            }
        }
    }
    let base_loss = base.iter().sum::<f64>() / base.len().max(1) as f64;
    priors
        .iter()
        .zip(gains)
        .map(|(&prior, g)| {
            let all: Vec<f64> = g.iter().map(|x| x.0).collect();
            let facing: Vec<f64> = g.iter().filter(|x| x.1).map(|x| x.0).collect();
            let half = |h: usize| {
                let xs: Vec<f64> = g.iter().filter(|x| x.2 == h).map(|x| x.0).collect();
                mean_hw(&xs).0
            };
            let (gain, half_width) = mean_hw(&all);
            let (gain_facing, half_width_facing) = mean_hw(&facing);
            ResidualScore {
                prior,
                n: all.len(),
                n_facing: facing.len(),
                base_loss,
                gain,
                half_width,
                gain_facing,
                half_width_facing,
                gain_halves: [half(0), half(1)],
            }
        })
        .collect()
}

/// Kv key of the stored residual fit ([`ResidualFit`]).
pub const NN_RESIDUAL_KEY: &str = "nn_residual.v1";

/// Shrinkage prior of the installed correction: within 0.05 mnats of the best on 2026-09-24
/// (+1.64 ± 0.30 vs +1.69 ± 0.33 at 10) with less variance.
pub const PRIOR: f32 = 30.0;

/// The learner's residual fit: the per-opponent ratios against one network and whether they help.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ResidualFit {
    /// `trained_at` of the network the ratios were fitted against; they apply only to it.
    pub net_trained_at: f64,
    /// Held-out gain facing a bet, nats per decision, and its 95% half-width.
    pub gain_facing: f64,
    /// 95% half-width of `gain_facing`.
    pub half_width_facing: f64,
    /// Validation decisions facing a bet.
    pub n_facing: usize,
    /// Whether the correction is installed: the held-out gain facing a bet is positive at 95%.
    pub active: bool,
    /// Every opponent's ratios facing a bet (fold, call, raise), over all stored live hands.
    pub ratios: std::collections::HashMap<String, [f32; 3]>,
}

/// The correction the fleet plays with: the stored fit's ratios when it is active and was fitted
/// against the live network (`live_net_trained_at`), otherwise none.
pub fn installed(stored: Option<&str>, live_net_trained_at: Option<f64>) -> std::sync::Arc<std::collections::HashMap<String, [f32; 3]>> {
    let fit = stored.and_then(|j| serde_json::from_str::<ResidualFit>(j).ok());
    match (fit, live_net_trained_at) {
        (Some(f), Some(t)) if f.active && f.net_trained_at == t => std::sync::Arc::new(f.ratios),
        _ => Default::default(),
    }
}

/// The installed correction read from the store (live network and residual fit).
pub fn installed_from_store(store: &Store) -> anyhow::Result<std::sync::Arc<std::collections::HashMap<String, [f32; 3]>>> {
    let net = crate::installs::read_checked::<crate::StoredNet>(store, crate::NN_KEY)?;
    // read_checked validated this JSON; retain Result rather than assuming parsing is infallible.
    let live = net
        .as_deref()
        .map(serde_json::from_str::<crate::StoredNet>)
        .transpose()?
        .filter(crate::neural::response_net_is_eligible)
        .map(|n| n.trained_at);
    let residual = crate::installs::read_checked::<ResidualFit>(store, NN_RESIDUAL_KEY)?;
    Ok(installed(residual.as_deref(), live))
}

/// Fit the correction against the stored live network and store it (the learner, hourly).
pub fn refit(store: &Store, store_dir: &std::path::Path) {
    let t0 = std::time::Instant::now();
    match fit(store, store_dir) {
        Ok(f) => {
            tracing::info!(
                "per-opponent response correction: {} opponents; held-out gain facing a bet {:+.2} ± {:.2} mnats on {} decisions -> {} ({:.1}s)",
                f.ratios.len(),
                f.gain_facing * 1000.0,
                f.half_width_facing * 1000.0,
                f.n_facing,
                if f.active { "installed" } else { "not installed" },
                t0.elapsed().as_secs_f64()
            );
            match serde_json::to_string(&f) {
                Ok(j) => {
                    if let Err(e) = store.put_kv(NN_RESIDUAL_KEY, &j) {
                        tracing::warn!("per-opponent response correction not stored: {e}");
                    }
                }
                Err(e) => tracing::warn!("per-opponent response correction not serialized: {e}"),
            }
        }
        Err(e) => tracing::warn!("per-opponent response correction not fitted: {e}"),
    }
}

/// The fit: the study at [`PRIOR`] plus the ratios over every stored live hand.
pub fn fit(store: &Store, store_dir: &std::path::Path) -> anyhow::Result<ResidualFit> {
    let (net, samples, split) = load(store, store_dir)?;
    let score = score(&samples, &net.net, split, &[PRIOR]).remove(0);
    let mut table = ResidualTable::new(PRIOR);
    for (name, s) in samples.iter().flatten() {
        table.observe(name, s.x[4] > 0.5, &net.net.predict(&s.x, &s.mask), &s.mask, s.label);
    }
    let active = score.n_facing >= MIN_FACING && score.gain_facing - score.half_width_facing > 0.0;
    Ok(ResidualFit {
        net_trained_at: net.trained_at,
        gain_facing: score.gain_facing,
        half_width_facing: score.half_width_facing,
        n_facing: score.n_facing,
        active,
        ratios: table.facing_ratios(),
    })
}

/// Fewest held-out decisions facing a bet the gate judges on.
const MIN_FACING: usize = 1_000;

type Loaded = (crate::StoredNet, Vec<Vec<(String, Sample)>>, usize);

fn load(store: &Store, store_dir: &std::path::Path) -> anyhow::Result<Loaded> {
    let stored: crate::StoredNet =
        serde_json::from_str(&store.get_kv(crate::NN_KEY)?.ok_or_else(|| anyhow::anyhow!("no stored response network"))?)?;
    let layout = ResponseFeatureSet::for_inputs(stored.net.input_size())
        .ok_or_else(|| anyhow::anyhow!("stored network has an unknown input layout"))?;
    let bots = store.bot_names()?;
    let hands = live_hands(store, &bots).ok_or_else(|| anyhow::anyhow!("live hands unreadable"))?;
    let mut profiles = ModelStore::default();
    for hand in warm_history(store_dir) {
        let hero = hand.players.iter().find_map(|(_, n)| bots.contains(n).then_some(n.clone()));
        profiles.observe(&hand, hero.as_deref());
    }
    let mut samples = Vec::with_capacity(hands.len());
    for (_, hand) in &hands {
        samples.push(named_samples_from_hand_for(hand, &profiles, &bots, layout).into_iter().map(|(name, s, _)| (name, s)).collect());
        let hero = hand.players.iter().find_map(|(_, n)| bots.contains(n).then_some(n.clone()));
        profiles.observe(hand, hero.as_deref());
    }
    let split = hands.len() * VALIDATION_SPLIT_PERCENT / 100;
    Ok((stored, samples, split))
}

/// Run the study on the stored live network and every stored live hand.
pub fn study(store: &Store, store_dir: &std::path::Path, priors: &[f32]) -> anyhow::Result<Vec<ResidualScore>> {
    let (net, samples, split) = load(store, store_dir)?;
    Ok(score(&samples, &net.net, split, priors))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv10_rng::{RngExt, SeedableRng};

    fn sample(facing: bool, label: usize) -> Sample {
        let mut x = vec![0.0; sv10_core::features::N_FEATURES];
        x[4] = if facing { 1.0 } else { 0.0 };
        Sample { x, mask: vec![facing, true, true], label, weight: 1.0 }
    }

    #[test]
    fn a_player_the_network_misreads_is_predicted_better_after_correction() {
        // Identical inputs for everyone, so the network cannot tell the nit (always folds) from the
        // station (always calls); their own history can.
        let net = Mlp::new(&[sv10_core::features::N_FEATURES, 4, 3], 1);
        let hands: Vec<Vec<(String, Sample)>> =
            (0..400).map(|_| vec![("nit".to_string(), sample(true, 0)), ("station".to_string(), sample(true, 1))]).collect();
        let scores = score(&hands, &net, 300, &[10.0, 100.0]);
        for s in &scores {
            assert_eq!((s.n, s.n_facing), (200, 200));
            assert!(s.gain > 0.1 && s.gain - s.half_width > 0.0, "{s:?}");
        }
        // Weaker shrinkage learns a deterministic player faster.
        assert!(scores[0].gain > scores[1].gain);
    }

    #[test]
    fn players_the_network_already_reads_gain_nothing_on_average() {
        let net = Mlp::new(&[sv10_core::features::N_FEATURES, 4, 3], 2);
        let p = net.predict(&sample(false, 1).x, &sample(false, 1).mask);
        // Labels drawn to match the network's own probabilities, so there is no residual to learn.
        let mut rng = sv10_rng::rngs::SmallRng::seed_from_u64(7);
        let hands: Vec<Vec<(String, Sample)>> = (0..3000)
            .map(|_| {
                let u: f32 = rng.random();
                let label = if u < p[1] { 1 } else { 2 };
                vec![("fair".to_string(), sample(false, label))]
            })
            .collect();
        let s = &score(&hands, &net, 1500, &[30.0])[0];
        assert!(s.gain.abs() < 0.01 && s.n_facing == 0, "{s:?}");
    }

    #[test]
    fn the_correction_applies_only_when_active_and_fitted_against_the_live_net() {
        let fit = ResidualFit {
            net_trained_at: 5.0,
            active: true,
            ratios: [("nit".to_string(), [2.0, 0.5, 1.0])].into_iter().collect(),
            ..Default::default()
        };
        let json = serde_json::to_string(&fit).unwrap();
        assert_eq!(installed(Some(&json), Some(5.0)).len(), 1);
        // Another network, no live network, an inactive fit or no fit: no correction.
        assert!(installed(Some(&json), Some(6.0)).is_empty());
        assert!(installed(Some(&json), None).is_empty());
        let off = serde_json::to_string(&ResidualFit { active: false, ..fit.clone() }).unwrap();
        assert!(installed(Some(&off), Some(5.0)).is_empty());
        assert!(installed(None, Some(5.0)).is_empty());
        assert!(installed(Some("not json"), Some(5.0)).is_empty());
    }
}

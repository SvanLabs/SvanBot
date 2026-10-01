//! Per-opponent fitted corrections (0210, 0214, 0223): the response-network residual ratios, fold
//! logit offsets and river sizing tells, installed in live decisions and refreshed when the learner
//! refits them.
//!
//! Every consumer goes through this module: the learner refits and stores them, the fleet loads
//! them at startup and whenever they change, and the dashboard shows the ones in use.
//! A new per-opponent fit is one field here plus its fit module.

use std::collections::HashMap;
use std::time::Instant;
use sv10_core::model::ModelStore;
use sv10_store::store::Store;

/// The installed per-opponent corrections, named as their [`ModelStore`] fields.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerFits {
    /// Per-opponent response-network correction: residual ratios (fold, call, raise) fitted from
    /// live hands against the network (0210); 1 = no correction.
    pub response_ratios: std::sync::Arc<HashMap<String, [f32; 3]>>,
    /// Per-opponent fold logit offsets for heads-up postflop bets, fitted from observed fold
    /// frequency against the street-calibrated estimate (0214); 0 = no correction.
    pub fold_offsets: std::sync::Arc<HashMap<String, f32>>,
    /// Per-opponent river sizing tells: how their river bet size tracks hand strength against the
    /// pool curve, fitted to their showdowns (0223); 0 = the pool curve.
    pub size_tells: std::sync::Arc<HashMap<String, f32>>,
}

impl PlayerFits {
    /// No corrections: what the learner and the golden snapshot play with.
    pub fn none() -> Self {
        Self::default()
    }

    /// The installed corrections from the stored fits (empty maps for any fit absent
    /// or inactive). A persistence read or typed parse failure returns an error so live refresh keeps the
    /// previously installed corrections.
    pub fn load(store: &Store) -> anyhow::Result<PlayerFits> {
        Ok(PlayerFits {
            response_ratios: crate::nnresidual::installed_from_store(store)?,
            fold_offsets: crate::playerfold::installed(
                crate::installs::read_checked::<crate::playerfold::PlayerFoldFit>(store, crate::playerfold::PLAYER_FOLD_KEY)?.as_deref(),
            ),
            size_tells: crate::playersize::installed(
                crate::installs::read_checked::<sv10_core::sizetell::SizeTellFit>(store, crate::playersize::PLAYER_SIZE_KEY)?.as_deref(),
            ),
        })
    }

    /// The corrections installed in `models`.
    pub fn of(models: &ModelStore) -> PlayerFits {
        PlayerFits {
            response_ratios: models.response_ratios.clone(),
            fold_offsets: models.fold_offsets.clone(),
            size_tells: models.size_tells.clone(),
        }
    }

    /// Install these corrections in `models`.
    pub fn apply(&self, models: &mut ModelStore) {
        models.response_ratios = self.response_ratios.clone();
        models.fold_offsets = self.fold_offsets.clone();
        models.size_tells = self.size_tells.clone();
    }

    /// One log line per correction that differs from `prev`, naming the count now in use.
    pub fn changes_from(&self, prev: &PlayerFits) -> Vec<String> {
        let mut lines = Vec::new();
        if self.response_ratios != prev.response_ratios {
            lines.push(format!("per-opponent response correction: {} opponents", self.response_ratios.len()));
        }
        if self.fold_offsets != prev.fold_offsets {
            lines.push(format!("per-opponent fold calibration: {} opponents", self.fold_offsets.len()));
        }
        if self.size_tells != prev.size_tells {
            lines.push(format!("per-opponent river sizing tells: {} opponents", self.size_tells.len()));
        }
        lines
    }
}

/// Refit and store every per-opponent fit (the learner: at start, every cycle, hourly while it
/// waits, and after a response network is approved). The fold offsets sit on the street shifts in
/// play, so this runs after the live fits ([`crate::livefits`]).
pub fn refit(store: &Store, store_dir: &std::path::Path) {
    let t0 = Instant::now();
    crate::nnresidual::refit(store, store_dir);
    crate::playerfold::refit(store);
    crate::playersize::refit(store);
    tracing::debug!("per-opponent fits refitted in {:.1}s", t0.elapsed().as_secs_f64());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_store_installs_nothing_and_apply_round_trips() {
        let dir = std::env::temp_dir().join(format!("sv10-playerfits-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        assert_eq!(PlayerFits::load(&store).unwrap(), PlayerFits::none());
        let fits = PlayerFits { fold_offsets: std::sync::Arc::new([("v".to_string(), -0.4)].into_iter().collect()), ..PlayerFits::none() };
        let mut models = ModelStore::default();
        fits.apply(&mut models);
        assert_eq!(PlayerFits::of(&models), fits);
        assert_eq!(models.profile("v").fold_logit_offset, -0.4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn player_fits_none_is_empty() {
        let none = PlayerFits::none();
        assert!(none.response_ratios.is_empty());
        assert!(none.fold_offsets.is_empty());
    }

    #[test]
    fn changes_from_reports_differs() {
        let prev = PlayerFits {
            response_ratios: std::sync::Arc::new([("a".to_string(), [1.0, 1.0, 1.0])].into_iter().collect()),
            fold_offsets: Default::default(),
            size_tells: Default::default(),
        };
        let now = PlayerFits {
            response_ratios: std::sync::Arc::new(
                [("a".to_string(), [1.0, 1.0, 1.0]), ("b".to_string(), [2.0, 1.0, 1.0])].into_iter().collect(),
            ),
            fold_offsets: std::sync::Arc::new([("x".to_string(), 0.5)].into_iter().collect()),
            size_tells: std::sync::Arc::new([("y".to_string(), 0.4)].into_iter().collect()),
        };
        let changes = now.changes_from(&prev);
        assert_eq!(changes.len(), 3);
        assert!(changes[0].contains("2 opponents") && changes[0].contains("response"));
        assert!(changes[1].contains("1 opponents") && changes[1].contains("fold"));
        assert!(changes[2].contains("1 opponents") && changes[2].contains("sizing"));
    }
}

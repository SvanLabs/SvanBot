//! Per-opponent river sizing tells (0223): the store side of [`sv10_core::sizetell`]. The learner
//! refits them with the other per-opponent fits ([`crate::playerfits::refit`]: at start, every
//! cycle and hourly while it waits), and the fleet installs them through
//! [`crate::playerfits::PlayerFits`] only while the stored fit is active.

use sv10_core::calibrate::samples_from_hand;
use sv10_core::model::{HandSummary, ModelStore};
use sv10_core::sizetell::SizeTellFit;
use sv10_store::store::Store;

/// Kv key of the stored fit ([`SizeTellFit`]).
pub const PLAYER_SIZE_KEY: &str = "player_size.v1";
/// Newest showdown hands read per refit (bounds the refit's time as the store grows).
const MAX_HANDS: usize = 60_000;

/// The river-bet showdowns of opponents in our stored hands, oldest first, profiled with the
/// stored opponent models; our own bots (every name that ever played for us) are excluded.
fn samples(store: &Store) -> anyhow::Result<Vec<sv10_core::calibrate::ShowdownSample>> {
    let models: ModelStore = store.get_kv(crate::MODELS_KEY)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let fleet = store.bot_names()?;
    let mut hands: Vec<(String, HandSummary)> = Vec::new();
    let mut unreadable = 0usize;
    for bot in &fleet {
        for row in store.recent_fit_hands(bot, MAX_HANDS)? {
            match serde_json::from_str::<HandSummary>(&row.summary) {
                Ok(h) if h.board.len() == 5 && !h.shown.is_empty() => hands.push((row.ended_at, h)),
                Ok(_) => {}
                Err(_) => unreadable += 1,
            }
        }
    }
    if unreadable > 0 {
        tracing::warn!("per-opponent sizing: {unreadable} stored hands had unreadable summaries");
    }
    hands.sort_by(|a, b| a.0.cmp(&b.0));
    let keep = hands.len().saturating_sub(MAX_HANDS);
    Ok(hands[keep..].iter().flat_map(|(_, h)| samples_from_hand(h, &models, &fleet)).filter(|s| s.bet_the_river()).collect())
}

/// Fit from the stored hands against the range model in play (not stored).
pub fn fit_from_store(store: &Store) -> anyhow::Result<SizeTellFit> {
    Ok(fits_from_store(store, &[sv10_core::sizetell::PRIOR])?.remove(0))
}

/// One fit per shrinkage prior on the same samples (for `review sizing-fit`).
pub fn fits_from_store(store: &Store, priors: &[f64]) -> anyhow::Result<Vec<SizeTellFit>> {
    let samples = samples(store)?;
    let rp = crate::fitted_range_params(store.get_kv(crate::RANGE_PARAMS_KEY)?.as_deref()).unwrap_or_default();
    Ok(priors.iter().map(|&p| sv10_core::sizetell::fit_with(&samples, &rp, p)).collect())
}

/// Refit and store (the learner, with the other per-opponent fits).
pub fn refit(store: &Store) {
    let t0 = std::time::Instant::now();
    let f = match fit_from_store(store) {
        Ok(f) => f,
        Err(e) => return tracing::warn!("per-opponent sizing not refitted, store unreadable: {e}"),
    };
    tracing::info!(
        "per-opponent river sizing: {} opponents; held-out gain {:+.2} ± {:.2} mnats on {} river-bet showdowns -> {} ({:.1}s)",
        f.tells.len(),
        f.gain * 1000.0,
        f.half_width * 1000.0,
        f.n,
        if f.active { "installed" } else { "not installed" },
        t0.elapsed().as_secs_f64()
    );
    match serde_json::to_string(&f) {
        Ok(j) => {
            if let Err(e) = store.put_kv(PLAYER_SIZE_KEY, &j) {
                tracing::warn!("per-opponent sizing not stored: {e}");
            }
        }
        Err(e) => tracing::warn!("per-opponent sizing not serialized: {e}"),
    }
}

/// The tells to play with from the stored fit (none unless it is active).
pub fn installed(stored: Option<&str>) -> std::sync::Arc<std::collections::HashMap<String, f32>> {
    sv10_core::sizetell::installed(stored.and_then(|j| serde_json::from_str::<SizeTellFit>(j).ok()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_an_active_readable_fit_installs() {
        let fit =
            sv10_core::sizetell::SizeTellFit { active: true, tells: [("v".to_string(), 0.5)].into_iter().collect(), ..Default::default() };
        assert_eq!(super::installed(Some(&serde_json::to_string(&fit).unwrap()))["v"], 0.5);
        let off = sv10_core::sizetell::SizeTellFit { active: false, ..fit };
        assert!(super::installed(Some(&serde_json::to_string(&off).unwrap())).is_empty());
        assert!(super::installed(None).is_empty() && super::installed(Some("junk")).is_empty());
    }
}

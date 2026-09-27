//! Neural response model training: every stored opponent decision becomes a sample, validation
//! is the most recent 15% of live hands, and the net is active only if it beats the stat model.

use crate::StoredNet;
use std::sync::Arc;
use sv10_core::model::{HandSummary, ModelStore};
use sv10_core::nn::{Mlp, Sample, TrainConfig, log_loss, mean_log_loss, train};
use sv10_store::store::Store;

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64()
}

/// Past-season hands added to neural training (most recent first).
const HISTORY_TRAIN_HANDS: usize = 60_000;

/// Layer widths of the production response model; a stored net of any other shape cannot warm-start.
const SHAPE: &[usize] = &[sv10_core::features::N_FEATURES, 48, 24, 3];

/// Fine-tune epochs for a warm start (a fresh seed trains the full budget below).
const WARM_EPOCHS: usize = 4;

/// Full training budget for a fresh seed.
const FRESH_EPOCHS: usize = 10;

/// A fresh train worse than the stored net by more than this keeps the stored net (0131):
/// validation sets drift as hands accumulate, so the comparison gets a tolerance.
const RATCHET_TOLERANCE: f64 = 0.005;

/// Training artifacts without this exact chronology contract are never exposed to live policy.
pub const RESPONSE_TRAINING_CONTRACT: &str = "profiles-before-hand-v1";

/// A stored net is reusable as a warm start when its layers match [`SHAPE`] exactly.
fn warm_startable(net: &Mlp) -> bool {
    let want: Vec<(usize, usize)> = SHAPE.windows(2).map(|w| (w[0], w[1])).collect();
    let have: Vec<(usize, usize)> = net.layers.iter().map(|l| (l.inputs, l.outputs)).collect();
    have == want
}

/// A stored net can be served when its hidden layers match [`SHAPE`] and its input width is a
/// layout inference still supports (the 37-input net keeps playing until a 38-input one is
/// approved; 0135).
fn fits_shape(net: &Mlp) -> bool {
    let Some(first) = net.layers.first() else { return false };
    let mut shape = SHAPE.to_vec();
    shape[0] = first.inputs;
    let want: Vec<(usize, usize)> = shape.windows(2).map(|w| (w[0], w[1])).collect();
    let have: Vec<(usize, usize)> = net.layers.iter().map(|l| (l.inputs, l.outputs)).collect();
    have == want && sv10_core::features::ResponseFeatureSet::for_inputs(first.inputs).is_some()
}

/// Return a response model only when predictive, layout, chronology and paired-poker gates all pass.
pub fn active_response_net(stored: Option<StoredNet>) -> Option<Arc<Mlp>> {
    stored.filter(response_net_is_eligible).map(|candidate| Arc::new(candidate.net))
}

/// The single live/dashboard eligibility predicate for a stored response model.
pub fn response_net_is_eligible(candidate: &StoredNet) -> bool {
    candidate.active && candidate.paired_poker_approved && has_current_training_contract(candidate) && fits_shape(&candidate.net)
}

/// Whether a stored artifact is predictive and well-formed but still waiting for the fresh-deal
/// paired poker check that exposes it to live play.
pub fn awaits_poker_approval(stored: &StoredNet) -> bool {
    stored.active && !stored.paired_poker_approved && has_current_training_contract(stored) && fits_shape(&stored.net)
}

/// Where the learner writes a freshly trained artifact (0142).
#[derive(Debug, PartialEq, Eq)]
pub enum NetSlot {
    /// The live key the fleet reads (`NN_KEY`).
    Live,
    /// The candidate key (`NN_CANDIDATE_KEY`): the approved incumbent keeps playing while the
    /// candidate waits for the paired poker gate, and moves to the live key only on approval.
    Candidate,
}

/// A predictive candidate that still needs poker approval must not displace an approved live
/// net: writing it to the live key left the fleet without a response model for the 60–90 s of
/// every gate (50 gaps logged by 2026-09-22), and after a rejection until the next cycle.
pub fn slot_for_trained(incumbent: Option<&StoredNet>, trained: &StoredNet) -> NetSlot {
    if awaits_poker_approval(trained) && incumbent.is_some_and(response_net_is_eligible) { NetSlot::Candidate } else { NetSlot::Live }
}

/// The net the fleet plays after the live key changes: an artifact still awaiting poker
/// approval never replaces the current net (defence for stores written by older learners).
pub fn next_live_net(current: Option<Arc<Mlp>>, stored: Option<StoredNet>) -> Option<Arc<Mlp>> {
    match stored {
        Some(s) if awaits_poker_approval(&s) => current,
        other => active_response_net(other),
    }
}

/// Outcome of the fresh-deal paired poker check that gates exposure of a candidate response model.
#[derive(Debug, PartialEq, Eq)]
pub enum PokerVerdict {
    /// Expose this artifact to live play.
    Approve,
    /// Keep it stored but unexposed, with the reason.
    Reject(&'static str),
}

/// Decide exposure for a candidate response model from its paired poker result (candidate minus
/// incumbent on identical fresh deals).
///
/// The simulated pool is profile clones drawn from the very stat profiles the fallback model uses,
/// so a paired sim is structurally unable to credit the net for reading *real* opponents better
/// than statistics: that benefit is what the held-out predictive gate measures on live hands.
/// This check exists to catch a net that costs chips, so a candidate is approved unless the paired
/// result is significantly negative (95% upper bound at or below zero).
pub fn poker_verdict(r: &sv10_core::sim::PairedResult) -> PokerVerdict {
    if r.differing == 0 {
        return PokerVerdict::Reject("the candidate never changed a decision");
    }
    if r.upper_95() <= 0.0 {
        return PokerVerdict::Reject("significantly worse than the incumbent on fresh deals");
    }
    PokerVerdict::Approve
}

/// Whether an artifact was evaluated with the current no-future-profile contract.
pub fn has_current_training_contract(stored: &StoredNet) -> bool {
    stored.training_contract == RESPONSE_TRAINING_CONTRACT
}

/// Share of live hands (oldest first) the response model trains on; the rest validate it.
pub const VALIDATION_SPLIT_PERCENT: usize = 85;

/// Every stored live hand of `bots` with its seat stacks, oldest first, with its end time.
pub fn live_hands(store: &Store, bots: &[String]) -> Option<Vec<(String, HandSummary)>> {
    let mut hands: Vec<(String, HandSummary)> = Vec::new();
    for b in bots {
        for row in store.recent_fit_hands(b, 200_000).ok()? {
            if let Ok(h) = serde_json::from_str::<HandSummary>(&row.summary)
                && !h.stacks.is_empty()
            {
                hands.push((row.ended_at.clone(), h));
            }
        }
    }
    hands.sort_by(|a, b| a.0.cmp(&b.0));
    Some(hands)
}

/// Past-season hands (oldest first) that warm the opponent profiles before live hands, as training does.
pub fn warm_history(store_dir: &std::path::Path) -> Vec<HandSummary> {
    crate::history::HistoryDb::open(&store_dir.join("history.db"))
        .and_then(|db| db.recent_summaries(HISTORY_TRAIN_HANDS, false))
        .unwrap_or_default()
        .into_iter()
        .map(|(_, hand)| hand)
        .collect()
}

/// Extract each hand using only profiles observable before that hand, then advance the profiles.
/// This is intentionally sequential: parallel extraction against the final model leaks validation
/// actions into their own features and baseline.
fn chronological_samples<'a>(
    hands: impl IntoIterator<Item = &'a HandSummary>,
    profile_models: &mut ModelStore,
    exclude: &[String],
) -> Vec<Vec<(Sample, Vec<f32>)>> {
    let mut out = Vec::new();
    for hand in hands {
        out.push(sv10_core::features::samples_from_hand(hand, profile_models, exclude));
        let hero = hand.players.iter().find_map(|(_, name)| exclude.contains(name).then_some(name.as_str()));
        profile_models.observe(hand, hero);
    }
    out
}

/// Train the neural response model on every stored opponent decision and mark its predictive gate
/// when it beats the stat model on the most recent 15% of hands. Exposure remains blocked until
/// this exact artifact also records paired-poker approval. `extra` hands (other sources) add
/// training samples only; validation stays on live hands.
pub fn train_response_model(
    store: &Store,
    store_dir: &std::path::Path,
    _models: &ModelStore,
    cycle: u64,
    extra: &[HandSummary],
) -> Option<StoredNet> {
    let exclude: Vec<String> = store.bot_names().ok()?;
    let hands = live_hands(store, &exclude)?;
    let split = hands.len() * VALIDATION_SPLIT_PERCENT / 100;
    let mut train_set: Vec<Sample> = Vec::new();
    let mut val: Vec<(Sample, Vec<f32>)> = Vec::new();
    // Past-season hands from the server export add training data only; validation stays on
    // the most recent live hands, so the activation gate still measures current opponents.
    let history = warm_history(store_dir);
    let history_hands = history.len();
    let mut live_profiles = ModelStore::default();
    train_set.extend(chronological_samples(history.iter(), &mut live_profiles, &exclude).into_iter().flatten().map(|(sample, _)| sample));
    let before_extra = train_set.len();
    // Other-source hands have no shared chronological frontier with live hands. They train against
    // their own past and never alter the profiles used by the live validation gate.
    let mut extra_profiles = ModelStore::default();
    train_set.extend(chronological_samples(extra, &mut extra_profiles, &exclude).into_iter().flatten().map(|(sample, _)| sample));
    if !extra.is_empty() {
        tracing::info!("neural model: {} other-source hands add {} training samples", extra.len(), train_set.len() - before_extra);
    }
    if history_hands > 0 {
        tracing::info!("neural model: {} past-season hands add {} training samples", history_hands, before_extra);
    }
    let live_samples = chronological_samples(hands.iter().map(|(_, hand)| hand), &mut live_profiles, &exclude);
    for (i, s) in live_samples.into_iter().enumerate() {
        if i < split {
            train_set.extend(s.into_iter().map(|(sample, _)| sample));
        } else {
            val.extend(s);
        }
    }
    if train_set.len() < 2000 || val.len() < 300 {
        tracing::info!("neural model: not enough data yet ({} train / {} val samples)", train_set.len(), val.len());
        return None;
    }
    let mut net;
    let stored: Option<StoredNet> = store.get_kv(crate::NN_KEY).ok().flatten().and_then(|j| serde_json::from_str(&j).ok());
    // Warm-start from the stored net when its shape still matches: the same optimum refined with
    // new hands beats a fresh seed lottery, and fine-tuning costs less than half the epochs.
    // Standardization is recomputed from the new data inside `train`, so no staleness carries over.
    match stored.as_ref().filter(|s| warm_startable(&s.net)) {
        Some(s) => {
            net = s.net.clone();
            train(&mut net, &train_set, &TrainConfig { epochs: WARM_EPOCHS, lr: 1e-3, seed: 10_000 + cycle, ..Default::default() });
        }
        None => {
            net = Mlp::new(SHAPE, 11 + cycle);
            train(&mut net, &train_set, &TrainConfig { epochs: FRESH_EPOCHS, ..Default::default() });
        }
    }
    let val_samples: Vec<Sample> = val.iter().map(|(s, _)| s.clone()).collect();
    let val_loss = mean_log_loss(&net, &val_samples);
    let baseline_loss = val.iter().map(|(s, b)| log_loss(b, s.label)).sum::<f64>() / val.len() as f64;
    let active = val_loss < baseline_loss - 0.01;
    tracing::info!(
        "neural model: val log-loss {val_loss:.4} vs stat baseline {baseline_loss:.4} on {} samples -> {}",
        val.len(),
        if active { "predictive gate passed; awaiting paired-poker approval" } else { "predictive gate failed" }
    );
    if active
        && let Some(s) = stored.as_ref().filter(|s| s.active && s.paired_poker_approved && has_current_training_contract(s))
        && val_loss > s.val_loss + RATCHET_TOLERANCE
    {
        tracing::info!("neural model: fresh train {val_loss:.4} worse than stored {:.4}; keeping the stored net", s.val_loss);
        return Some(s.clone());
    }
    Some(StoredNet {
        net,
        active,
        training_contract: RESPONSE_TRAINING_CONTRACT.into(),
        paired_poker_approved: false,
        val_loss,
        baseline_loss,
        train_samples: train_set.len(),
        val_samples: val.len(),
        trained_at: now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv10_core::engine::{ActionKind, ActionRecord, Street};

    fn stored(net: Mlp, active: bool) -> StoredNet {
        StoredNet {
            net,
            active,
            training_contract: RESPONSE_TRAINING_CONTRACT.into(),
            paired_poker_approved: true,
            val_loss: 0.5,
            baseline_loss: 0.6,
            train_samples: 2_000,
            val_samples: 300,
            trained_at: 1.0,
        }
    }

    fn paired(mean_bb100: f64, se_bb100: f64) -> sv10_core::sim::PairedResult {
        sv10_core::sim::PairedResult { hands: 100_000, mean_bb: mean_bb100 / 100.0, se_bb: se_bb100 / 100.0, differing: 5_000 }
    }

    #[test]
    fn a_predictive_unapproved_artifact_awaits_the_poker_gate() {
        let mut candidate = stored(Mlp::new(SHAPE, 6), true);
        candidate.paired_poker_approved = false;
        assert!(awaits_poker_approval(&candidate));
        assert!(!response_net_is_eligible(&candidate));
        candidate.paired_poker_approved = true;
        assert!(!awaits_poker_approval(&candidate));
        assert!(response_net_is_eligible(&candidate));
        let mut failed = stored(Mlp::new(SHAPE, 7), false);
        failed.paired_poker_approved = false;
        assert!(!awaits_poker_approval(&failed));
    }

    #[test]
    fn a_pending_candidate_never_takes_the_live_net_away() {
        let live = stored(Mlp::new(SHAPE, 11), true);
        let mut fresh = stored(Mlp::new(SHAPE, 12), true);
        fresh.paired_poker_approved = false;
        // With an approved incumbent the candidate waits in its own slot...
        assert_eq!(slot_for_trained(Some(&live), &fresh), NetSlot::Candidate);
        // ...and the fleet keeps the live net even if a pending artifact reaches the live key.
        let current = active_response_net(Some(live.clone()));
        let kept = next_live_net(current.clone(), Some(fresh.clone())).expect("live net kept");
        assert!(Arc::ptr_eq(&kept, current.as_ref().unwrap()));
        // Approval moves it to the live key, and the fleet switches to it.
        fresh.paired_poker_approved = true;
        assert_eq!(slot_for_trained(Some(&live), &fresh), NetSlot::Live);
        assert!(next_live_net(current.clone(), Some(fresh)).is_some_and(|n| !Arc::ptr_eq(&n, current.as_ref().unwrap())));
        // An inactive (predictively failed) artifact still replaces it, as before.
        assert!(next_live_net(current, Some(stored(Mlp::new(SHAPE, 13), false))).is_none());
    }

    #[test]
    fn a_first_candidate_without_an_incumbent_goes_to_the_live_key() {
        let mut fresh = stored(Mlp::new(SHAPE, 14), true);
        fresh.paired_poker_approved = false;
        assert_eq!(slot_for_trained(None, &fresh), NetSlot::Live);
        let unapproved_incumbent = fresh.clone();
        assert_eq!(slot_for_trained(Some(&unapproved_incumbent), &fresh), NetSlot::Live);
        assert!(next_live_net(None, Some(fresh)).is_none());
    }

    #[test]
    fn poker_gate_only_rejects_demonstrated_harm() {
        // Clone pools cannot credit the net, so a neutral or noisy result still ships.
        assert_eq!(poker_verdict(&paired(0.0, 4.0)), PokerVerdict::Approve);
        assert_eq!(poker_verdict(&paired(-5.0, 4.0)), PokerVerdict::Approve);
        assert_eq!(poker_verdict(&paired(12.0, 4.0)), PokerVerdict::Approve);
        // Significantly worse: upper bound at or below zero.
        assert!(matches!(poker_verdict(&paired(-12.0, 4.0)), PokerVerdict::Reject(_)));
        // A net that never changes a decision has nothing to expose.
        assert!(matches!(poker_verdict(&sv10_core::sim::PairedResult { differing: 0, ..paired(3.0, 1.0) }), PokerVerdict::Reject(_)));
    }

    #[test]
    fn only_matching_shapes_warm_start() {
        assert!(fits_shape(&Mlp::new(SHAPE, 1)));
        assert!(warm_startable(&Mlp::new(SHAPE, 1)));
        // The previous 37-input layout is still served, but never warm-started into the new layout.
        let old = Mlp::new(&[sv10_core::features::INCUMBENT_FEATURES, 48, 24, 3], 1);
        assert!(fits_shape(&old) && !warm_startable(&old));
        assert!(!fits_shape(&Mlp::new(&[sv10_core::features::N_FEATURES, 16, 3], 1)));
        assert!(!fits_shape(&Mlp::new(&[20, 48, 24, 3], 1)));
    }

    #[test]
    fn only_active_matching_response_nets_are_exposed() {
        let eligible = stored(Mlp::new(SHAPE, 1), true);
        assert!(response_net_is_eligible(&eligible));
        assert!(active_response_net(Some(eligible)).is_some());
        assert!(active_response_net(Some(stored(Mlp::new(SHAPE, 2), false))).is_none());
        assert!(active_response_net(Some(stored(Mlp::new(&[20, 48, 24, 3], 3), true))).is_none());
        let mut legacy = stored(Mlp::new(SHAPE, 4), true);
        legacy.training_contract.clear();
        assert!(!response_net_is_eligible(&legacy));
        assert!(active_response_net(Some(legacy)).is_none());
        let mut unconfirmed = stored(Mlp::new(SHAPE, 5), true);
        unconfirmed.paired_poker_approved = false;
        assert!(active_response_net(Some(unconfirmed)).is_none());
    }

    fn response_hand(kind: ActionKind) -> HandSummary {
        HandSummary {
            players: vec![(0, "Hero".into()), (1, "Villain".into())],
            button: 0,
            bb: 20,
            history: vec![ActionRecord {
                seat: 1,
                street: Street::Preflop,
                kind,
                to: 0,
                pot_before: 30,
                to_call_before: 20,
                bet_before: 20,
                full_raise: false,
                think_ms: None,
                street_open: false,
            }],
            board: vec![],
            shown: vec![],
            stacks: vec![(0, 2_000), (1, 2_000)],
        }
    }

    #[test]
    fn chronological_extraction_never_sees_the_current_or_future_hand() {
        let hands = [response_hand(ActionKind::Fold), response_hand(ActionKind::Call)];
        let mut profiles = ModelStore::default();
        let samples = chronological_samples(&hands, &mut profiles, &["Hero".into()]);
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0][0].0.x[24], 0.0, "first hand must use the unseen-player prior");
        assert!(samples[1][0].0.x[24] > 0.0, "second hand may use only the first hand's profile update");
    }
}

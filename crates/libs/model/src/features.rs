//! Features for the neural opponent-response model, computable both from a
//! replayed hand (training) and from a live decision (inference), plus the
//! hand-built stat model used as the baseline it has to beat.

#[cfg(test)]
mod texture_tests;

use crate::model::{HandSummary, ModelStore, Profile, aggressive};
use sv10_cards::cards::Card;
use sv10_engine::engine::{ActionKind, Street};
use sv10_engine::situation::postflop_order;
use sv10_nn::nn::Sample;

/// 37 since 2026-09-15: every per-opponent statistic the model tracks is an input (c-bet,
/// fold to c-bet, opens, calls, 4-bets, limps, river bluffs, fold by bet size, showdown wins) plus
/// whether the bet faced is the preflop aggressor's flop bet. A stored network with another input
/// size is never used. Production trains [`ResponseFeatureSet::StraightTexture39`]; networks
/// of the older 37- and 38-input layouts still run with their original inputs.
pub const N_FEATURES: usize = 39;
/// Inputs of the [`ResponseFeatureSet::Incumbent37`] layout, still served for networks trained on it.
pub const INCUMBENT_FEATURES: usize = 37;

/// Versioned response-model layouts. Training uses the production layout ([`N_FEATURES`] inputs);
/// inference picks the layout from each network's input width, so a stored network keeps working
/// until a network of the new layout passes the neural gates (0135).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseFeatureSet {
    /// The 37-input layout (production until 2026-09-22).
    Incumbent37,
    /// 38 inputs: the 37 plus the responder's calls on completed earlier postflop streets. Held-out
    /// log-loss −2.77 mnats (95% −3.95..−1.58) validating after 2026-09-21 and −1.09 (−1.86..−0.33)
    /// on 2026-09-18..21 against the 37-input layout with identical splits and seeds (0135).
    PriorStreetCalls38,
    /// 39 inputs: corrected straight connectivity at index 15 (distinct ranks including the wheel),
    /// plus wheel connectivity at index 38. The older layouts retain their trained texture values.
    StraightTexture39,
}

impl ResponseFeatureSet {
    /// The production layout new networks are trained on.
    pub const PRODUCTION: ResponseFeatureSet = ResponseFeatureSet::StraightTexture39;

    /// Input width of this layout.
    pub fn inputs(self) -> usize {
        match self {
            ResponseFeatureSet::Incumbent37 => INCUMBENT_FEATURES,
            ResponseFeatureSet::PriorStreetCalls38 => 38,
            ResponseFeatureSet::StraightTexture39 => N_FEATURES,
        }
    }

    /// The layout a network with `inputs` input units was trained on, if it is one we serve.
    pub fn for_inputs(inputs: usize) -> Option<ResponseFeatureSet> {
        [ResponseFeatureSet::Incumbent37, ResponseFeatureSet::PriorStreetCalls38, ResponseFeatureSet::StraightTexture39]
            .into_iter()
            .find(|s| s.inputs() == inputs)
    }
}

/// An opponent's decision point as the response model sees it.
#[derive(Clone, Debug)]
pub struct ResponseContext<'a> {
    /// Street of the decision.
    pub street: Street,
    /// Chips the player owes.
    pub to_call: i64,
    /// Pot before the decision.
    pub pot_before: i64,
    /// Player's chips behind.
    pub stack_behind: i64,
    /// Big blind.
    pub bb: i64,
    /// Players still in the hand.
    pub active_players: usize,
    /// Whether the player acts last postflop.
    pub in_position: bool,
    /// Raises made preflop.
    pub preflop_raises: usize,
    /// Whether the player made the last preflop raise.
    pub was_preflop_aggressor: bool,
    /// Facing a flop bet from the preflop aggressor (a continuation bet).
    pub facing_cbet: bool,
    /// Calls this responder made on completed earlier postflop streets.
    pub prior_postflop_calls: usize,
    /// Board cards.
    pub board: &'a [Card],
    /// The player's shrunk statistics.
    pub profile: &'a Profile,
}

fn legacy_texture(board: &[Card]) -> [f32; 4] {
    if board.is_empty() {
        return [0.0; 4];
    }
    let mut ranks = [0u8; 13];
    let mut suits = [0u8; 4];
    for c in board {
        ranks[c.rank() as usize] += 1;
        suits[c.suit() as usize] += 1;
    }
    let paired = ranks.iter().any(|&r| r >= 2) as u8 as f32;
    let flushy = ((*suits.iter().max().unwrap() as f32) - 1.0).max(0.0) / 3.0;
    let mut best = 0;
    for lo in 0..=9 {
        let mut n = 0;
        for k in 0..5 {
            let r = if lo + k == 13 { 12 } else { (lo + k) % 13 };
            if ranks[r] > 0 {
                n += 1;
            }
        }
        best = best.max(n);
    }
    let high = board.iter().map(|c| c.rank()).max().unwrap() as f32 / 12.0;
    [paired, flushy, best as f32 / 5.0, high]
}

fn wheel_connectivity(board: &[Card]) -> f32 {
    [12, 0, 1, 2, 3].iter().filter(|&&rank| board.iter().any(|card| card.rank() == rank)).count() as f32 / 5.0
}

fn texture(board: &[Card]) -> [f32; 4] {
    let mut t = legacy_texture(board);
    let best = (0..=8).map(|lo| (lo..lo + 5).filter(|&rank| board.iter().any(|card| card.rank() == rank)).count()).max().unwrap_or(0);
    t[2] = (best as f32 / 5.0).max(wheel_connectivity(board));
    t
}

/// The production-layout ([`N_FEATURES`]) model inputs for a decision point.
pub fn features(c: &ResponseContext) -> Vec<f32> {
    features_for(c, ResponseFeatureSet::PRODUCTION)
}

/// Inputs for an explicitly versioned response-model layout.
pub fn features_for(c: &ResponseContext, feature_set: ResponseFeatureSet) -> Vec<f32> {
    let mut f = vec![0f32; INCUMBENT_FEATURES];
    f[c.street.index()] = 1.0;
    let facing = c.to_call > 0;
    f[4] = facing as u8 as f32;
    let base = (c.pot_before - c.to_call).max(1) as f32;
    f[5] = if facing { (1.0 + c.to_call as f32 / base).ln() } else { 0.0 };
    let bb = c.bb.max(1) as f32;
    f[6] = (1.0 + c.pot_before as f32 / bb).ln();
    f[7] = (1.0 + c.stack_behind.max(0) as f32 / c.pot_before.max(1) as f32).ln();
    f[8] = c.active_players as f32 / 6.0;
    f[9] = c.in_position as u8 as f32;
    f[10] = c.preflop_raises.min(3) as f32 / 3.0;
    f[11] = c.was_preflop_aggressor as u8 as f32;
    f[12] = (facing && c.to_call >= c.stack_behind) as u8 as f32;
    let t = if feature_set == ResponseFeatureSet::StraightTexture39 { texture(c.board) } else { legacy_texture(c.board) };
    f[13..17].copy_from_slice(&t);
    let p = c.profile;
    let st = c.street.index().saturating_sub(1).min(2);
    f[17] = p.vpip;
    f[18] = p.pfr;
    f[19] = p.three_bet;
    f[20] = p.bet_first[st];
    f[21] = p.fold_vs_bet[st];
    f[22] = p.raise_vs_bet[st];
    f[23] = p.wtsd;
    f[24] = p.confidence;
    f[25] = p.fold_to_3bet;
    f[26] = p.cbet;
    f[27] = p.fold_to_cbet;
    f[28] = p.open_raise;
    f[29] = p.call_open;
    f[30] = p.four_bet;
    f[31] = p.fold_to_4bet;
    f[32] = p.limp;
    f[33] = p.river_bluff;
    f[34] = if facing { p.fold_vs_size[crate::model::size_bucket(c.to_call as f64 / base as f64)] } else { 0.0 };
    f[35] = p.won_showdown;
    f[36] = c.facing_cbet as u8 as f32;
    if feature_set != ResponseFeatureSet::Incumbent37 {
        f.push(c.prior_postflop_calls.min(3) as f32 / 3.0);
    }
    if feature_set == ResponseFeatureSet::StraightTexture39 {
        f.push(wheel_connectivity(c.board));
    }
    f
}

/// Count only this responder's calls on completed earlier postflop streets.
pub fn prior_postflop_calls(history: &[sv10_engine::engine::ActionRecord], seat: usize, current_street: Street) -> usize {
    history
        .iter()
        .filter(|record| {
            record.seat == seat
                && record.kind == ActionKind::Call
                && record.street != Street::Preflop
                && record.street.index() < current_street.index()
        })
        .count()
}

/// Legal classes: fold only when facing a bet; call/check and bet/raise always.
pub fn mask(c: &ResponseContext) -> Vec<bool> {
    vec![c.to_call > 0, true, true]
}

/// Hand-built stat model prediction for the same decision.
pub fn baseline(c: &ResponseContext) -> Vec<f32> {
    let p = c.profile;
    let st = c.street.index().saturating_sub(1).min(2);
    let (fold, raise) = if c.street == Street::Preflop {
        match c.preflop_raises {
            0 => (1.0 - p.open_raise - p.limp, p.open_raise),
            1 => (1.0 - p.call_open - p.three_bet, p.three_bet),
            2 => (p.fold_to_3bet, p.four_bet),
            _ => (0.35, 0.1),
        }
    } else if c.to_call > 0 {
        let base = (c.pot_before - c.to_call).max(1) as f64;
        (p.fold_to_bet(c.street, c.to_call as f64 / base) as f32, p.raise_vs_bet[st])
    } else {
        (0.0, p.bet_first[st])
    };
    let fold = if c.to_call > 0 { fold.clamp(0.01, 0.97) } else { 0.0 };
    let raise = raise.clamp(0.01, 0.9);
    let call = (1.0 - fold - raise).max(0.02);
    let s = fold + call + raise;
    vec![fold / s, call / s, raise / s]
}

/// Turn every non-hero decision in a stored hand into a training sample.
/// Returns (sample, baseline probabilities) pairs.
pub fn samples_from_hand(hand: &HandSummary, models: &ModelStore, exclude: &[String]) -> Vec<(Sample, Vec<f32>)> {
    samples_from_hand_for(hand, models, exclude, ResponseFeatureSet::PRODUCTION)
}

/// Extract samples for an explicitly versioned response-model layout.
pub fn samples_from_hand_for(
    hand: &HandSummary,
    models: &ModelStore,
    exclude: &[String],
    feature_set: ResponseFeatureSet,
) -> Vec<(Sample, Vec<f32>)> {
    named_samples_from_hand_for(hand, models, exclude, feature_set).into_iter().map(|(_, sample, base)| (sample, base)).collect()
}

/// [`samples_from_hand_for`] with the responder's name on each sample, for per-opponent study of
/// the network's residuals (0210).
pub fn named_samples_from_hand_for(
    hand: &HandSummary,
    models: &ModelStore,
    exclude: &[String],
    feature_set: ResponseFeatureSet,
) -> Vec<(String, Sample, Vec<f32>)> {
    let names: std::collections::HashMap<usize, &String> = hand.players.iter().map(|(s, n)| (*s, n)).collect();
    let stacks: std::collections::HashMap<usize, i64> = hand.stacks.iter().copied().collect();
    let seats: Vec<usize> = hand.players.iter().map(|(s, _)| *s).collect();
    let order = postflop_order(&seats, hand.button);
    let mut invested: std::collections::HashMap<usize, i64> = std::collections::HashMap::new();
    let mut street_bet: std::collections::HashMap<usize, i64> = std::collections::HashMap::new();
    let mut folded: Vec<usize> = Vec::new();
    let mut street = Street::Preflop;
    let mut preflop_raises = 0;
    let mut pf_aggressor = None;
    let mut street_aggressor = None;
    let mut out = Vec::new();
    for rec in &hand.history {
        if rec.street != street {
            street = rec.street;
            street_bet.clear();
            street_aggressor = None;
        }
        // Histories omit blind posts, but the first preflop record retains them.
        // Seed before sampling, including a blind that checks and adds no chips.
        if street == Street::Preflop && !street_bet.contains_key(&rec.seat) {
            let posted = rec.bet_before.max(0);
            invested.insert(rec.seat, posted);
            street_bet.insert(rec.seat, posted);
        }
        let Some(name) = names.get(&rec.seat) else { continue };
        if !exclude.contains(name) && stacks.contains_key(&rec.seat) {
            let profile = models.profile(name);
            let live: Vec<usize> = order.iter().copied().filter(|s| !folded.contains(s)).collect();
            let ctx = ResponseContext {
                street,
                to_call: rec.to_call_before,
                pot_before: rec.pot_before,
                stack_behind: stacks[&rec.seat] - invested.get(&rec.seat).copied().unwrap_or(0),
                bb: hand.bb,
                active_players: live.len(),
                in_position: live.last() == Some(&rec.seat),
                preflop_raises,
                was_preflop_aggressor: pf_aggressor == Some(rec.seat),
                facing_cbet: street == Street::Flop
                    && rec.to_call_before > 0
                    && street_aggressor.is_some()
                    && street_aggressor == pf_aggressor,
                prior_postflop_calls: prior_postflop_calls(&hand.history, rec.seat, street),
                board: &hand.board[..street.board_len().min(hand.board.len())],
                profile: &profile,
            };
            let label = match rec.kind {
                ActionKind::Fold if rec.to_call_before > 0 => 0,
                ActionKind::Fold | ActionKind::Check | ActionKind::Call => 1,
                _ if aggressive(rec) => 2,
                _ => 1,
            };
            out.push((
                (*name).clone(),
                Sample { x: features_for(&ctx, feature_set), mask: mask(&ctx), label, weight: 1.0 },
                baseline(&ctx),
            ));
        }
        let add = if rec.to > 0 { rec.to - street_bet.get(&rec.seat).copied().unwrap_or(0) } else { 0 };
        if add > 0 {
            *invested.entry(rec.seat).or_default() += add;
            street_bet.insert(rec.seat, rec.to);
        }
        if rec.kind == ActionKind::Fold {
            folded.push(rec.seat);
        }
        if aggressive(rec) {
            street_aggressor = Some(rec.seat);
        }
        if aggressive(rec) && street == Street::Preflop {
            preflop_raises += 1;
            pf_aggressor = Some(rec.seat);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelStore;
    use sv10_engine::engine::ActionRecord;

    fn action(seat: usize, street: Street, kind: ActionKind) -> ActionRecord {
        ActionRecord {
            seat,
            street,
            kind,
            to: 20,
            pot_before: 100,
            to_call_before: 20,
            bet_before: 0,
            full_raise: false,
            think_ms: None,
            street_open: false,
        }
    }

    #[test]
    fn feature_vector_matches_the_network_layout_and_is_finite() {
        let prof = ModelStore::default().profile("x");
        let board: Vec<Card> = ["Ah", "7d", "2c"].iter().map(|c| Card::parse(c).unwrap()).collect();
        for (street, b, to_call) in [(Street::Preflop, &board[..0], 40), (Street::Flop, &board[..], 0), (Street::Flop, &board[..], 120)] {
            let ctx = ResponseContext {
                street,
                to_call,
                pot_before: 180,
                stack_behind: 4_000,
                bb: 20,
                active_players: 3,
                in_position: true,
                preflop_raises: 1,
                was_preflop_aggressor: false,
                facing_cbet: street == Street::Flop && to_call > 0,
                prior_postflop_calls: 0,
                board: b,
                profile: &prof,
            };
            let f = features(&ctx);
            assert_eq!(f.len(), N_FEATURES, "a layout change must bump N_FEATURES (old networks are then ignored)");
            assert!(f.iter().all(|x| x.is_finite()), "{street:?}: {f:?}");
            assert_eq!(mask(&ctx).len(), 3);
            let mut candidate_ctx = ctx.clone();
            candidate_ctx.prior_postflop_calls = 9;
            let candidate = features_for(&candidate_ctx, ResponseFeatureSet::PriorStreetCalls38);
            assert_eq!(candidate.len(), 38);
            assert_eq!(candidate[37], 1.0);
            let old = features_for(&candidate_ctx, ResponseFeatureSet::Incumbent37);
            assert_eq!(old.len(), INCUMBENT_FEATURES);
            assert_eq!(&candidate[..INCUMBENT_FEATURES], &old[..], "the 38-input layout extends the 37 unchanged");
            assert_eq!(ResponseFeatureSet::for_inputs(37), Some(ResponseFeatureSet::Incumbent37));
            assert_eq!(ResponseFeatureSet::for_inputs(38), Some(ResponseFeatureSet::PriorStreetCalls38));
            assert_eq!(ResponseFeatureSet::for_inputs(26), None);
        }
    }

    #[test]
    fn prior_street_calls_are_named_versioned_and_ignore_current_street() {
        let history = vec![
            action(0, Street::Flop, ActionKind::Call),
            action(1, Street::Flop, ActionKind::Call),
            action(0, Street::Turn, ActionKind::Call),
            action(0, Street::River, ActionKind::Call),
        ];
        let hand = HandSummary {
            players: vec![(0, "villain".into()), (1, "other".into())],
            button: 1,
            bb: 20,
            history,
            board: ["Ah", "7d", "2c", "Ts", "3s"].iter().map(|card| Card::parse(card).unwrap()).collect(),
            stacks: vec![(0, 2_000), (1, 2_000)],
            shown: vec![],
        };
        let models = ModelStore::default();
        // Default extraction follows the versioned production layout.
        let incumbent = samples_from_hand(&hand, &models, &[]);
        let explicit_incumbent = samples_from_hand_for(&hand, &models, &[], ResponseFeatureSet::PRODUCTION);
        let candidate = samples_from_hand_for(&hand, &models, &[], ResponseFeatureSet::PriorStreetCalls38);

        assert_eq!(incumbent.len(), explicit_incumbent.len());
        for ((left, left_base), (right, right_base)) in incumbent.iter().zip(&explicit_incumbent) {
            assert_eq!(
                (&left.x, &left.mask, left.label, left.weight, left_base),
                (&right.x, &right.mask, right.label, right.weight, right_base)
            );
        }
        let villain = candidate.iter().filter(|(sample, _)| sample.x.len() == 38).map(|(sample, _)| sample.x[37]).collect::<Vec<_>>();
        assert_eq!(villain, vec![0.0, 0.0, 1.0 / 3.0, 2.0 / 3.0]);
        assert_eq!(prior_postflop_calls(&hand.history, 0, Street::Turn), 1);
        assert_eq!(prior_postflop_calls(&hand.history, 0, Street::River), 2);
        assert_eq!(prior_postflop_calls(&hand.history, 1, Street::River), 1);
    }
}

#[cfg(test)]
mod blind_stack_tests {
    use super::*;
    use sv10_engine::engine::{Action, Hand};
    use sv10_rng::{SeedableRng, rngs::SmallRng};

    #[test]
    fn training_stack_features_match_engine_chips_after_blinds_and_checks() {
        let mut rng = SmallRng::seed_from_u64(801);
        let mut hand = Hand::new(&[200; 3], 0, 10, 20, &mut rng);
        let mut contexts = Vec::new();
        for action in [Action::Call, Action::Call, Action::Check, Action::Check, Action::Check, Action::Check] {
            let actor = hand.actor().unwrap();
            let stack = hand.seats[actor].stack;
            let pot = hand.pot();
            contexts.push((actor, (1.0 + stack as f32 / pot as f32).ln()));
            hand.apply(action).unwrap();
        }
        let summary = HandSummary {
            players: (0..3).map(|i| (i, format!("p{i}"))).collect(),
            button: 0,
            bb: 20,
            history: hand.history.clone(),
            board: hand.board.clone(),
            stacks: vec![(0, 200), (1, 200), (2, 200)],
            shown: vec![],
        };
        let samples = named_samples_from_hand_for(&summary, &ModelStore::default(), &[], ResponseFeatureSet::PRODUCTION);
        assert_eq!(samples.len(), contexts.len());
        for ((name, sample, _), (actor, expected)) in samples.iter().zip(contexts) {
            assert_eq!(name, &format!("p{actor}"));
            assert!((sample.x[7] - expected).abs() < 1e-6, "{name}: training {} versus actual engine {expected}", sample.x[7]);
        }
    }
}

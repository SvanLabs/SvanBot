//! Opponent responses to hero actions: fold probabilities, equity realization and raise sizes.

use super::*;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub(super) struct Responder<'a> {
    pub(super) seat: usize,
    pub(super) stack_total: i64,
    pub(super) bet: i64,
    pub(super) profile: Profile,
    pub(super) range: &'a Range,
    pub(super) acted_this_street: bool,
    /// Called another player's all-in earlier on this street (0201).
    pub(super) called_all_in: bool,
}

/// Highest fold probability for a responder who has already called an all-in this street (0201):
/// stored hands show such callers fold 5 of 18 times (0.28) when re-raised, while the model had
/// priced one at 0.99 from a 10-hand fold-to-4-bet stat times the preflop fold scale.
pub(super) const CALLED_ALL_IN_FOLD_CAP: f64 = 0.30;

/// Whether `seat` called another player's all-in earlier on the current street.
fn called_an_all_in(sit: &Situation, seat: usize) -> bool {
    let mut all_in_seen = false;
    for r in sit.history.iter().filter(|r| r.street == sit.street) {
        if r.seat != seat && r.kind == sv10_engine::engine::ActionKind::AllIn {
            all_in_seen = true;
        } else if r.seat == seat && all_in_seen && r.kind == sv10_engine::engine::ActionKind::Call {
            return true;
        }
    }
    false
}

impl<'a> Responder<'a> {
    /// One responder per live opponent: priced profile, estimated range and whether they already
    /// acted this street (preflop, a blind-or-better bet counts only with a history record).
    /// Returns the responders with the acted set `check_lookahead` also needs.
    pub(super) fn for_table(
        sit: &'a Situation,
        models: &'a ModelStore,
        ranges: &'a HashMap<usize, Range>,
    ) -> (Vec<Responder<'a>>, HashSet<usize>) {
        let live: Vec<_> = sit.live_opponents().cloned().collect();
        let acted: HashSet<usize> = sit.history.iter().filter(|r| r.street == sit.street).map(|r| r.seat).collect();
        let responders = live
            .iter()
            .map(|p| Responder {
                seat: p.seat,
                stack_total: p.stack + p.bet,
                bet: p.bet,
                profile: models.profile(&p.name),
                range: &ranges[&p.seat],
                acted_this_street: acted.contains(&p.seat)
                    || (sit.street == Street::Preflop && p.bet > 0 && p.bet >= sit.bb && acted.contains(&p.seat)),
                called_all_in: called_an_all_in(sit, p.seat),
            })
            .collect();
        (responders, acted)
    }
}

/// One internally consistent response to one proposed raise-to amount.
pub(super) struct ResponderPrice {
    pub(super) fold_prob: f64,
    pub(super) continue_prob: f64,
    pub(super) raise_given_continue: f64,
    pub(super) continue_range: Range,
}

/// Deep pricing module: callers supply one situation and ask for all responders at one raise-to.
/// Fold frequency, continuation range and raise-back share are derived together from that amount.
pub(super) struct ResponsePricing<'a> {
    sit: &'a Situation,
    params: &'a Params,
    nn: Option<&'a sv10_nn::nn::Mlp>,
    hero_view: Profile,
    hero_prior: Range,
    strength: Arc<Vec<f32>>,
    passive_line: bool,
}

impl<'a> ResponsePricing<'a> {
    pub(super) fn new(
        sit: &'a Situation,
        models: &'a ModelStore,
        params: &'a Params,
        nn: Option<&'a sv10_nn::nn::Mlp>,
        passive_line: bool,
    ) -> Self {
        let hero_view = models.hero_seen_view(sit.hero().name.as_str(), params.hero_image);
        let hero_prior = perceived_range(sit, hero_view.clone(), &params.range);
        Self { sit, params, nn, hero_view, hero_prior, strength: board_strengths(&sit.board), passive_line }
    }

    pub(super) fn for_raise_to(&self, responders: &[Responder<'_>], to: i64) -> Vec<ResponderPrice> {
        let lik = aggressive_likelihood(self.sit, &self.hero_view, to, &self.hero_prior, &self.strength, &self.params.range);
        let mut hero_bet_range = self.hero_prior.clone();
        for (weight, likelihood) in hero_bet_range.w.iter_mut().zip(lik) {
            *weight *= likelihood;
        }
        let eq_table = equity_vs_histogram(&strength_histogram(&hero_bet_range, &self.strength), self.sit.street);
        responders.iter().map(|responder| self.price_one(responder, responders.len(), to, &eq_table)).collect()
    }

    fn price_one(
        &self,
        responder: &Responder<'_>,
        responder_count: usize,
        to: i64,
        eq_table: &[f32; sv10_model::oprange::STRENGTH_BINS],
    ) -> ResponderPrice {
        if responder.stack_total <= responder.bet {
            return ResponderPrice {
                fold_prob: 0.0,
                continue_prob: 1.0,
                raise_given_continue: 0.0,
                continue_range: responder.range.clone(),
            };
        }
        let pot = self.sit.pot as f64;
        let add = (to - self.sit.hero().bet) as f64;
        let cost = ((to.min(responder.stack_total)) - responder.bet).max(0) as f64;
        let final_pot = pot + add + cost;
        let lightness = if self.sit.street == Street::Preflop {
            ((responder.profile.call_open - defaults::CALL_OPEN) * 0.5).clamp(-0.1, 0.2)
        } else {
            let street = self.sit.street.index() - 1;
            ((defaults::FOLD_VS_BET[street] - responder.profile.fold_vs_bet[street]) * 0.6).clamp(-0.12, 0.25)
        };
        let street_price = [0.8f32, 0.85, 0.92, 1.0][self.sit.street.index()];
        let need = (cost / final_pot) as f32 * street_price - lightness;
        let mut profile_fold = fold_prob(self.sit, responder, to, self.params);
        let mut neural_raise_share = None;
        if let Some((net, layout)) = self
            .nn
            .and_then(|network| sv10_model::features::ResponseFeatureSet::for_inputs(network.input_size()).map(|layout| (network, layout)))
        {
            let preflop_aggressor =
                self.sit.history.iter().rfind(|record| record.street == Street::Preflop && aggressive(record)).map(|record| record.seat);
            let context = sv10_model::features::ResponseContext {
                street: self.sit.street,
                to_call: cost as i64,
                pot_before: (pot + add) as i64,
                stack_behind: responder.stack_total - responder.bet,
                bb: self.sit.bb,
                active_players: responder_count + 1,
                in_position: self.sit.in_position(responder.seat),
                preflop_raises: preflop_raises(self.sit) + usize::from(self.sit.street == Street::Preflop),
                was_preflop_aggressor: preflop_aggressor == Some(responder.seat),
                facing_cbet: self.sit.street == Street::Flop && preflop_aggressor == Some(self.sit.hero_seat),
                prior_postflop_calls: sv10_model::features::prior_postflop_calls(&self.sit.history, responder.seat, self.sit.street),
                board: &self.sit.board,
                profile: &responder.profile,
            };
            let mask = sv10_model::features::mask(&context);
            let probabilities = sv10_model::residual::apply_ratio(
                &net.predict(&sv10_model::features::features_for(&context, layout), &mask),
                &mask,
                responder.profile.response_ratio,
            );
            if probabilities[0].is_finite() {
                profile_fold = (0.7 * probabilities[0] as f64 + 0.3 * profile_fold).clamp(0.0, 0.99);
            }
            if probabilities[1].is_finite() && probabilities[2].is_finite() && probabilities[1] + probabilities[2] > 0.0 {
                neural_raise_share = Some((probabilities[2] / (probabilities[1] + probabilities[2])) as f64);
            }
        }
        let (model_continue, mut continue_range) = if self.sit.street == Street::Preflop {
            villain_continue_preflop(
                responder.range,
                preflop_raise_fraction(self.sit, &self.hero_view, to),
                need,
                (1.0 - profile_fold) as f32,
            )
        } else {
            villain_continue(responder.range, &self.strength, eq_table, need, (1.0 - profile_fold) as f32)
        };
        let street_scale = if self.sit.street == Street::Preflop { self.params.preflop_fold_scale } else { 1.0 };
        let mut fold = ((1.0 - model_continue as f64) * self.params.fold_scale * street_scale).clamp(0.0, 0.99);
        if self.passive_line && self.params.passive_fold_bonus != 0.0 {
            fold = (fold + self.params.passive_fold_bonus).clamp(0.0, 0.99);
        }
        let current_street_aggression = self
            .sit
            .history
            .iter()
            .filter(|record| record.street == self.sit.street && record.seat == responder.seat && aggressive(record))
            .count();
        if current_street_aggression > 0 {
            fold = fold.min(0.45 / (1.0 + current_street_aggression as f64).powf(1.4));
        }
        if self.sit.street != Street::Preflop {
            fold = shift_fold_logit(fold, self.params.fold_logit_shift[self.sit.street.index() - 1]);
            // Per-opponent fold calibration (0214), measured on heads-up postflop bets only.
            if responder_count == 1 && responder.profile.fold_logit_offset != 0.0 {
                fold = shift_fold_logit(fold, responder.profile.fold_logit_offset as f64);
            }
        }
        if responder.called_all_in {
            fold = fold.min(CALLED_ALL_IN_FOLD_CAP);
        }
        let continue_prob = 1.0 - fold;
        rescale_range_mass(&mut continue_range, responder.range, continue_prob);
        let max_to = self.sit.max_raise_to.unwrap_or(0);
        let raise_given_continue = if to >= max_to || responder.stack_total <= to {
            0.0
        } else if self.sit.street == Street::Preflop {
            preflop_reraise_share(self.sit, responder)
        } else {
            let street = self.sit.street.index() - 1;
            let stat = (responder.profile.raise_vs_bet[street] as f64 / (1.0 - responder.profile.fold_vs_bet[street] as f64).max(0.05))
                .clamp(0.0, 0.8);
            neural_raise_share.map(|neural| 0.6 * neural + 0.4 * stat).unwrap_or(stat).clamp(0.0, 0.8)
        };
        ResponderPrice { fold_prob: fold, continue_prob, raise_given_continue, continue_range }
    }
}

/// Move a fold probability by `shift` on the logit scale (0156); 0 returns it unchanged, and the
/// result stays inside the 0..0.99 band every fold estimate uses.
pub(super) fn shift_fold_logit(fold: f64, shift: f64) -> f64 {
    if shift == 0.0 || fold <= 0.0 {
        return fold;
    }
    let p = fold.clamp(1e-4, 1.0 - 1e-4);
    let logit = (p / (1.0 - p)).ln() + shift;
    (1.0 / (1.0 + (-logit).exp())).clamp(0.0, 0.99)
}

fn rescale_range_mass(range: &mut Range, source: &Range, continue_prob: f64) {
    let target = source.total() * continue_prob;
    let have = range.total();
    if have <= 0.0 && target > 0.0 {
        *range = source.clone();
    }
    let have = range.total();
    if have > 0.0 {
        let scale = (target / have) as f32;
        range.w.iter_mut().for_each(|weight| *weight *= scale);
    }
}

pub(super) fn preflop_raises(sit: &Situation) -> usize {
    sit.history.iter().filter(|r| r.street == Street::Preflop && aggressive(r)).count()
}

pub(super) fn limpers(sit: &Situation) -> usize {
    let mut n = 0;
    for r in sit.history.iter().filter(|r| r.street == Street::Preflop) {
        if aggressive(r) {
            return 0;
        }
        if r.kind == sv10_engine::engine::ActionKind::Call {
            n += 1;
        }
    }
    n
}

/// Probability that one opponent folds to hero raising the street bet to `to`.
pub(super) fn fold_prob(sit: &Situation, r: &Responder, to: i64, params: &Params) -> f64 {
    if r.stack_total <= r.bet {
        return 0.0; // already all-in
    }
    let p = &r.profile;
    let cur = sit.current_bet();
    let hero_bet = sit.hero().bet;
    if sit.street == Street::Preflop {
        let bb = sit.bb as f64;
        let raises = preflop_raises(sit);
        let to_bb = to as f64 / bb;
        let f = match raises {
            0 => {
                if r.bet >= sit.bb && r.acted_this_street {
                    // Limper facing an iso-raise; looser players (by their own VPIP) continue more (0101).
                    let looseness = (p.vpip as f64 / defaults::VPIP as f64).clamp(0.5, 2.0).powf(params.profile_response_weight);
                    let cont = (0.55 * (4.0 / to_bb).sqrt() * looseness).clamp(0.15, 0.9);
                    1.0 - cont
                } else {
                    let cont = ((p.call_open + p.three_bet) as f64 * (2.5 / to_bb).powf(0.35)).clamp(0.02, 0.9);
                    1.0 - cont
                }
            }
            1 => {
                if r.acted_this_street && r.bet == cur {
                    let standard = cur as f64 * 3.3;
                    let cont = (1.0 - p.fold_to_3bet as f64) * (standard / to as f64).powf(0.35).min(1.3);
                    1.0 - cont
                } else {
                    1.0 - (p.three_bet as f64 * 0.8).clamp(0.01, 0.3)
                }
            }
            2 => {
                if r.acted_this_street && r.bet == cur {
                    p.fold_to_4bet as f64
                } else {
                    0.97
                }
            }
            // Players who have already 4-bet or 5-bet almost never fold to more.
            _ => 0.2,
        };
        return f.clamp(0.0, 0.99);
    }
    let pot_after_call = (sit.pot + (cur - hero_bet)).max(1) as f64;
    let frac = (to - cur) as f64 / pot_after_call;
    let mut f = p.fold_to_bet(sit.street, frac);
    // A flop c-bet: the player's own fold-to-c-bet rate, scaled by size like any bet (0101).
    let pf_aggressor = sit.history.iter().rfind(|h| h.street == Street::Preflop && aggressive(h)).map(|h| h.seat);
    if sit.street == Street::Flop && cur == 0 && pf_aggressor == Some(sit.hero_seat) && params.profile_response_weight > 0.0 {
        let generic = p.fold_to_bet(sit.street, 0.66);
        let shift = (p.fold_to_cbet as f64 - generic) * params.profile_response_weight;
        f = (f + shift).clamp(0.01, 0.98);
    }
    if cur > 0 {
        f += params.raise_fold_bonus;
    }
    f.clamp(0.01, 0.98)
}

/// Share of one opponent's continuing hands that re-raise hero's preflop raise, from their
/// tendencies: players facing an open 3-bet, the opener facing hero's 3-bet 4-bets, a limper
/// rarely limp-raises, and anyone facing a 4-bet or more mostly jams or folds.
pub(super) fn preflop_reraise_share(sit: &Situation, r: &Responder) -> f64 {
    let p = &r.profile;
    let cur = sit.current_bet();
    let share = match preflop_raises(sit) {
        0 if r.acted_this_street => 0.1,
        0 => p.three_bet as f64 / (p.three_bet + p.call_open).max(0.02) as f64,
        1 if r.acted_this_street && r.bet == cur => p.four_bet as f64 / (1.0 - p.fold_to_3bet).max(0.05) as f64,
        1 => 0.3,
        _ => 0.4,
    };
    share.clamp(0.0, 0.8)
}

/// Share of raw equity a hand actually realizes if the hand continues:
/// weak hands get pushed off their equity, strong hands extract extra.
pub(super) fn realize(street: Street, ip: bool, eq: f64, weight: f64) -> f64 {
    let street_weight = [1.0, 0.75, 0.4, 0.0][street.index()];
    let base = 0.4 + 1.2 * eq - 0.55 * eq * eq;
    let pos = if ip { 1.0 } else { 0.9 };
    1.0 + (base * pos - 1.0) * weight * street_weight
}

pub(super) fn raise_targets(sit: &Situation, params: &Params, ip: bool, eq: f64) -> Vec<i64> {
    let (Some(min_to), Some(max_to)) = (sit.min_raise_to, sit.max_raise_to) else {
        return vec![];
    };
    // A malformed legal range (min above max) must not panic inside clamp.
    let min_to = min_to.min(max_to);
    let cur = sit.current_bet();
    let bb = sit.bb as f64;
    let mut t: Vec<f64> = Vec::new();
    if sit.street == Street::Preflop {
        match preflop_raises(sit) {
            0 => {
                let short = sit.effective_stack() as f64 / bb <= params.preflop_jam_bb;
                let open = if short { params.short_open_bb } else { params.open_bb };
                let base = open + params.limper_bb * limpers(sit) as f64;
                t.push(base * bb);
                t.push((base + 1.5) * bb);
            }
            1 => {
                let callers = sit.players.iter().filter(|p| p.bet == cur && p.seat != sit.hero_seat).count().saturating_sub(1);
                let m = if ip { params.three_bet_ip } else { params.three_bet_oop };
                t.push(cur as f64 * m + callers as f64 * cur as f64);
            }
            _ => t.push(cur as f64 * params.four_bet),
        }
    } else {
        let hero_bet = sit.hero().bet;
        let pot_after_call = (sit.pot + (cur - hero_bet)) as f64;
        for &s in &params.bet_sizes {
            t.push(cur as f64 + s * pot_after_call);
        }
    }
    let mut out: Vec<i64> = t
        .into_iter()
        .map(|x| (x.round() as i64).clamp(min_to, max_to))
        .map(|x| if x as f64 >= max_to as f64 * 0.7 { max_to } else { x })
        .collect();
    // Response models are least reliable for huge sizes: only consider jamming
    // when the stack is small relative to the pot.
    let hero_bet = sit.hero().bet;
    let jam_size = (max_to - cur) as f64;
    let pot_after_call = (sit.pot + (cur - hero_bet)).max(1) as f64;
    let jam_ok = if sit.street == Street::Preflop {
        let eff_bb = sit.effective_stack() as f64 / bb;
        eff_bb <= params.preflop_jam_bb
            || (preflop_raises(sit) >= 2 && eff_bb <= 2.0 * params.preflop_jam_bb)
            || jam_size <= pot_after_call * 3.0
    } else {
        jam_size <= pot_after_call * params.jam_pot_ratio || (eq >= 0.93 && jam_size <= pot_after_call * 5.0)
    };
    if jam_ok {
        out.push(max_to);
    } else {
        out.retain(|&x| x != max_to || (max_to - cur) as f64 <= pot_after_call * params.jam_pot_ratio);
    }
    if out.is_empty() {
        out.push(min_to);
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {

    #[test]
    fn fold_logit_shift_moves_folds_monotonically_and_zero_is_identity() {
        for fold in [0.0, 0.05, 0.3, 0.6, 0.99] {
            assert_eq!(shift_fold_logit(fold, 0.0), fold);
        }
        assert_eq!(shift_fold_logit(0.0, 1.0), 0.0, "a responder who never folds stays at zero");
        let base = 0.30;
        let river = shift_fold_logit(base, -0.74);
        assert!((river - 0.1698).abs() < 1e-3, "river -0.74 logit on 30%: {river}");
        assert!(shift_fold_logit(base, 0.21) > base);
        assert!(shift_fold_logit(0.98, 3.0) <= 0.99);
    }

    use super::*;
    use sv10_engine::engine::{Action, ActionKind, ActionRecord, Hand};
    use sv10_rng::SeedableRng;
    use sv10_rng::rngs::SmallRng;

    fn prices(sit: &Situation) -> Vec<(f64, ResponderPrice)> {
        let models = ModelStore::default();
        let params = Params { fold_scale: 0.7, preflop_fold_scale: 0.9, passive_fold_bonus: 0.03, ..Params::default() };
        let ranges = estimate_ranges(sit, &models, &params.range);
        let (responders, _) = Responder::for_table(sit, &models, &ranges);
        let to = sit.min_raise_to.expect("raise available").min(sit.max_raise_to.expect("raise maximum"));
        let pricing = ResponsePricing::new(sit, &models, &params, None, false);
        responders.iter().zip(pricing.for_raise_to(&responders, to)).map(|(responder, price)| (responder.range.total(), price)).collect()
    }

    fn assert_invariants(sit: &Situation) {
        for (source_mass, price) in prices(sit) {
            assert!(price.fold_prob.is_finite() && (0.0..=1.0).contains(&price.fold_prob));
            assert!(price.continue_prob.is_finite() && (0.0..=1.0).contains(&price.continue_prob));
            assert!(price.raise_given_continue.is_finite() && (0.0..=1.0).contains(&price.raise_given_continue));
            assert!((price.fold_prob + price.continue_prob - 1.0).abs() < 1e-12);
            let expected_mass = source_mass * price.continue_prob;
            assert!((price.continue_range.total() - expected_mass).abs() < 2e-4, "continuing range mass disagrees with probability");
        }
    }

    #[test]
    fn short_stacks_open_to_their_own_size_and_the_jam_threshold_is_a_knob() {
        // 0171: an unopened UTG spot at 25 bb (short) and 100 bb (deep), blinds 10/20.
        let names = (0..4).map(|seat| format!("p{seat}")).collect::<Vec<_>>();
        let targets = |stack: i64, params: &Params| {
            let hand = Hand::new(&[stack; 4], 0, 10, 20, &mut SmallRng::seed_from_u64(9));
            let sit = Situation::from_hand(&hand, hand.actor().unwrap(), &names);
            raise_targets(&sit, params, false, 0.5)
        };
        let defaults = Params::default();
        assert!(targets(500, &defaults).contains(&50) && targets(2_000, &defaults).contains(&50), "defaults open 2.5 bb at every depth");
        assert!(targets(500, &defaults).contains(&500), "and may jam at 25 bb, as before");
        let short = Params { short_open_bb: 2.0, ..Params::default() };
        assert!(targets(500, &short).contains(&40) && !targets(500, &short).contains(&50), "{:?}", targets(500, &short));
        assert!(targets(2_000, &short).contains(&50), "deep stacks keep open_bb");
        let tight = Params { preflop_jam_bb: 20.0, ..Params::default() };
        assert!(!targets(500, &tight).contains(&500), "25 bb is above a 20 bb jam threshold");
        assert!(targets(500, &tight).contains(&50), "above the threshold the stack is not short: open_bb");
    }

    #[test]
    fn responder_price_invariants_hold_preflop_flop_and_multiway() {
        let mut rng = SmallRng::seed_from_u64(45);
        let preflop = Hand::new(&[2_000; 4], 0, 10, 20, &mut rng);
        let names = (0..4).map(|seat| format!("p{seat}")).collect::<Vec<_>>();
        assert_invariants(&Situation::from_hand(&preflop, preflop.actor().unwrap(), &names));

        let mut flop = Hand::new(&[2_000; 3], 0, 10, 20, &mut rng);
        flop.apply(Action::Call).unwrap();
        flop.apply(Action::Call).unwrap();
        flop.apply(Action::Check).unwrap();
        let names = (0..3).map(|seat| format!("p{seat}")).collect::<Vec<_>>();
        assert_eq!(flop.street, Street::Flop);
        assert_invariants(&Situation::from_hand(&flop, flop.actor().unwrap(), &names));
    }

    #[test]
    fn a_caller_of_an_all_in_is_not_priced_to_fold_to_a_squeeze() {
        // A predicted 99% fold is no edge against a player who just called an all-in: SuraGunnar
        // jammed 6-3o for 490 bb into one and was called (stored hands: they fold 5 of 18 times).
        let mut rng = SmallRng::seed_from_u64(5);
        let mut hand = Hand::new(&[10_000, 1_600, 24_000], 0, 10, 20, &mut rng);
        hand.apply(Action::RaiseTo(760)).unwrap(); // hero (button) opens
        hand.apply(Action::AllIn).unwrap(); // small blind shoves
        hand.apply(Action::Call).unwrap(); // big blind calls the shove
        assert_eq!(hand.actor(), Some(0));
        let names: Vec<String> = ["hero", "shover", "caller"].iter().map(|s| s.to_string()).collect();
        let sit = Situation::from_hand(&hand, 0, &names);
        // A profile that "always folds to a 4-bet" on a tiny sample, the case that fooled the model.
        let mut models = ModelStore::default();
        let st = sv10_model::model::PlayerStats {
            hands: 5_000.0,
            fold_to_4bet: sv10_model::model::Counter { opp: 10.0, hit: 10.0 },
            ..Default::default()
        };
        models.players.insert("caller".into(), st);
        let params = Params::default();
        let ranges = estimate_ranges(&sit, &models, &params.range);
        let (responders, _) = Responder::for_table(&sit, &models, &ranges);
        let pricing = ResponsePricing::new(&sit, &models, &params, None, false);
        let max_to = sit.max_raise_to.unwrap();
        for (responder, price) in responders.iter().zip(pricing.for_raise_to(&responders, max_to)) {
            if responder.seat == 2 {
                assert!(responder.called_all_in, "the big blind called an all-in");
                assert!(price.fold_prob <= CALLED_ALL_IN_FOLD_CAP + 1e-12, "fold {}", price.fold_prob);
            }
        }
    }

    #[test]
    fn all_in_responders_cannot_fold_or_raise() {
        let sit = sv10_engine::situation::fixtures::river_jam_with_all_ins();
        let models = ModelStore::default();
        let params = Params::default();
        let ranges = estimate_ranges(&sit, &models, &params.range);
        let (responders, _) = Responder::for_table(&sit, &models, &ranges);
        let pricing = ResponsePricing::new(&sit, &models, &params, None, false);
        for (responder, price) in responders.iter().zip(pricing.for_raise_to(&responders, 20)) {
            if responder.stack_total <= responder.bet {
                assert_eq!((price.fold_prob, price.continue_prob, price.raise_given_continue), (0.0, 1.0, 0.0));
            }
        }
    }

    #[test]
    fn prior_street_aggression_does_not_trigger_current_street_fold_cap() {
        let mut sit = sv10_engine::situation::fixtures::river_jam_with_all_ins();
        sit.history.clear();
        let baseline = sit.clone();
        sit.history.push(ActionRecord {
            seat: 0,
            street: Street::Flop,
            kind: ActionKind::Raise,
            to: 100,
            pot_before: 100,
            to_call_before: 0,
            bet_before: 0,
            full_raise: true,
            think_ms: None,
            street_open: false,
        });
        let mut range = Range::default();
        range.normalize();
        let models = ModelStore::default();
        let profile = models.profile("villain");
        let responder = || Responder {
            seat: 0,
            stack_total: 10_337,
            bet: 0,
            profile: profile.clone(),
            range: &range,
            acted_this_street: false,
            called_all_in: false,
        };
        let params = Params::default();
        let before = ResponsePricing::new(&baseline, &models, &params, None, false).for_raise_to(&[responder()], 1_000);
        let after = ResponsePricing::new(&sit, &models, &params, None, false).for_raise_to(&[responder()], 1_000);
        assert!((before[0].fold_prob - after[0].fold_prob).abs() < 1e-12);
    }
}

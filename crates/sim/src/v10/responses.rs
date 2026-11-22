//! How v10.0.2 priced the table's answer to a raise: per opponent, the chance of a fold, of a
//! call, and of a raise back, and the range that goes on.

use std::collections::{BTreeMap, BTreeSet};

use super::cards::Range;
use super::continuation::{
    STRENGTH_BINS, equity_vs_histogram, preflop_raise_fraction, strength_histogram,
    villain_continue, villain_continue_preflop,
};
use super::params::Params;
use super::profile::{Profile, defaults};
use super::ranges::{Boards, aggressive_likelihood, perceived_range};
use super::situation::{ActionKind, Situation, Street};

/// One live opponent, as the pricing sees it.
pub struct Responder<'a> {
    pub seat: usize,
    pub stack_total: i64,
    pub bet: i64,
    pub profile: Profile,
    pub range: &'a Range,
    pub acted_this_street: bool,
    pub called_all_in: bool,
}

/// A seat that called an all-in this street folds to a raise this often at most.
const CALLED_ALL_IN_FOLD_CAP: f64 = 0.30;

fn called_an_all_in(sit: &Situation, seat: usize) -> bool {
    let mut all_in_seen = false;
    for r in sit.history.iter().filter(|r| r.street == sit.street) {
        if r.seat != seat && r.kind == ActionKind::AllIn {
            all_in_seen = true;
        } else if r.seat == seat && all_in_seen && r.kind == ActionKind::Call {
            return true;
        }
    }
    false
}

impl<'a> Responder<'a> {
    /// Every live opponent in seat order, and the seats that have acted this street.
    pub fn for_table(
        sit: &Situation,
        profile: &Profile,
        ranges: &'a BTreeMap<usize, Range>,
    ) -> (Vec<Responder<'a>>, BTreeSet<usize>) {
        let acted: BTreeSet<usize> = sit
            .history
            .iter()
            .filter(|r| r.street == sit.street)
            .map(|r| r.seat)
            .collect();
        let responders = sit
            .live_opponents()
            .map(|p| Responder {
                seat: p.seat,
                stack_total: p.stack + p.bet,
                bet: p.bet,
                profile: profile.clone(),
                range: &ranges[&p.seat],
                acted_this_street: acted.contains(&p.seat),
                called_all_in: called_an_all_in(sit, p.seat),
            })
            .collect();
        (responders, acted)
    }

    fn all_in(&self) -> bool {
        self.stack_total <= self.bet
    }
}

/// One opponent's answer to a raise.
pub struct ResponderPrice {
    pub fold_prob: f64,
    pub continue_prob: f64,
    pub raise_given_continue: f64,
    pub continue_range: Range,
}

/// The pricing of the table's answers at one spot, for any raise size.
pub struct ResponsePricing<'a> {
    sit: &'a Situation,
    params: &'a Params,
    hero_view: Profile,
    hero_prior: Range,
    strength: Vec<f32>,
    passive_line: bool,
}

impl<'a> ResponsePricing<'a> {
    pub fn new(
        sit: &'a Situation,
        profile: &Profile,
        params: &'a Params,
        passive_line: bool,
        boards: &mut Boards,
    ) -> Self {
        let hero_prior = perceived_range(sit, profile, &params.range, boards);
        let strength = boards.info(&sit.board).strength.clone();
        Self {
            sit,
            params,
            hero_view: profile.clone(),
            hero_prior,
            strength,
            passive_line,
        }
    }

    pub fn for_raise_to(
        &self,
        responders: &[Responder<'_>],
        to: i64,
        boards: &mut Boards,
    ) -> Vec<ResponderPrice> {
        let lik = aggressive_likelihood(self.sit, &self.hero_view, to, &self.params.range, boards);
        let mut hero_bet_range = self.hero_prior.clone();
        for (weight, likelihood) in hero_bet_range.w.iter_mut().zip(lik) {
            *weight *= likelihood;
        }
        let eq_table = equity_vs_histogram(
            &strength_histogram(&hero_bet_range, &self.strength),
            self.sit.street,
        );
        responders
            .iter()
            .map(|r| self.price_one(r, responders.len(), to, &eq_table))
            .collect()
    }

    fn price_one(
        &self,
        responder: &Responder<'_>,
        responder_count: usize,
        to: i64,
        eq_table: &[f32; STRENGTH_BINS],
    ) -> ResponderPrice {
        if responder.all_in() {
            return ResponderPrice {
                fold_prob: 0.0,
                continue_prob: 1.0,
                raise_given_continue: 0.0,
                continue_range: responder.range.clone(),
            };
        }
        let sit = self.sit;
        let p = &responder.profile;
        let pot = sit.pot as f64;
        let add = (to - sit.hero().bet) as f64;
        let cost = (to.min(responder.stack_total) - responder.bet).max(0) as f64;
        let final_pot = pot + add + cost;
        let lightness = if sit.street == Street::Preflop {
            ((p.call_open - defaults::CALL_OPEN) * 0.5).clamp(-0.1, 0.2)
        } else {
            let st = sit.street.index() - 1;
            ((defaults::FOLD_VS_BET[st] - p.fold_vs_bet[st]) * 0.6).clamp(-0.12, 0.25)
        };
        let street_price = [0.8f32, 0.85, 0.92, 1.0][sit.street.index()];
        let need = (cost / final_pot) as f32 * street_price - lightness;
        let profile_fold = fold_prob(sit, responder, to, self.params);
        let (model_continue, mut continue_range) = if sit.street == Street::Preflop {
            villain_continue_preflop(
                responder.range,
                preflop_raise_fraction(sit, &self.hero_view, to),
                need,
                (1.0 - profile_fold) as f32,
            )
        } else {
            villain_continue(
                responder.range,
                &self.strength,
                eq_table,
                need,
                (1.0 - profile_fold) as f32,
            )
        };
        let street_scale = if sit.street == Street::Preflop {
            self.params.preflop_fold_scale
        } else {
            1.0
        };
        let mut fold = ((1.0 - f64::from(model_continue)) * self.params.fold_scale * street_scale)
            .clamp(0.0, 0.99);
        if self.passive_line && self.params.passive_fold_bonus != 0.0 {
            fold = (fold + self.params.passive_fold_bonus).clamp(0.0, 0.99);
        }
        let aggression = sit
            .history
            .iter()
            .filter(|r| r.street == sit.street && r.seat == responder.seat && r.aggressive())
            .count();
        if aggression > 0 {
            fold = fold.min(0.45 / (1.0 + aggression as f64).powf(1.4));
        }
        if sit.street != Street::Preflop {
            fold = shift_fold_logit(fold, self.params.fold_logit_shift[sit.street.index() - 1]);
            if responder_count == 1 && p.fold_logit_offset != 0.0 {
                fold = shift_fold_logit(fold, f64::from(p.fold_logit_offset));
            }
        }
        if responder.called_all_in {
            fold = fold.min(CALLED_ALL_IN_FOLD_CAP);
        }
        let continue_prob = 1.0 - fold;
        rescale_range_mass(&mut continue_range, responder.range, continue_prob);
        let max_to = sit.max_raise_to.unwrap_or(0);
        let raise_given_continue = if to >= max_to || responder.stack_total <= to {
            0.0
        } else if sit.street == Street::Preflop {
            preflop_reraise_share(sit, responder)
        } else {
            let st = sit.street.index() - 1;
            (f64::from(p.raise_vs_bet[st]) / (1.0 - f64::from(p.fold_vs_bet[st])).max(0.05))
                .clamp(0.0, 0.8)
        };
        ResponderPrice {
            fold_prob: fold,
            continue_prob,
            raise_given_continue,
            continue_range,
        }
    }
}

/// `fold` moved by `shift` on the logit scale.
pub fn shift_fold_logit(fold: f64, shift: f64) -> f64 {
    if shift == 0.0 || fold <= 0.0 {
        return fold;
    }
    let p = fold.clamp(1e-4, 1.0 - 1e-4);
    let logit = (p / (1.0 - p)).ln() + shift;
    (1.0 / (1.0 + (-logit).exp())).clamp(0.0, 0.99)
}

fn rescale_range_mass(range: &mut Range, source: &Range, continue_prob: f64) {
    let target = source.total() * continue_prob;
    if range.total() <= 0.0 && target > 0.0 {
        *range = source.clone();
    }
    let have = range.total();
    if have > 0.0 {
        let scale = (target / have) as f32;
        range.w.iter_mut().for_each(|weight| *weight *= scale);
    }
}

pub fn preflop_raises(sit: &Situation) -> usize {
    sit.history
        .iter()
        .filter(|r| r.street == Street::Preflop && r.aggressive())
        .count()
}

/// The calls before any raise preflop.
pub fn limpers(sit: &Situation) -> usize {
    let mut n = 0;
    for r in sit.history.iter().filter(|r| r.street == Street::Preflop) {
        if r.aggressive() {
            return 0;
        }
        if r.kind == ActionKind::Call {
            n += 1;
        }
    }
    n
}

/// The responder's fold chance by its profile alone.
fn fold_prob(sit: &Situation, r: &Responder, to: i64, params: &Params) -> f64 {
    if r.all_in() {
        return 0.0;
    }
    let p = &r.profile;
    let cur = sit.current_bet();
    if sit.street == Street::Preflop {
        let to_bb = to as f64 / sit.bb as f64;
        let f = match preflop_raises(sit) {
            0 if r.bet >= sit.bb && r.acted_this_street => {
                let looseness = (f64::from(p.vpip) / f64::from(defaults::VPIP))
                    .clamp(0.5, 2.0)
                    .powf(params.profile_response_weight);
                1.0 - (0.55 * (4.0 / to_bb).sqrt() * looseness).clamp(0.15, 0.9)
            }
            0 => {
                1.0 - (f64::from(p.call_open + p.three_bet) * (2.5 / to_bb).powf(0.35))
                    .clamp(0.02, 0.9)
            }
            1 if r.acted_this_street && r.bet == cur => {
                let standard = cur as f64 * 3.3;
                1.0 - (1.0 - f64::from(p.fold_to_3bet)) * (standard / to as f64).powf(0.35).min(1.3)
            }
            1 => 1.0 - (f64::from(p.three_bet) * 0.8).clamp(0.01, 0.3),
            2 if r.acted_this_street && r.bet == cur => f64::from(p.fold_to_4bet),
            2 => 0.97,
            _ => 0.2,
        };
        return f.clamp(0.0, 0.99);
    }
    let hero_bet = sit.hero().bet;
    let pot_after_call = (sit.pot + (cur - hero_bet)).max(1) as f64;
    let frac = (to - cur) as f64 / pot_after_call;
    let mut f = p.fold_to_bet(sit.street, frac);
    let pf_aggressor = sit
        .history
        .iter()
        .rfind(|h| h.street == Street::Preflop && h.aggressive())
        .map(|h| h.seat);
    if sit.street == Street::Flop
        && cur == 0
        && pf_aggressor == Some(sit.hero_seat)
        && params.profile_response_weight > 0.0
    {
        let generic = p.fold_to_bet(sit.street, 0.66);
        let shift = (f64::from(p.fold_to_cbet) - generic) * params.profile_response_weight;
        f = (f + shift).clamp(0.01, 0.98);
    }
    if cur > 0 {
        f += params.raise_fold_bonus;
    }
    f.clamp(0.01, 0.98)
}

fn preflop_reraise_share(sit: &Situation, r: &Responder) -> f64 {
    let p = &r.profile;
    let cur = sit.current_bet();
    let share = match preflop_raises(sit) {
        0 if r.acted_this_street => 0.1,
        0 => f64::from(p.three_bet) / f64::from((p.three_bet + p.call_open).max(0.02)),
        1 if r.acted_this_street && r.bet == cur => {
            f64::from(p.four_bet) / f64::from((1.0 - p.fold_to_3bet).max(0.05))
        }
        1 => 0.3,
        _ => 0.4,
    };
    share.clamp(0.0, 0.8)
}

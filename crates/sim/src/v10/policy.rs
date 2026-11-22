//! v10.0.2's decision: measure hero's equity against the ranges, price a check or a call, price
//! each raise size against the table's answers, and pick by a softmax over the values.
//!
//! The draws come in v10's order: the shared deals, then any equity the deals cannot answer,
//! then one float for the pick. Parity with the recorded actions rests on that order.

use super::cards::Range;
use super::deals::SharedDeals;
use super::draw;
use super::equity::equity_vs_ranges_chunked;
use super::params::Params;
use super::profile::Profile;
use super::ranges::{Boards, estimate_ranges};
use super::responses::{Responder, ResponsePricing, limpers, preflop_raises};
use super::situation::{Situation, Street};
use svanbot_core::rng::Rng;

/// What v10.0.2 does at its turn. A raise is to a street total.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Fold,
    Check,
    Call,
    RaiseTo(i64),
    AllIn,
}

/// v10.0.2's action at `sit`, every seat read with the population default, as an empty model
/// store gave it.
pub fn decide(sit: &Situation, params: &Params, rng: &mut Rng) -> Action {
    let profile = Profile::population_default();
    let mut boards = Boards::default();
    decide_with(sit, &profile, params, &mut boards, rng)
}

fn unmeasured(sit: &Situation) -> Action {
    if sit.can_check {
        Action::Check
    } else {
        Action::Fold
    }
}

struct Measure<'a> {
    deals: Option<SharedDeals>,
    sit: &'a Situation,
    params: &'a Params,
}

impl Measure<'_> {
    fn equity(
        &self,
        subset: &[(usize, Option<&Range>)],
        refs: &[&Range],
        samples: usize,
        rng: &mut Rng,
    ) -> Option<f64> {
        match self.deals.as_ref().and_then(|d| d.equity(subset)) {
            Some(e) => Some(e),
            None => equity_vs_ranges_chunked(
                self.sit.hole,
                &self.sit.board,
                refs,
                samples,
                self.params.deal_chunks,
                rng,
            ),
        }
    }
}

/// [`decide`] with the profile every seat is read with, and the caller's board cache.
pub fn decide_with(
    sit: &Situation,
    profile: &Profile,
    params: &Params,
    boards: &mut Boards,
    rng: &mut Rng,
) -> Action {
    let sit = &sit.without_uncallable();
    let ranges = estimate_ranges(sit, profile, &params.range, boards);
    let hero = sit.hero().clone();
    let ip = sit.in_position(sit.hero_seat);
    let (responders, _) = Responder::for_table(sit, profile, &ranges);
    let mut discount = 1.0;
    let equity_seats: Vec<usize> = if sit.street == Street::Preflop && preflop_raises(sit) > 0 {
        let mut v = Vec::new();
        for (i, r) in responders.iter().enumerate() {
            if r.acted_this_street {
                v.push(i);
            } else {
                discount *=
                    1.0 - 0.5 * f64::from((r.profile.call_open + r.profile.three_bet).min(0.8));
            }
        }
        if v.is_empty() {
            (0..responders.len()).collect()
        } else {
            v
        }
    } else {
        (0..responders.len()).collect()
    };
    let all_ranges: Vec<&Range> = responders.iter().map(|r| r.range).collect();
    let deals = (params.reuse_deals && !all_ranges.is_empty())
        .then(|| {
            SharedDeals::new_chunked(
                sit.hole,
                &sit.board,
                &all_ranges,
                params.samples,
                params.deal_chunks,
                rng,
            )
        })
        .filter(|d| d.is_exact() || d.len() >= params.samples);
    let measure = Measure { deals, sit, params };
    let base_subset: Vec<(usize, Option<&Range>)> =
        equity_seats.iter().map(|&i| (i, None)).collect();
    let base_refs: Vec<&Range> = equity_seats.iter().map(|&i| responders[i].range).collect();
    let Some(raw_eq) = measure.equity(&base_subset, &base_refs, params.samples, rng) else {
        return unmeasured(sit);
    };
    let eq = raw_eq * discount;
    let pot = sit.pot as f64;
    let call = sit.call_amount as f64;
    let mut cands: Vec<(Action, f64)> = Vec::new();
    if sit.can_check {
        let ev = eq * pot * realize(sit.street, ip, eq, params.realize_weight);
        cands.push((Action::Check, ev));
    } else {
        cands.push((Action::Fold, 0.0));
        let all_in_call = sit.call_amount >= hero.stack;
        let r = if all_in_call {
            1.0
        } else {
            realize(sit.street, ip, eq, params.realize_weight)
        };
        let facing_3bet = sit.street == Street::Preflop && preflop_raises(sit) >= 2;
        let margin = params.call_margin
            + if facing_3bet {
                params.three_bet_call_margin
            } else {
                0.0
            };
        let facing_all_in = all_in_call || sit.live_opponents().any(|p| p.stack == 0);
        let eq_call = call_equity(
            eq,
            sit.street,
            facing_all_in,
            pot / sit.bb.max(1) as f64,
            villain_bet_to_pot(sit),
            params,
        );
        cands.push((
            Action::Call,
            eq_call * (pot + call) * r - call - margin * pot,
        ));
    }
    let passive_line = sit.street != Street::Preflop
        && sit.live_opponents().all(|p| {
            !sit.history
                .iter()
                .any(|h| h.seat == p.seat && h.street != Street::Preflop && h.aggressive())
        })
        && sit.history.iter().any(|h| h.street != Street::Preflop);
    let max_to = sit.max_raise_to.unwrap_or(0);
    let pricing = ResponsePricing::new(sit, profile, params, passive_line, boards);
    let all_in_idx: Vec<usize> = responders
        .iter()
        .enumerate()
        .filter(|(_, r)| r.stack_total <= r.bet)
        .map(|(i, _)| i)
        .collect();
    let fold_branch = if all_in_idx.is_empty() {
        pot
    } else {
        let seats: Vec<usize> = all_in_idx.iter().map(|&i| responders[i].seat).collect();
        let (side, main) = sit.split_at_all_ins(&seats);
        let subset: Vec<(usize, Option<&Range>)> = all_in_idx.iter().map(|&i| (i, None)).collect();
        let refs: Vec<&Range> = all_in_idx.iter().map(|&i| responders[i].range).collect();
        let Some(eq_all_in) = measure.equity(&subset, &refs, params.samples / 2, rng) else {
            return unmeasured(sit);
        };
        side + eq_all_in * main
    };
    let targets = if params.raise_allowed(sit, eq) {
        raise_targets(sit, params, ip, eq)
    } else {
        Vec::new()
    };
    for to in targets {
        let add = (to - hero.bet) as f64;
        let prices = pricing.for_raise_to(&responders, to, boards);
        let mut all_fold = 1.0;
        let mut no_raise = 1.0;
        let mut preflop_no_raise = 1.0;
        let mut cont_ranges: Vec<(f64, Range)> = Vec::new();
        for (responder, price) in responders.iter().zip(prices) {
            if responder.stack_total > responder.bet {
                all_fold *= price.fold_prob;
            }
            let no = 1.0 - price.continue_prob * price.raise_given_continue;
            if sit.street == Street::Preflop {
                preflop_no_raise *= no;
            } else {
                no_raise *= no;
            }
            cont_ranges.push((price.continue_prob, price.continue_range));
        }
        if sit.street == Street::Preflop && params.preflop_fold_logit_shift != 0.0 {
            all_fold =
                super::responses::shift_fold_logit(all_fold, params.preflop_fold_logit_shift);
        }
        let called = (1.0 - all_fold).max(1e-6);
        let conts: Vec<f64> = cont_ranges.iter().map(|(c, _)| *c).collect();
        let costs: Vec<f64> = responders
            .iter()
            .map(|r| (to.min(r.stack_total) - r.bet).max(0) as f64)
            .collect();
        let active: Vec<usize> = (0..conts.len())
            .filter(|i| !all_in_idx.contains(i))
            .collect();
        let mut p_exactly_one = 0.0;
        for &i in &active {
            let mut p = conts[i];
            for &j in &active {
                if j != i {
                    p *= 1.0 - conts[j];
                }
            }
            p_exactly_one += p;
        }
        let p_one = (p_exactly_one / called).clamp(0.0, 1.0);
        let mut order = active.clone();
        order.sort_by(|&a, &b| conts[b].total_cmp(&conts[a]));
        let r = if to == max_to {
            1.0
        } else {
            (realize(sit.street, ip, eq, params.realize_weight) + params.initiative).min(1.1)
        };
        let vs_continuing = |callers: &[usize], rng: &mut Rng| -> Option<f64> {
            let idx: Vec<usize> = all_in_idx.iter().chain(callers).copied().collect();
            let subset: Vec<(usize, Option<&Range>)> =
                idx.iter().map(|&i| (i, Some(&cont_ranges[i].1))).collect();
            let refs: Vec<&Range> = idx.iter().map(|&i| &cont_ranges[i].1).collect();
            measure.equity(&subset, &refs, params.samples / 2, rng)
        };
        let mut called_value = 0.0;
        let mut eq_c = eq;
        if let Some(&first) = order.first() {
            let Some(eq1) = vs_continuing(&[first], rng) else {
                return unmeasured(sit);
            };
            let v1 = eq1 * (pot + add + costs[first]) * r - add;
            eq_c = eq1;
            called_value = v1;
            if order.len() >= 2 && p_one < 0.999 {
                let second = order[1];
                let Some(eq2) = vs_continuing(&[first, second], rng) else {
                    return unmeasured(sit);
                };
                let v2 = eq2 * (pot + add + costs[first] + costs[second]) * r - add;
                called_value = p_one * v1 + (1.0 - p_one) * v2;
                eq_c = p_one * eq1 + (1.0 - p_one) * eq2;
            }
        }
        let raised = ((1.0 - no_raise) / called).clamp(0.0, 1.0)
            * params.raise_risk.clamp(0.0, 2.0)
            + ((1.0 - preflop_no_raise) / called).clamp(0.0, 1.0)
                * params.preflop_raise_risk.clamp(0.0, 2.0);
        if raised > 0.0 {
            let raise_value = (eq_c * eq_c * (pot + 4.0 * add) - 3.0 * add).max(-add);
            let q = raised.min(1.0);
            called_value = (1.0 - q) * called_value + q * raise_value;
        }
        let ev = all_fold * fold_branch + (1.0 - all_fold) * called_value;
        let action = if to == max_to {
            Action::AllIn
        } else {
            Action::RaiseTo(to)
        };
        cands.push((action, ev));
    }
    pick(&cands, (params.temperature * pot).max(1.0), rng)
}

/// A softmax draw over the values at `temp`, with anything more than four temperatures short
/// of the best left out.
fn pick(cands: &[(Action, f64)], temp: f64, rng: &mut Rng) -> Action {
    let best = cands.iter().map(|(_, ev)| *ev).fold(f64::MIN, f64::max);
    let weights: Vec<f64> = cands
        .iter()
        .map(|(_, ev)| {
            if best - ev > temp * 4.0 {
                0.0
            } else {
                ((ev - best) / temp).exp()
            }
        })
        .collect();
    let total: f64 = weights.iter().sum();
    let mut x = draw::f64(rng) * total;
    let mut chosen = 0;
    for (i, w) in weights.iter().enumerate() {
        if x < *w {
            chosen = i;
            break;
        }
        x -= w;
    }
    cands[chosen].0
}

/// The share of the pot hero's equity is worth before the river, by position.
pub fn realize(street: Street, ip: bool, eq: f64, weight: f64) -> f64 {
    let street_weight = [1.0, 0.75, 0.4, 0.0][street.index()];
    let base = 0.4 + 1.2 * eq - 0.55 * eq * eq;
    let pos = if ip { 1.0 } else { 0.9 };
    1.0 + (base * pos - 1.0) * weight * street_weight
}

const DEEP_CALL_MIN_POT_BB: f64 = 500.0;
const OVERBET_CALL_MIN_RATIO: f64 = 1.5;
const OVERBET_SLOPE_CAP: f64 = 0.40;

fn overbet_size_x(bet_to_pot: f64) -> f64 {
    if bet_to_pot >= OVERBET_CALL_MIN_RATIO {
        1.0 + (bet_to_pot / OVERBET_CALL_MIN_RATIO).ln()
    } else {
        0.0
    }
}

/// The size of the last bet this street, as a share of the pot it went into.
fn villain_bet_to_pot(sit: &Situation) -> f64 {
    sit.history
        .iter()
        .rfind(|r| r.street == sit.street && r.seat != sit.hero_seat && r.aggressive())
        .filter(|r| r.pot_before > 0)
        .map(|r| (r.to - r.bet_before) as f64 / r.pot_before as f64)
        .unwrap_or(0.0)
}

/// Hero's equity for a call, shaded against an all-in by the call shifts.
fn call_equity(
    eq: f64,
    street: Street,
    facing_all_in: bool,
    pot_bb: f64,
    bet_to_pot: f64,
    params: &Params,
) -> f64 {
    if !facing_all_in {
        return eq;
    }
    let river = if street == Street::River {
        params.river_jam_call_shift
    } else {
        0.0
    };
    let deep = if pot_bb >= DEEP_CALL_MIN_POT_BB {
        params.deep_call_shift
    } else {
        0.0
    };
    let flat = if bet_to_pot >= OVERBET_CALL_MIN_RATIO {
        params.overbet_call_shift
    } else {
        0.0
    };
    let sized = (params.overbet_call_slope * overbet_size_x(bet_to_pot)).min(OVERBET_SLOPE_CAP);
    (eq - river.max(deep).max(flat.max(sized))).max(0.0)
}

/// The raise sizes v10.0.2 priced at `sit`, ascending.
pub fn raise_targets(sit: &Situation, params: &Params, ip: bool, eq: f64) -> Vec<i64> {
    let (Some(min_to), Some(max_to)) = (sit.min_raise_to, sit.max_raise_to) else {
        return vec![];
    };
    let min_to = min_to.min(max_to);
    let cur = sit.current_bet();
    let bb = sit.bb as f64;
    let hero_bet = sit.hero().bet;
    let mut t: Vec<f64> = Vec::new();
    if sit.street == Street::Preflop {
        match preflop_raises(sit) {
            0 => {
                let short = sit.effective_stack() as f64 / bb <= params.preflop_jam_bb;
                let open = if short {
                    params.short_open_bb
                } else {
                    params.open_bb
                };
                let base = open + params.limper_bb * limpers(sit) as f64;
                t.push(base * bb);
                t.push((base + 1.5) * bb);
            }
            1 => {
                let callers = sit
                    .players
                    .iter()
                    .filter(|p| p.bet == cur && p.seat != sit.hero_seat)
                    .count()
                    .saturating_sub(1);
                let m = if ip {
                    params.three_bet_ip
                } else {
                    params.three_bet_oop
                };
                t.push(cur as f64 * m + callers as f64 * cur as f64);
            }
            _ => t.push(cur as f64 * params.four_bet),
        }
    } else {
        let pot_after_call = (sit.pot + (cur - hero_bet)) as f64;
        for &s in &params.bet_sizes {
            t.push(cur as f64 + s * pot_after_call);
        }
    }
    let mut out: Vec<i64> = t
        .into_iter()
        .map(|x| (x.round() as i64).clamp(min_to, max_to))
        .map(|x| {
            if x as f64 >= max_to as f64 * 0.7 {
                max_to
            } else {
                x
            }
        })
        .collect();
    let jam_size = (max_to - cur) as f64;
    let pot_after_call = (sit.pot + (cur - hero_bet)).max(1) as f64;
    let jam_ok = if sit.street == Street::Preflop {
        let eff_bb = sit.effective_stack() as f64 / bb;
        eff_bb <= params.preflop_jam_bb
            || (preflop_raises(sit) >= 2 && eff_bb <= 2.0 * params.preflop_jam_bb)
            || jam_size <= pot_after_call * 3.0
    } else {
        jam_size <= pot_after_call * params.jam_pot_ratio
            || (eq >= 0.93 && jam_size <= pot_after_call * 5.0)
    };
    if jam_ok {
        out.push(max_to);
    } else {
        out.retain(|&x| x != max_to || jam_size <= pot_after_call * params.jam_pot_ratio);
    }
    if out.is_empty() {
        out.push(min_to);
    }
    out.sort_unstable();
    out.dedup();
    out
}

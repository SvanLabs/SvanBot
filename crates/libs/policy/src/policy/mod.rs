//! Exploitative EV policy: estimate every live opponent's range and response
//! tendencies from their learned profile, score each legal action by expected
//! chips, and pick the best (mixing only between near-equal options).

use serde::{Deserialize, Serialize};
use sv10_cards::range::Range;
use sv10_engine::engine::{Action, Street};
use sv10_engine::situation::Situation;
use sv10_equity::equity::SharedDeals;
use sv10_model::model::defaults;
use sv10_model::model::{ModelStore, Profile, aggressive};
use sv10_model::oprange::{
    aggressive_likelihood, board_strengths, equity_vs_histogram, estimate_ranges, perceived_range, preflop_raise_fraction,
    strength_histogram, villain_continue, villain_continue_preflop,
};
use sv10_rng::{Rng, RngExt};

mod measure;
mod params;
mod responses;
pub use params::Params;
use responses::*;

/// One priced option of a decision (shown on the dashboard's "why this move" bars).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    /// `fold`, `check`, `call`, `raise` or `all_in`.
    pub action: String,
    /// Raise-to total for raises.
    pub amount: Option<i64>,
    /// Expected chips relative to folding, calibration term included.
    pub ev: f64,
    /// Modeled chance that every opponent folds to it (bets and raises).
    pub fold_prob: f64,
    /// Hero's equity when the action is called (or against the current ranges for check/call).
    pub equity_called: f64,
    /// Spot category used for self-calibration (None for folds).
    pub category: Option<String>,
    /// Calibration correction already included in `ev`, in chips.
    pub bias: f64,
}

/// Spot category for self-calibration: street, action family and bet-size class.
pub fn spot_category(street: Street, action: &str, amount: Option<i64>, pot: i64, current_bet: i64, hero_bet: i64) -> Option<String> {
    let s = street.name();
    match action {
        "fold" => None,
        "check" => Some(format!("{s}:check")),
        "call" => Some(format!("{s}:call")),
        "all_in" => Some(format!("{s}:allin")),
        _ => {
            let to = amount.unwrap_or(0);
            let base = (pot + (current_bet - hero_bet).max(0)).max(1) as f64;
            let frac = (to - current_bet).max(0) as f64 / base;
            let class = if frac < 0.6 {
                "small"
            } else if frac < 1.3 {
                "big"
            } else {
                "over"
            };
            Some(format!("{s}:{}:{class}", if current_bet > 0 { "raise" } else { "bet" }))
        }
    }
}

/// A decision with everything that explains it.
#[derive(Clone, Debug)]
pub struct Decision {
    /// The action to send.
    pub action: Action,
    /// Protocol name of the action.
    pub action_name: String,
    /// Raise-to total for raises.
    pub amount: Option<i64>,
    /// Hero's equity against the estimated ranges; `None` when the deal draw could not measure it
    /// and no option could be priced (#424).
    pub equity: Option<f64>,
    /// Share of the final pot hero must put in to call.
    pub pot_odds: f64,
    /// Every option considered, with its EV.
    pub candidates: Vec<Candidate>,
    /// One-line human explanation.
    pub reason: String,
    /// The candidate that was picked (with its category and calibration term).
    pub chosen: Candidate,
}

fn action_label(a: Action) -> (String, Option<i64>) {
    match a {
        Action::Fold => ("fold".into(), None),
        Action::Check => ("check".into(), None),
        Action::Call => ("call".into(), None),
        Action::RaiseTo(t) => ("raise".into(), Some(t)),
        Action::AllIn => ("all_in".into(), None),
    }
}

/// Decide for `sit` with the stat response model only.
pub fn decide<R: Rng>(sit: &Situation, models: &ModelStore, params: &Params, rng: &mut R) -> Decision {
    decide_with(sit, models, params, None, rng)
}

/// Decide for `sit`, using `nn` for opponent responses when given, in the strength mode set by `params`.
pub fn decide_with<R: Rng>(sit: &Situation, models: &ModelStore, params: &Params, nn: Option<&sv10_nn::nn::Mlp>, rng: &mut R) -> Decision {
    sv10_model::oprange::with_strength_mode(params.exact_strengths, || decide_inner(sit, models, params, nn, rng))
}

fn decide_inner<R: Rng>(sit: &Situation, models: &ModelStore, params: &Params, nn: Option<&sv10_nn::nn::Mlp>, rng: &mut R) -> Decision {
    // Price every option against the pot hero can actually win (live 2026-09-14: calls with 6%
    // equity against a 900k-chip shove were scored at +53k).
    let sit = &sit.without_uncallable();
    let ranges = estimate_ranges(sit, models, &params.range);
    let hero = sit.hero().clone();
    let ip = sit.in_position(sit.hero_seat);
    let (responders, acted) = Responder::for_table(sit, models, &ranges);
    // Preflop, players still to act behind a raise mostly fold: take equity
    // against the players already in and discount for the rest.
    let mut discount = 1.0;
    let equity_seats: Vec<usize> = if sit.street == Street::Preflop && preflop_raises(sit) > 0 {
        let mut v = Vec::new();
        for (i, r) in responders.iter().enumerate() {
            if r.acted_this_street {
                v.push(i);
            } else {
                discount *= 1.0 - 0.5 * (r.profile.call_open + r.profile.three_bet).min(0.8) as f64;
            }
        }
        if v.is_empty() { (0..responders.len()).collect() } else { v }
    } else {
        (0..responders.len()).collect()
    };
    let all_ranges: Vec<&Range> = responders.iter().map(|r| r.range).collect();
    let deals = measure::shared_deals(sit, &all_ranges, params, rng);
    let base_subset: Vec<(usize, Option<&Range>)> = equity_seats.iter().map(|&i| (i, None)).collect();
    let base_refs: Vec<&Range> = equity_seats.iter().map(|&i| responders[i].range).collect();
    let Some(raw_eq) = measure::measured_equity(deals.as_ref(), sit, params, &base_subset, &base_refs, params.samples, rng) else {
        return measure::unmeasured(sit);
    };
    let eq = raw_eq * discount;
    let pot = sit.pot as f64;
    let call = sit.call_amount as f64;
    let pot_odds = if call > 0.0 { call / (pot + call) } else { 0.0 };
    let mut cands: Vec<(Action, Candidate)> = Vec::new();
    if sit.can_check {
        let mut ev = eq * pot * realize(sit.street, ip, eq, params.realize_weight);
        if params.check_lookahead > 0.0
            && sit.street != Street::Preflop
            && let Some(look) = check_lookahead(sit, &responders, &acted, deals.as_ref(), eq, ip, params)
        {
            ev = (1.0 - params.check_lookahead.min(1.0)) * ev + params.check_lookahead.min(1.0) * look;
        }
        cands.push((
            Action::Check,
            Candidate { action: "check".into(), amount: None, ev, fold_prob: 0.0, equity_called: eq, category: None, bias: 0.0 },
        ));
    } else {
        cands.push((
            Action::Fold,
            Candidate { action: "fold".into(), amount: None, ev: 0.0, fold_prob: 0.0, equity_called: 0.0, category: None, bias: 0.0 },
        ));
        let all_in_call = sit.call_amount >= hero.stack;
        let r = if all_in_call { 1.0 } else { realize(sit.street, ip, eq, params.realize_weight) };
        let facing_3bet = sit.street == Street::Preflop && preflop_raises(sit) >= 2;
        let margin = params.call_margin + if facing_3bet { params.three_bet_call_margin } else { 0.0 };
        // River calls against an all-in over-estimated equity by 0.068 (0159): the live fit
        // lowers only those calls, so the recorded estimate stays raw and refits never compound.
        let facing_all_in = all_in_call || sit.live_opponents().any(|p| p.stack == 0);
        let eq_call = call_equity(eq, sit.street, facing_all_in, pot / sit.bb.max(1) as f64, villain_bet_to_pot(sit), params);
        let ev = eq_call * (pot + call) * r - call - margin * pot;
        cands.push((
            Action::Call,
            Candidate { action: "call".into(), amount: None, ev, fold_prob: 0.0, equity_called: eq, category: None, bias: 0.0 },
        ));
    }
    // Every live opponent has only checked on every postflop street so far.
    let passive_line = sit.street != Street::Preflop
        && sit.live_opponents().all(|p| !sit.history.iter().any(|h| h.seat == p.seat && h.street != Street::Preflop && aggressive(h)))
        && sit.history.iter().any(|h| h.street != Street::Preflop);
    let max_to = sit.max_raise_to.unwrap_or(0);
    let response_pricing = ResponsePricing::new(sit, models, params, nn, passive_line);
    // Opponents already all-in stay in the pot when everyone else folds (live 2026-09-15: a 10%
    // equity river jam was scored as winning 12.5k when the only player who could fold did).
    let all_in_idx: Vec<usize> = responders.iter().enumerate().filter(|(_, r)| r.stack_total <= r.bet).map(|(i, _)| i).collect();
    let fold_branch = if all_in_idx.is_empty() {
        pot
    } else {
        let seats: Vec<usize> = all_in_idx.iter().map(|&i| responders[i].seat).collect();
        let (side, main) = sit.split_at_all_ins(&seats);
        let subset: Vec<(usize, Option<&Range>)> = all_in_idx.iter().map(|&i| (i, None)).collect();
        let refs: Vec<&Range> = all_in_idx.iter().map(|&i| responders[i].range).collect();
        let Some(eq_all_in) = measure::measured_equity(deals.as_ref(), sit, params, &subset, &refs, params.samples / 2, rng) else {
            return measure::unmeasured(sit);
        };
        side + eq_all_in * main
    };
    // No bluff raise wars: once a street has two raises, only raise with real equity.
    let street_raises = sit.history.iter().filter(|h| h.street == sit.street && aggressive(h)).count();
    let raise_allowed = !(sit.street != Street::Preflop && street_raises >= 2 && eq < 0.55)
        && !(sit.street == Street::River && street_raises >= 1 && eq < 0.5);
    let targets = if raise_allowed { raise_targets(sit, params, ip, eq) } else { Vec::new() };
    for to in targets {
        let add = (to - hero.bet) as f64;
        let prices = response_pricing.for_raise_to(&responders, to);
        let mut all_fold = 1.0;
        let mut cont_ranges: Vec<(f64, Range)> = Vec::new();
        // Probability that nobody raises us back, over all responders.
        let mut no_raise = 1.0;
        let mut preflop_no_raise = 1.0;
        for (responder, price) in responders.iter().zip(prices) {
            if responder.stack_total > responder.bet {
                all_fold *= price.fold_prob;
            }
            if sit.street == Street::Preflop {
                preflop_no_raise *= 1.0 - price.continue_prob * price.raise_given_continue;
            } else {
                no_raise *= 1.0 - price.continue_prob * price.raise_given_continue;
            }
            debug_assert!(responder.stack_total > responder.bet || price.fold_prob == 0.0);
            cont_ranges.push((price.continue_prob, price.continue_range));
        }
        // Preflop everyone-folds estimates ran about 2x reality on live raises (2026-09-23); the live
        // calibration shift pulls them back. Recorded per decision so refits never compound.
        if sit.street == Street::Preflop && params.preflop_fold_logit_shift != 0.0 {
            all_fold = shift_fold_logit(all_fold, params.preflop_fold_logit_shift);
        }
        let called = (1.0 - all_fold).max(1e-6);
        // Mixture over "exactly one caller" and "two or more callers", among the players who can still
        // act; players already all-in are in every showdown and never count as callers.
        let conts: Vec<f64> = cont_ranges.iter().map(|(c, _)| *c).collect();
        let costs: Vec<f64> = responders.iter().map(|r| ((to.min(r.stack_total)) - r.bet).max(0) as f64).collect();
        let active: Vec<usize> = (0..conts.len()).filter(|i| !all_in_idx.contains(i)).collect();
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
        let r = if to == max_to { 1.0 } else { (realize(sit.street, ip, eq, params.realize_weight) + params.initiative).min(1.1) };
        let mut called_value = 0.0;
        let mut eq_c = eq;
        // Equity against the continuing part of one or two responders' ranges: reweight the shared
        // deals when possible, otherwise sample afresh.
        let vs_continuing = |callers: &[usize], rng: &mut R| -> Option<f64> {
            let idx: Vec<usize> = all_in_idx.iter().chain(callers).copied().collect();
            let subset: Vec<(usize, Option<&Range>)> = idx.iter().map(|&i| (i, Some(&cont_ranges[i].1))).collect();
            let refs: Vec<&Range> = idx.iter().map(|&i| &cont_ranges[i].1).collect();
            measure::measured_equity(deals.as_ref(), sit, params, &subset, &refs, params.samples / 2, rng)
        };
        if let Some(&first) = order.first() {
            let Some(eq1) = vs_continuing(&[first], rng) else {
                return measure::unmeasured(sit);
            };
            let v1 = eq1 * (pot + add + costs[first]) * r - add;
            eq_c = eq1;
            called_value = v1;
            if order.len() >= 2 && p_one < 0.999 {
                let second = order[1];
                let Some(eq2) = vs_continuing(&[first, second], rng) else {
                    return measure::unmeasured(sit);
                };
                let v2 = eq2 * (pot + add + costs[first] + costs[second]) * r - add;
                called_value = p_one * v1 + (1.0 - p_one) * v2;
                eq_c = p_one * eq1 + (1.0 - p_one) * eq2;
            }
        }
        // Raise-back branch: facing a raise (about 3x our bet) we keep playing only with the part
        // of our equity that holds up against a raising range; otherwise we lose the bet.
        let raised = ((1.0 - no_raise) / called).clamp(0.0, 1.0) * params.raise_risk.clamp(0.0, 2.0)
            + ((1.0 - preflop_no_raise) / called).clamp(0.0, 1.0) * params.preflop_raise_risk.clamp(0.0, 2.0);
        if raised > 0.0 {
            let eq_vs_raise = eq_c * eq_c;
            let raise_value = (eq_vs_raise * (pot + 4.0 * add) - 3.0 * add).max(-add);
            let q = raised.min(1.0);
            called_value = (1.0 - q) * called_value + q * raise_value;
        }
        let ev = all_fold * fold_branch + (1.0 - all_fold) * called_value;
        let action = if to == max_to { Action::AllIn } else { Action::RaiseTo(to) };
        let (name, _) = action_label(action);
        cands.push((
            action,
            Candidate { action: name, amount: Some(to), ev, fold_prob: all_fold, equity_called: eq_c, category: None, bias: 0.0 },
        ));
    }
    let cur_bet = sit.current_bet();
    for (_, c) in cands.iter_mut() {
        c.category = spot_category(sit.street, &c.action, c.amount, sit.pot, cur_bet, hero.bet);
        if let Some(b) = c.category.as_ref().and_then(|k| params.ev_bias.get(k)) {
            let cap = c.category.as_ref().and_then(|k| params.ev_bias_pot_cap.get(k)).copied();
            c.bias = applied_bias(*b, sit.bb, sit.pot + sit.call_amount.max(0), cap);
            c.ev += c.bias;
        }
    }
    let best = cands.iter().map(|(_, c)| c.ev).fold(f64::MIN, f64::max);
    let temp = (params.temperature * pot).max(1.0);
    let weights: Vec<f64> = cands.iter().map(|(_, c)| if best - c.ev > temp * 4.0 { 0.0 } else { ((c.ev - best) / temp).exp() }).collect();
    let total: f64 = weights.iter().sum();
    let mut x = rng.random::<f64>() * total;
    let mut pick = 0;
    for (i, w) in weights.iter().enumerate() {
        if x < *w {
            pick = i;
            break;
        }
        x -= w;
    }
    let (action, chosen) = cands[pick].clone();
    let (action_name, amount) = action_label(action);
    // The opponents' fold chance belongs to our bets and raises only: after a fold, check or call
    // it is always 0 and read as "we fold 0%" on the dashboard.
    let fold_chance = if matches!(action, Action::RaiseTo(_) | Action::AllIn) {
        format!(", they fold {:.0}%", chosen.fold_prob * 100.0)
    } else {
        String::new()
    };
    let reason = format!("eq {:.2} vs {} opp, {} ev {:.0} (best {:.0}){fold_chance}", eq, responders.len(), chosen.action, chosen.ev, best);
    Decision {
        action,
        action_name,
        amount: if matches!(action, Action::RaiseTo(_) | Action::AllIn) { chosen.amount } else { amount },
        equity: Some(eq),
        pot_odds,
        candidates: cands.into_iter().map(|(_, c)| c).collect(),
        reason,
        chosen: chosen.clone(),
    }
}

/// Value of checking when opponents still act behind hero this street (0103): with the chance that
/// one of them bets (their own bet-first rates), hero answers the most likely bettor's narrowed
/// betting range by calling or folding; otherwise the street checks through and hero realizes its
/// equity as before. Returns None when nobody acts behind or the shared deals cannot reweight.
fn check_lookahead(
    sit: &Situation,
    responders: &[Responder],
    acted: &std::collections::HashSet<usize>,
    deals: Option<&SharedDeals>,
    eq: f64,
    ip: bool,
    params: &Params,
) -> Option<f64> {
    let deals = deals?;
    let st = sit.street.index() - 1;
    let behind: Vec<usize> =
        (0..responders.len()).filter(|&i| !acted.contains(&responders[i].seat) && responders[i].stack_total > responders[i].bet).collect();
    if behind.is_empty() {
        return None;
    }
    let p_check: f64 = behind.iter().map(|&i| 1.0 - responders[i].profile.bet_first[st] as f64).product();
    let p_bet = 1.0 - p_check;
    let bettor = *behind.iter().max_by(|&&a, &&b| responders[a].profile.bet_first[st].total_cmp(&responders[b].profile.bet_first[st]))?;
    let pot = sit.pot as f64;
    let size = 0.66;
    let bet = (pot * size).min((responders[bettor].stack_total - responders[bettor].bet) as f64).min(sit.hero().stack as f64);
    if bet <= 0.0 {
        return None;
    }
    let pct = &sv10_model::oprange::board_info(&sit.board).pct;
    let lik = sv10_model::oprange::postflop_likelihood(
        sv10_engine::engine::ActionKind::Raise,
        true,
        0,
        size,
        sit.street,
        &responders[bettor].profile,
        pct,
        &params.range,
    );
    let mut bet_range = responders[bettor].range.clone();
    for (w, l) in bet_range.w.iter_mut().zip(lik.iter()) {
        *w *= *l;
    }
    let subset: Vec<(usize, Option<&Range>)> =
        (0..responders.len()).map(|i| (i, if i == bettor { Some(&bet_range) } else { None })).collect();
    let eq_bet = deals.equity(&subset)?;
    let r = realize(sit.street, ip, eq_bet, params.realize_weight);
    let call = eq_bet * (pot + 2.0 * bet) * r - bet - params.call_margin * (pot + bet);
    let through = eq * pot * realize(sit.street, ip, eq, params.realize_weight);
    Some(p_check * through + p_bet * call.max(0.0))
}

/// Pot (big blinds, before our call) from which [`Params::deep_call_shift`] applies to a call of an all-in.
pub const DEEP_CALL_MIN_POT_BB: f64 = 500.0;

/// Villain bet size (as a multiple of the pot before it) from which [`Params::overbet_call_shift`] applies.
pub const OVERBET_CALL_MIN_RATIO: f64 = 1.5;

/// Largest equity shift the size-scaled overbet fit may apply (0233): an extrapolation guard for
/// shoves far beyond the sizes it was fitted on (LESSONS 29).
pub const OVERBET_SLOPE_CAP: f64 = 0.40;

/// Size feature of the size-scaled overbet shift: `1 + ln(r / OVERBET_CALL_MIN_RATIO)` for a bet of
/// `r` times the pot at or above the overbet threshold (1 at 1.5x, ~2 at 4x, ~5.8 at 180x), else 0.
pub fn overbet_size_x(bet_to_pot: f64) -> f64 {
    if bet_to_pot >= OVERBET_CALL_MIN_RATIO { 1.0 + (bet_to_pot / OVERBET_CALL_MIN_RATIO).ln() } else { 0.0 }
}

/// The last bet or raise an opponent made on the current street, as a multiple of the pot before
/// it (0 when there is none).
pub fn villain_bet_to_pot(sit: &Situation) -> f64 {
    sit.history
        .iter()
        .rfind(|r| r.street == sit.street && r.seat != sit.hero_seat && aggressive(r))
        .filter(|r| r.pot_before > 0)
        .map(|r| (r.to - r.bet_before) as f64 / r.pot_before as f64)
        .unwrap_or(0.0)
}

/// Equity a call is priced with: the raw estimate, lowered by the live-fitted shifts when calling an
/// all-in on the river (0159), in a pot of at least [`DEEP_CALL_MIN_POT_BB`] or against an overbet of
/// at least [`OVERBET_CALL_MIN_RATIO`] times the pot (2026-09-23), the overbet shift growing with the
/// bet's size when the size-scaled fit is installed (0233). The largest applicable shift is used,
/// never the sum, and the recorded estimate stays raw so refits never compound.
pub fn call_equity(eq: f64, street: Street, facing_all_in: bool, pot_bb: f64, bet_to_pot: f64, params: &Params) -> f64 {
    if !facing_all_in {
        return eq;
    }
    let river = if street == Street::River { params.river_jam_call_shift } else { 0.0 };
    let deep = if pot_bb >= DEEP_CALL_MIN_POT_BB { params.deep_call_shift } else { 0.0 };
    let flat = if bet_to_pot >= OVERBET_CALL_MIN_RATIO { params.overbet_call_shift } else { 0.0 };
    let sized = (params.overbet_call_slope * overbet_size_x(bet_to_pot)).min(OVERBET_SLOPE_CAP);
    let overbet = flat.max(sized);
    (eq - river.max(deep).max(overbet)).max(0.0)
}

/// The live calibration's per-category correction in chips. Residuals scale with the pot (river
/// calls: -0.7 bb in pots up to 50 bb, -326 bb past 1,000 bb, 2026-09-24), so a penalty fitted in
/// big blinds never exceeds `pot_cap` (the category's supported per-pot residual) of `scale`
/// (pot + call, the calibration's unit); upward corrections are capped by the calibration step.
fn applied_bias(bias_bb: f64, bb: i64, scale: i64, pot_cap: Option<f64>) -> f64 {
    let chips = bias_bb * bb as f64;
    match pot_cap {
        Some(cap) if chips < 0.0 => chips.max(-cap * scale as f64),
        _ => chips,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv10_rng::SeedableRng;

    /// Every option a decision considered, as (action, raise-to total) — the shape two decisions
    /// must agree on when a correction moves their pricing but not their protocol legality.
    fn options(d: &Decision) -> Vec<(String, Option<i64>)> {
        d.candidates.iter().map(|c| (c.action.clone(), c.amount)).collect()
    }

    #[test]
    fn a_penalty_is_capped_by_its_own_per_pot_evidence() {
        // Live 2026-09-24: river:call carried -8.8 bb from big-pot residuals while calls in pots up to
        // 100 bb realized their prediction; its per-pot residual bounds the penalty in a small pot.
        assert_eq!(applied_bias(-8.8, 20, 280, Some(0.1)), -0.1 * 280.0);
        assert_eq!(applied_bias(-8.8, 20, 100_000, Some(0.1)), -8.8 * 20.0);
        // preflop:raise:over missed by more than the pot: its own cap leaves the bb penalty whole.
        assert_eq!(applied_bias(-1.91, 20, 60, Some(1.3)), -1.91 * 20.0);
        // No per-pot evidence (records before 0207): the bb correction as fitted.
        assert_eq!(applied_bias(-8.8, 20, 280, None), -8.8 * 20.0);
        // Upward corrections keep their own small cap in the calibration step.
        assert_eq!(applied_bias(3.0, 20, 30, Some(0.1)), 60.0);
    }

    #[test]
    fn prior_street_calls_live_history_matches_feature_contract() {
        use sv10_engine::engine::{ActionKind, ActionRecord};
        let action = |seat, street, kind| ActionRecord {
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
        };
        let history = vec![
            action(1, Street::Flop, ActionKind::Call),
            action(2, Street::Flop, ActionKind::Call),
            action(1, Street::Turn, ActionKind::Call),
            action(1, Street::River, ActionKind::Call),
        ];
        assert_eq!(sv10_model::features::prior_postflop_calls(&history, 1, Street::Turn), 1);
        assert_eq!(sv10_model::features::prior_postflop_calls(&history, 1, Street::River), 2);
        assert_eq!(sv10_model::features::prior_postflop_calls(&history, 2, Street::River), 1);
    }

    #[test]
    fn folds_weak_hand_against_uncallable_overshove() {
        let sit = sv10_engine::situation::fixtures::uncallable_overshove();
        let mut rng = sv10_rng::rngs::SmallRng::seed_from_u64(7);
        let d = decide_with(&sit, &ModelStore::default(), &Params::default(), None, &mut rng);
        let call = d.candidates.iter().find(|c| c.action == "call").unwrap();
        // Winnable: 470 before the river + 3,716 matched by the shover + 1,320 + hero's 3,716.
        assert!(call.ev < 9_222.0 * 0.5, "call EV {} counts chips no one can win", call.ev);
        assert_eq!(d.action_name, "fold", "{:?}", d.candidates);
    }

    #[test]
    fn the_river_jam_call_shift_lowers_only_river_calls_against_an_all_in() {
        // 0159: river all-in calls realized 0.068 less equity than estimated; the live fit
        // subtracts its shift from the call's equity there and nowhere else.
        let sit = sv10_engine::situation::fixtures::uncallable_overshove();
        assert_eq!(sit.street, Street::River);
        let call_ev = |sit: &Situation, shift: f64| {
            let params = Params { river_jam_call_shift: shift, ..Params::default() };
            let mut rng = sv10_rng::rngs::SmallRng::seed_from_u64(7);
            let d = decide_with(sit, &ModelStore::default(), &params, None, &mut rng);
            d.candidates.iter().find(|c| c.action == "call").unwrap().ev
        };
        let (plain, shifted) = (call_ev(&sit, 0.0), call_ev(&sit, 0.1));
        assert!(shifted < plain - 1.0, "shifted {shifted} vs {plain}");
        let mut turn = sit.clone();
        turn.street = Street::Turn;
        assert_eq!(call_ev(&turn, 0.1), call_ev(&turn, 0.0), "only river calls are shifted");
    }

    #[test]
    fn the_preflop_fold_shift_lowers_only_preflop_raise_fold_equity() {
        let mut deal_rng = sv10_rng::rngs::SmallRng::seed_from_u64(23);
        let hand = sv10_engine::engine::Hand::new(&[2_000; 4], 0, 10, 20, &mut deal_rng);
        let names = vec!["hero".into(), "a".into(), "b".into(), "c".into()];
        let sit = Situation::from_hand(&hand, hand.actor().expect("preflop actor"), &names);
        let folds = |shift: f64| {
            let params = Params { preflop_fold_logit_shift: shift, ..Params::default() };
            let d = decide_with(&sit, &ModelStore::default(), &params, None, &mut sv10_rng::rngs::SmallRng::seed_from_u64(3));
            d.candidates.iter().filter(|c| c.action == "raise" || c.action == "all_in").map(|c| c.fold_prob).collect::<Vec<_>>()
        };
        let (plain, shifted) = (folds(0.0), folds(-1.2));
        assert!(!plain.is_empty());
        assert!(plain.iter().zip(&shifted).all(|(p, s)| s < p || *p == 0.0), "{plain:?} vs {shifted:?}");
    }

    #[test]
    fn deep_pot_all_in_calls_take_the_larger_live_shift() {
        let p = Params { river_jam_call_shift: 0.06, deep_call_shift: 0.15, ..Params::default() };
        // Not facing an all-in: untouched on every street and pot.
        assert_eq!(call_equity(0.6, Street::River, false, 900.0, 0.0, &p), 0.6);
        // Small river pot: only the river shift.
        assert!((call_equity(0.6, Street::River, true, 80.0, 0.0, &p) - 0.54).abs() < 1e-12);
        // Deep turn pot: only the deep shift.
        assert!((call_equity(0.6, Street::Turn, true, 600.0, 0.0, &p) - 0.45).abs() < 1e-12);
        // Deep river pot: the larger shift, not both.
        assert!((call_equity(0.6, Street::River, true, 600.0, 0.0, &p) - 0.45).abs() < 1e-12);
        // Just under the threshold, and never below zero.
        assert_eq!(call_equity(0.6, Street::Turn, true, 499.0, 0.0, &p), 0.6);
        assert_eq!(call_equity(0.1, Street::Turn, true, 600.0, 0.0, &p), 0.0);
        // A 180x-pot overbet (2026-09-23, set vs nut straight): the overbet shift wins when larger.
        let o = Params { overbet_call_shift: 0.3, ..p.clone() };
        assert!((call_equity(0.685, Street::Turn, true, 10_000.0, 180.0, &o) - 0.385).abs() < 1e-12);
        assert!((call_equity(0.6, Street::Turn, true, 80.0, 1.2, &o) - 0.6).abs() < 1e-12, "a normal bet is not an overbet");
    }

    #[test]
    fn the_size_scaled_overbet_shift_grows_with_the_bet_and_is_capped() {
        assert_eq!(overbet_size_x(1.2), 0.0);
        assert!((overbet_size_x(1.5) - 1.0).abs() < 1e-12);
        assert!((overbet_size_x(180.0) - (1.0 + 120f64.ln())).abs() < 1e-12);
        let p = Params { overbet_call_slope: 0.06, ..Params::default() };
        let at = |r: f64| 0.7 - call_equity(0.7, Street::Turn, true, 100.0, r, &p);
        // Below the threshold: nothing; then the shift rises with the size, up to the cap.
        assert_eq!(at(1.4), 0.0);
        assert!((at(1.5) - 0.06).abs() < 1e-12);
        assert!(at(4.0) > at(2.0) && at(20.0) > at(4.0) && at(180.0) > at(20.0));
        assert!((at(180.0) - 0.06 * (1.0 + 120f64.ln())).abs() < 1e-12, "180x pot: {}", at(180.0));
        let steep = Params { overbet_call_slope: 0.2, ..Params::default() };
        assert!((0.7 - call_equity(0.7, Street::Turn, true, 100.0, 1e6, &steep) - OVERBET_SLOPE_CAP).abs() < 1e-12, "capped");
        // With a flat shift installed too, the larger one applies, never the sum.
        let both = Params { overbet_call_shift: 0.2, ..p.clone() };
        assert!((0.7 - call_equity(0.7, Street::Turn, true, 100.0, 2.0, &both) - 0.2).abs() < 1e-12);
        assert!((0.7 - call_equity(0.7, Street::Turn, true, 100.0, 180.0, &both) - 0.06 * (1.0 + 120f64.ln())).abs() < 1e-12);
        // Not facing an all-in: untouched.
        assert_eq!(call_equity(0.7, Street::Turn, false, 100.0, 180.0, &p), 0.7);
    }

    #[test]
    fn all_in_jam_is_scored_against_the_splittable_pot() {
        let sit = sv10_engine::situation::fixtures::river_jam_with_all_ins();
        let mut rng = sv10_rng::rngs::SmallRng::seed_from_u64(11);
        let d = decide_with(&sit, &ModelStore::default(), &Params::default(), None, &mut rng);
        let jam = d.candidates.iter().find(|c| c.action == "all_in").expect("jam considered");
        // Before the fix the jam was scored as if a fold won all 12,536 chips (EV 11,551 here).
        assert!(jam.ev < jam.fold_prob * 12_536.0, "{jam:?}");
    }

    #[test]
    fn neural_response_changes_pricing_without_changing_legal_options() {
        let mut deal_rng = sv10_rng::rngs::SmallRng::seed_from_u64(23);
        let hand = sv10_engine::engine::Hand::new(&[2_000; 3], 0, 10, 20, &mut deal_rng);
        let names = vec!["hero".into(), "villain-a".into(), "villain-b".into()];
        let sit = Situation::from_hand(&hand, hand.actor().expect("preflop actor"), &names);
        let params = Params::default();
        let models = ModelStore::default();
        let decide = |net: Option<&sv10_nn::nn::Mlp>| {
            let mut rng = sv10_rng::rngs::SmallRng::seed_from_u64(29);
            decide_with(&sit, &models, &params, net, &mut rng)
        };
        let stat = decide(None);

        // A width no layout serves is ignored; the previous 37-input layout is still served (0135).
        let mismatch = sv10_nn::nn::Mlp::new(&[sv10_model::features::N_FEATURES + 1, 3], 30);
        let ignored = decide(Some(&mismatch));
        let mut old_layout = sv10_nn::nn::Mlp::new(&[sv10_model::features::INCUMBENT_FEATURES, 3], 33);
        old_layout.layers[0].w.fill(0.0);
        old_layout.layers[0].b = vec![12.0, -12.0, -12.0];
        let old_priced = decide(Some(&old_layout));
        assert!(
            stat.candidates.iter().zip(&old_priced.candidates).any(|(a, b)| a.fold_prob != b.fold_prob),
            "a 37-input network must still price responses"
        );
        let mut fold_heavy = sv10_nn::nn::Mlp::new(&[sv10_model::features::N_FEATURES, 3], 31);
        fold_heavy.layers[0].w.fill(0.0);
        fold_heavy.layers[0].b = vec![12.0, -12.0, -12.0];
        let mut raise_heavy = sv10_nn::nn::Mlp::new(&[sv10_model::features::N_FEATURES, 3], 32);
        raise_heavy.layers[0].w.fill(0.0);
        raise_heavy.layers[0].b = vec![-12.0, -12.0, 12.0];
        let fold_priced = decide(Some(&fold_heavy));
        let raise_priced = decide(Some(&raise_heavy));

        for priced in [&ignored, &fold_priced, &raise_priced] {
            assert_eq!(options(&stat), options(priced), "the response model must not alter protocol legality");
            assert!(priced.candidates.iter().all(|c| c.ev.is_finite() && c.fold_prob.is_finite() && c.equity_called.is_finite()));
        }
        for (fallback, mismatched) in stat.candidates.iter().zip(&ignored.candidates) {
            assert_eq!(
                (fallback.ev, fallback.fold_prob, fallback.equity_called),
                (mismatched.ev, mismatched.fold_prob, mismatched.equity_called)
            );
        }
        assert!(
            fold_priced.candidates.iter().zip(&raise_priced.candidates).any(|(a, b)| a.fold_prob != b.fold_prob && a.ev != b.ev),
            "opposing matching networks did not affect pricing: fold={:?} raise={:?}",
            fold_priced.candidates,
            raise_priced.candidates
        );
    }

    #[test]
    fn a_per_opponent_fold_offset_moves_heads_up_postflop_fold_estimates_only() {
        // 0214: an opponent who folds less than priced is priced as folding less to our flop bets.
        let mut deal_rng = sv10_rng::rngs::SmallRng::seed_from_u64(41);
        let mut hand = sv10_engine::engine::Hand::new(&[2_000; 2], 0, 10, 20, &mut deal_rng);
        hand.apply(Action::Call).unwrap();
        hand.apply(Action::Check).unwrap();
        let actor = hand.actor().expect("flop actor");
        let names: Vec<String> = (0..2).map(|i| if i == actor { "hero".into() } else { "villain".into() }).collect();
        let sit = Situation::from_hand(&hand, actor, &names);
        let params = Params::default();
        let decide = |models: &ModelStore| {
            let mut rng = sv10_rng::rngs::SmallRng::seed_from_u64(43);
            decide_with(&sit, models, &params, None, &mut rng)
        };
        let plain = decide(&ModelStore::default());
        let sticky = ModelStore {
            fold_offsets: std::sync::Arc::new([("villain".to_string(), -1.0f32)].into_iter().collect()),
            ..Default::default()
        };
        let corrected = decide(&sticky);
        assert_eq!(options(&plain), options(&corrected));
        let bets: Vec<_> = plain.candidates.iter().zip(&corrected.candidates).filter(|(a, _)| a.fold_prob > 0.0).collect();
        assert!(!bets.is_empty(), "no bet candidates on the flop");
        for (a, b) in bets {
            assert!(b.fold_prob < a.fold_prob, "{a:?} vs {b:?}");
        }
    }

    #[test]
    fn a_per_opponent_correction_moves_the_network_read_of_that_opponent() {
        // 0210: the same network reads opponents whose residuals say they fold more as folding more.
        let mut deal_rng = sv10_rng::rngs::SmallRng::seed_from_u64(23);
        let hand = sv10_engine::engine::Hand::new(&[2_000; 3], 0, 10, 20, &mut deal_rng);
        let names = vec!["hero".into(), "villain-a".into(), "villain-b".into()];
        let sit = Situation::from_hand(&hand, hand.actor().expect("preflop actor"), &names);
        let params = Params::default();
        let mut neutral = sv10_nn::nn::Mlp::new(&[sv10_model::features::N_FEATURES, 3], 34);
        neutral.layers[0].w.fill(0.0);
        neutral.layers[0].b = vec![0.0, 0.0, 0.0];
        let decide = |models: &ModelStore| {
            let mut rng = sv10_rng::rngs::SmallRng::seed_from_u64(29);
            decide_with(&sit, models, &params, Some(&neutral), &mut rng)
        };
        let plain = decide(&ModelStore::default());
        let folders = ModelStore {
            response_ratios: std::sync::Arc::new(["villain-a", "villain-b"].iter().map(|n| (n.to_string(), [4.0, 1.0, 1.0])).collect()),
            ..Default::default()
        };
        let corrected = decide(&folders);
        assert_eq!(options(&plain), options(&corrected));
        let aggressive = plain.candidates.iter().zip(&corrected.candidates).filter(|(a, _)| a.fold_prob > 0.0);
        let mut seen = 0;
        for (a, b) in aggressive {
            assert!(b.fold_prob >= a.fold_prob, "{a:?} vs {b:?}");
            seen += usize::from(b.fold_prob > a.fold_prob);
        }
        assert!(seen > 0, "no bet or raise was priced as folding more");
    }
}

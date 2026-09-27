//! Opponent range reconstruction: replay the hand's actions and weight each
//! combo by how likely that player (per their learned profile) was to take
//! the observed action with it.

use crate::model::{ModelStore, Profile, aggressive};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;
use sv10_cards::cards::{Card, CardMask};
use sv10_cards::range::{NUM_COMBOS, Range, combo_mask};
use sv10_engine::engine::{ActionKind, ActionRecord, Street};
use sv10_engine::situation::{Position, Situation};
use sv10_equity::equity::combo_strengths;
use sv10_equity::preflop;
use sv10_rng::SeedableRng;
use sv10_rng::rngs::SmallRng;

mod boards;
mod continuation;
pub use boards::*;
pub use continuation::*;

/// Shape constants of the range-reconstruction likelihoods. The defaults are the original
/// hand-set values; `calibrate` fits them by maximum likelihood of real showdown hands given
/// each player's action line, and live play uses the fitted set through `policy::Params`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RangeParams {
    /// Soft threshold width as a share of the top fraction, and its minimum.
    pub soft_width: f32,
    /// Minimum soft threshold width.
    pub soft_min: f32,
    /// Postflop bet/raise: bluff share bounds, size where polarization starts, its exponent,
    /// how much of the bluff share survives, and the floor for any combo.
    pub bluff_min: f32,
    /// Largest bluff share of a betting range.
    pub bluff_max: f32,
    /// Pot fraction at which bet-size polarization starts.
    pub size_anchor: f32,
    /// Exponent of polarization with bet size.
    pub size_exp: f32,
    /// Share of the bluff region that survives into the range.
    pub bluff_keep: f32,
    /// Floor weight for any combo after a bet or raise.
    pub aggr_floor: f32,
    /// Postflop call: width multiplier on the continue fraction, how strongly raising hands
    /// are removed, and the floor.
    pub call_width: f32,
    /// How strongly hands that would have raised are removed from a calling range.
    pub call_raise_trap: f32,
    /// Floor weight for any combo after a call.
    pub call_floor: f32,
    /// Postflop check: how strongly betting hands are removed and at what width.
    pub check_trap: f32,
    /// Top share of the range treated as betting hands when removing them after a check.
    pub check_frac: f32,
    /// Preflop: floor, raise-width multiplier, limp width/trap, flat-call width/trap,
    /// cold-call-of-a-3-bet width, and checked-option trap.
    pub pre_floor: f32,
    /// Width multiplier on preflop raising ranges.
    pub raise_width: f32,
    /// Width multiplier on limping ranges.
    pub limp_width: f32,
    /// How strongly raising hands are removed from limping ranges.
    pub limp_trap: f32,
    /// Top share treated as raising hands for the limp trap.
    pub limp_trap_frac: f32,
    /// Width multiplier on flat-call-of-an-open ranges.
    pub call_open_width: f32,
    /// How strongly 3-betting hands are removed from flat-calling ranges.
    pub call_open_trap: f32,
    /// Top share treated as 3-betting hands for that trap.
    pub call_open_trap_frac: f32,
    /// Width multiplier for cold-calling a 3-bet.
    pub cold_call_width: f32,
    /// How strongly raising hands are removed when the big blind checks its option.
    pub pre_check_trap: f32,
    /// Weight of the player's own positional VPIP in limp/call-first ranges (0 = overall VPIP
    /// scaled by a generic position factor, 1 = positional VPIP only).
    pub limp_positional: f32,
    /// How the bet/raise floor shrinks with bet size: floor × size_scale^this (0 = constant floor,
    /// the original form; >0 lets oversized bets narrow a range instead of flattening it, 0102).
    pub floor_size_exp: f32,
    /// How the bluff share shrinks with bet size: size_scale^this (1 = the original form, <1 =
    /// oversized bets stay polarized with bluffs).
    pub bluff_size_exp: f32,
    /// How the soft threshold's minimum width shrinks with bet size after a bet or raise:
    /// `soft_min × size_scale^this` (0235). 0 = the original form, where even a 100x-pot shove kept
    /// roughly the top `soft_min` of hands soft-included and calls of such shoves over-estimated
    /// equity by +0.34 to +0.49 (0233).
    pub soft_size_exp: f32,
    /// Timing tell (0234): an aggressive postflop action taken in `r` times the player's typical
    /// think time scales their value and bluff shares by `r^-think_exp` (clamped to 1/4..4). Positive
    /// = slower-than-usual aggression is stronger. 0 = timing ignored; `calibrate` fits it once timed
    /// showdowns exist and keeps it only under the held-out gate.
    pub think_exp: f32,
}

impl RangeParams {
    /// The original hand-set values (the baseline a fitted set must beat on held-out showdowns).
    pub const DEFAULT: RangeParams = RangeParams {
        soft_width: 0.12,
        soft_min: 0.025,
        bluff_min: 0.08,
        bluff_max: 0.6,
        size_anchor: 0.66,
        size_exp: 0.5,
        bluff_keep: 0.9,
        aggr_floor: 0.02,
        call_width: 1.0,
        call_raise_trap: 0.5,
        call_floor: 0.03,
        check_trap: 0.65,
        check_frac: 0.6,
        pre_floor: 0.01,
        raise_width: 1.0,
        limp_width: 1.0,
        limp_trap: 0.6,
        limp_trap_frac: 0.6,
        call_open_width: 1.0,
        call_open_trap: 0.65,
        call_open_trap_frac: 0.8,
        cold_call_width: 1.6,
        pre_check_trap: 0.6,
        limp_positional: 0.0,
        floor_size_exp: 0.0,
        bluff_size_exp: 1.0,
        soft_size_exp: 0.0,
        think_exp: 0.0,
    };

    /// Every parameter with its allowed range, for fitting.
    pub fn fields_mut(&mut self) -> Vec<(&'static str, &mut f32, f32, f32)> {
        vec![
            ("soft_width", &mut self.soft_width, 0.03, 0.5),
            ("soft_min", &mut self.soft_min, 0.005, 0.1),
            ("bluff_min", &mut self.bluff_min, 0.0, 0.4),
            ("bluff_max", &mut self.bluff_max, 0.2, 0.9),
            ("size_anchor", &mut self.size_anchor, 0.25, 1.5),
            ("size_exp", &mut self.size_exp, 0.0, 1.5),
            ("bluff_keep", &mut self.bluff_keep, 0.1, 1.0),
            ("aggr_floor", &mut self.aggr_floor, 0.002, 0.2),
            ("call_width", &mut self.call_width, 0.4, 2.0),
            ("call_raise_trap", &mut self.call_raise_trap, 0.0, 0.95),
            ("call_floor", &mut self.call_floor, 0.002, 0.3),
            ("check_trap", &mut self.check_trap, 0.0, 0.95),
            ("check_frac", &mut self.check_frac, 0.1, 1.5),
            ("pre_floor", &mut self.pre_floor, 0.001, 0.1),
            ("raise_width", &mut self.raise_width, 0.4, 2.5),
            ("limp_width", &mut self.limp_width, 0.4, 2.0),
            ("limp_trap", &mut self.limp_trap, 0.0, 0.95),
            ("limp_trap_frac", &mut self.limp_trap_frac, 0.1, 1.5),
            ("call_open_width", &mut self.call_open_width, 0.4, 2.5),
            ("call_open_trap", &mut self.call_open_trap, 0.0, 0.95),
            ("call_open_trap_frac", &mut self.call_open_trap_frac, 0.1, 1.5),
            ("cold_call_width", &mut self.cold_call_width, 0.5, 4.0),
            ("pre_check_trap", &mut self.pre_check_trap, 0.0, 0.95),
            ("limp_positional", &mut self.limp_positional, 0.0, 1.0),
            ("floor_size_exp", &mut self.floor_size_exp, 0.0, 1.5),
            ("bluff_size_exp", &mut self.bluff_size_exp, 0.0, 1.5),
            ("soft_size_exp", &mut self.soft_size_exp, 0.0, 2.0),
            ("think_exp", &mut self.think_exp, -1.5, 1.5),
        ]
    }
}

impl Default for RangeParams {
    fn default() -> Self {
        RangeParams::DEFAULT
    }
}

#[inline]
fn soft_top_p(pct: f32, frac: f32, rp: &RangeParams) -> f32 {
    soft_top_p_min(pct, frac, rp.soft_width, rp.soft_min)
}

/// [`soft_top_p`] with an explicit minimum width (a big bet narrows it, `RangeParams::soft_size_exp`).
#[inline]
fn soft_top_p_min(pct: f32, frac: f32, soft_width: f32, soft_min: f32) -> f32 {
    let frac = frac.clamp(0.0, 1.0);
    let t = (frac * soft_width).max(soft_min);
    1.0 / (1.0 + ((pct - frac) / t).exp())
}

pub use sv10_equity::preflop::soft_top;

/// Weighted percentile of each combo within `range` by strength (0 = strongest).
pub fn range_percentiles(range: &Range, strength: &[f32]) -> Vec<f32> {
    let mut idx: Vec<usize> = (0..NUM_COMBOS).filter(|&i| range.w[i] > 0.0).collect();
    idx.sort_by(|&a, &b| strength[b].partial_cmp(&strength[a]).unwrap_or(std::cmp::Ordering::Equal));
    let total: f32 = idx.iter().map(|&i| range.w[i]).sum();
    let mut pct = vec![1.0f32; NUM_COMBOS];
    if total <= 0.0 {
        return pct;
    }
    let mut acc = 0.0;
    for &i in &idx {
        pct[i] = (acc + range.w[i] * 0.5) / total;
        acc += range.w[i];
    }
    pct
}

fn position_open_factor(pos: Position) -> f32 {
    match pos {
        Position::Early => 0.65,
        Position::Middle => 0.8,
        Position::Cutoff => 1.0,
        Position::Button => 1.35,
        Position::SmallBlind => 1.1,
        Position::BigBlind => 1.0,
    }
}

fn apply(range: &mut Range, f: impl Fn(usize) -> f32) {
    for i in 0..NUM_COMBOS {
        if range.w[i] > 0.0 {
            range.w[i] *= f(i);
        }
    }
}

/// River bet size (added over the pot after calling) at which a sizing tell is neutral (0223).
pub const SIZE_TELL_REF: f64 = 0.66;
/// Bound on the factor a sizing tell applies to the value share: extrapolation guard (LESSONS 29).
const SIZE_TELL_MAX_FACTOR: f64 = 4.0;

/// Factor on the value and bluff shares of a river bet from the bettor's sizing tell (0223); exactly
/// 1 for no tell or before the river, so an untold player replays exactly as before.
pub fn size_tell_factor(tell: f32, size_frac: f64, street: Street) -> f32 {
    if tell == 0.0 || street != Street::River {
        return 1.0;
    }
    (size_frac.max(0.05) / SIZE_TELL_REF).powf(-f64::from(tell)).clamp(1.0 / SIZE_TELL_MAX_FACTOR, SIZE_TELL_MAX_FACTOR) as f32
}

/// Largest factor the timing tell applies to a betting range's shares (extrapolation guard).
const THINK_TELL_MAX_FACTOR: f32 = 4.0;

/// Factor on the value and bluff shares of an aggressive action taken in `ratio` times the
/// player's typical think time (0234); exactly 1 without a timing tell or without timing.
pub fn think_tell_factor(think_exp: f32, ratio: f32) -> f32 {
    if think_exp == 0.0 || ratio <= 0.0 || ratio == 1.0 {
        return 1.0;
    }
    ratio.powf(-think_exp).clamp(1.0 / THINK_TELL_MAX_FACTOR, THINK_TELL_MAX_FACTOR)
}

/// The think-time ratio of an observed action against the actor's typical time in its context
/// (1 = typical, or unknown).
pub fn think_ratio(rec: &ActionRecord, prof: &Profile) -> f32 {
    match rec.think_ms {
        Some(ms) => {
            let base = prof.think_base[crate::model::think_context(rec)];
            if base > 0.0 { ms.max(1) as f32 / base } else { 1.0 }
        }
        None => 1.0,
    }
}

/// Postflop likelihood weights for one observed action.
#[allow(clippy::too_many_arguments)]
pub fn postflop_likelihood(
    kind: ActionKind,
    aggr: bool,
    facing: i64,
    size_frac: f64,
    street: Street,
    prof: &Profile,
    pct: &[f32],
    rp: &RangeParams,
) -> Vec<f32> {
    postflop_likelihood_with(kind, aggr, facing, size_frac, street, prof, pct, rp, 1.0)
}

/// [`postflop_likelihood`] for an action taken in `think_ratio` times the actor's typical think
/// time (0234; 1 = typical or untimed).
#[allow(clippy::too_many_arguments)]
pub fn postflop_likelihood_with(
    kind: ActionKind,
    aggr: bool,
    facing: i64,
    size_frac: f64,
    street: Street,
    prof: &Profile,
    pct: &[f32],
    rp: &RangeParams,
    think_ratio: f32,
) -> Vec<f32> {
    let st = street.index() - 1;
    let bluff = prof.river_bluff.clamp(rp.bluff_min, rp.bluff_max.max(rp.bluff_min));
    let mut out = vec![1.0f32; NUM_COMBOS];
    if aggr {
        let a = if facing == 0 { prof.bet_first[st] } else { prof.raise_vs_bet[st] }.clamp(0.02, 0.95);
        // Bigger bets come from a stronger, more polarized range.
        let anchor = rp.size_anchor as f64;
        let size_scale = if size_frac > anchor { (anchor / size_frac).powf(rp.size_exp as f64) as f32 } else { 1.0 };
        // A sizing tell (0223) tilts the whole betting range by size: below 1 both the value part and
        // the bluffs shrink (a stronger range), above 1 both grow.
        let tell = size_tell_factor(prof.size_tell, size_frac, street) * think_tell_factor(rp.think_exp, think_ratio);
        let value = a * (1.0 - bluff) * size_scale * tell;
        let bluff_share = (a * bluff * size_scale.powf(rp.bluff_size_exp) * tell / (1.0 - value).max(0.05)).min(1.0);
        let floor = if rp.floor_size_exp == 0.0 { rp.aggr_floor } else { rp.aggr_floor * size_scale.powf(rp.floor_size_exp) };
        // A big bet also sharpens the edge of the value region (0235); 1 below the anchor size.
        let soft_min = if rp.soft_size_exp == 0.0 { rp.soft_min } else { rp.soft_min * size_scale.powf(rp.soft_size_exp) };
        for i in 0..NUM_COMBOS {
            let v = soft_top_p_min(pct[i], value, rp.soft_width, soft_min);
            out[i] = v + (1.0 - v) * bluff_share * rp.bluff_keep + floor;
        }
    } else if kind == ActionKind::Call || kind == ActionKind::AllIn {
        let f = prof.fold_to_bet(street, size_frac) as f32;
        let r = prof.raise_vs_bet[st];
        let width = ((1.0 - f) * rp.call_width).min(1.0);
        for i in 0..NUM_COMBOS {
            out[i] = soft_top_p(pct[i], width, rp) * (1.0 - rp.call_raise_trap * soft_top_p(pct[i], r, rp)) + rp.call_floor;
        }
    } else if kind == ActionKind::Check {
        let a = prof.bet_first[st];
        for i in 0..NUM_COMBOS {
            out[i] = 1.0 - rp.check_trap * soft_top_p(pct[i], a * rp.check_frac, rp);
        }
    }
    out
}

/// Estimated range for every live opponent at the current decision point.
pub fn estimate_ranges(sit: &Situation, models: &ModelStore, rp: &RangeParams) -> HashMap<usize, Range> {
    let dead = sit.hole[0].bit() | sit.hole[1].bit() | sit.board.iter().fold(0u64, |m, c| m | c.bit());
    let seats = sit.live_opponents().map(|p| (p.seat, models.profile(&p.name))).collect();
    replay_ranges(sit, seats, dead, rp)
}

/// How hero's range looks to an opponent who models hero with `profile`
/// (hero's own cards are unknown to them, so only the board is dead).
pub fn perceived_range(sit: &Situation, profile: Profile, rp: &RangeParams) -> Range {
    let dead = sit.board.iter().fold(0u64, |m, c| m | c.bit());
    replay_ranges(sit, vec![(sit.hero_seat, profile)], dead, rp).remove(&sit.hero_seat).unwrap_or_else(Range::full)
}

fn replay_ranges(sit: &Situation, seats: Vec<(usize, Profile)>, dead: CardMask, rp: &RangeParams) -> HashMap<usize, Range> {
    replay_ranges_with(sit, seats, dead, rp, &board_info)
}

/// Range replay with explicit parameters and a board provider (the calibrator supplies
/// precomputed boards so a fit never touches the shared board cache).
pub fn replay_ranges_with(
    sit: &Situation,
    seats: Vec<(usize, Profile)>,
    dead: CardMask,
    rp: &RangeParams,
    boards: &dyn Fn(&[Card]) -> Arc<BoardInfo>,
) -> HashMap<usize, Range> {
    let mut ranges: HashMap<usize, Range> = HashMap::new();
    let mut profiles: HashMap<usize, Profile> = HashMap::new();
    for (seat, prof) in seats {
        let mut r = Range::full();
        r.remove_dead(dead);
        ranges.insert(seat, r);
        profiles.insert(seat, prof);
    }
    let pre = preflop::table();
    let mut raises = 0;
    for rec in sit.history.iter().filter(|r| r.street == Street::Preflop) {
        let aggr = aggressive(rec);
        if let (Some(range), Some(prof)) = (ranges.get_mut(&rec.seat), profiles.get(&rec.seat)) {
            let pos = sit.position_of(rec.seat);
            let pct = &pre.combo_percentile;
            let floor = rp.pre_floor;
            if aggr {
                let frac = match raises {
                    0 => prof.open_pos[crate::model::position_group(pos)],
                    1 => prof.three_bet,
                    _ => (prof.four_bet * prof.three_bet * 4.0).max(0.02),
                } * rp.raise_width;
                let frac = frac.clamp(0.02, 0.95);
                apply(range, |i| soft_top_p(pct[i], frac, rp) + floor);
            } else if matches!(rec.kind, ActionKind::Call | ActionKind::AllIn) {
                match raises {
                    0 => {
                        let generic = prof.vpip * position_open_factor(pos).min(1.2);
                        let own = prof.vpip_pos[crate::model::position_group(pos)];
                        let w = rp.limp_positional.clamp(0.0, 1.0);
                        let v = (((1.0 - w) * generic + w * own) * rp.limp_width).clamp(0.05, 1.0);
                        let o = prof.open_raise * rp.limp_trap_frac;
                        apply(range, |i| soft_top_p(pct[i], v, rp) * (1.0 - rp.limp_trap * soft_top_p(pct[i], o, rp)) + floor);
                    }
                    1 => {
                        let c = ((prof.call_open + prof.three_bet) * rp.call_open_width).clamp(0.04, 1.0);
                        let t = prof.three_bet * rp.call_open_trap_frac;
                        apply(range, |i| soft_top_p(pct[i], c, rp) * (1.0 - rp.call_open_trap * soft_top_p(pct[i], t, rp)) + floor);
                    }
                    _ => {
                        let c = (prof.three_bet * rp.cold_call_width).clamp(0.03, 0.6);
                        let t = prof.four_bet * 0.5;
                        apply(range, |i| soft_top_p(pct[i], c, rp) * (1.0 - 0.5 * soft_top_p(pct[i], t, rp)) + floor);
                    }
                }
            } else if rec.kind == ActionKind::Check {
                let pfr = prof.pfr;
                apply(range, |i| 1.0 - rp.pre_check_trap * soft_top_p(pct[i], pfr, rp));
            }
        }
        if aggr {
            raises += 1;
        }
    }
    for street in [Street::Flop, Street::Turn, Street::River] {
        if sit.board.len() < street.board_len() {
            break;
        }
        let info = boards(&sit.board[..street.board_len()]);
        let pct = &info.pct;
        for rec in sit.history.iter().filter(|r| r.street == street) {
            let (Some(range), Some(prof)) = (ranges.get_mut(&rec.seat), profiles.get(&rec.seat)) else {
                continue;
            };
            if rec.kind == ActionKind::Fold {
                continue;
            }
            // Bots act on absolute hand strength, so score actions against the
            // board-wide ordering rather than re-ranking inside an already narrowed range.
            let base = (rec.pot_before - rec.to_call_before).max(1) as f64;
            let size_frac = if aggressive(rec) {
                (rec.to - rec.bet_before - rec.to_call_before) as f64 / (rec.pot_before + rec.to_call_before).max(1) as f64
            } else {
                rec.to_call_before as f64 / base
            };
            let l = postflop_likelihood_with(
                rec.kind,
                aggressive(rec),
                rec.to_call_before,
                size_frac,
                street,
                prof,
                pct,
                rp,
                think_ratio(rec, prof),
            );
            apply(range, |i| l[i]);
        }
    }
    for r in ranges.values_mut() {
        if r.is_empty() {
            *r = Range::full();
            r.remove_dead(dead);
        }
        r.normalize();
    }
    ranges
}

/// Percentile of every combo among all live combos on this board (0 = strongest).
pub fn global_pct(strength: &[f32]) -> Vec<f32> {
    let mut full = Range::full();
    for i in 0..NUM_COMBOS {
        if strength[i] <= 0.0 {
            full.w[i] = 0.0;
        }
    }
    range_percentiles(&full, strength)
}

/// Likelihood weights for a hypothetical aggressive action by `seat` to `to`.
pub fn aggressive_likelihood(sit: &Situation, prof: &Profile, to: i64, range: &Range, strength: &[f32], rp: &RangeParams) -> Vec<f32> {
    let cur = sit.current_bet();
    let hero_bet = sit.players.iter().find(|p| p.seat == sit.hero_seat).map(|p| p.bet).unwrap_or(0);
    if sit.street == Street::Preflop {
        let raises = sit.history.iter().filter(|r| r.street == Street::Preflop && aggressive(r)).count();
        let pos = sit.position_of(sit.hero_seat);
        let bb = sit.bb as f64;
        let mut frac = match raises {
            0 => prof.open_raise * position_open_factor(pos),
            1 => prof.three_bet,
            _ => (prof.four_bet * prof.three_bet * 4.0).max(0.02),
        };
        // Shoves and oversized raises come from a tighter range.
        let normal = match raises {
            0 => 2.5 * bb,
            1 => cur as f64 * 3.3,
            _ => cur as f64 * 2.3,
        };
        if to as f64 > normal * 1.5 {
            frac *= ((normal * 1.5) / to as f64).powf(0.5) as f32;
        }
        let frac = frac.clamp(0.015, 0.95);
        let pct = &preflop::table().combo_percentile;
        return (0..NUM_COMBOS).map(|i| soft_top_p(pct[i], frac, rp) + rp.pre_floor * 0.5).collect();
    }
    let _ = (range, strength);
    let info = board_info(&sit.board);
    let pct = &info.pct;
    let facing = cur - hero_bet;
    let pot_after_call = (sit.pot + facing).max(1) as f64;
    let size_frac = (to - cur) as f64 / pot_after_call;
    postflop_likelihood(ActionKind::Raise, true, facing, size_frac, sit.street, prof, pct, rp)
}

/// Narrow a range to the part expected to continue (top `keep` by strength).
pub fn continuing(range: &Range, strength: &[f32], keep: f32) -> Range {
    let pct = range_percentiles(range, strength);
    let mut r = range.clone();
    apply(&mut r, |i| soft_top(pct[i], keep) + 0.002);
    r
}

/// Cards no opponent can hold: hero's hole cards and the board.
pub fn dead_mask(sit: &Situation) -> CardMask {
    sit.hole[0].bit() | sit.hole[1].bit() | sit.board.iter().fold(0u64, |m, c| m | c.bit())
}

/// Whether combo `i` avoids every dead card.
pub fn combo_live(i: usize, dead: CardMask) -> bool {
    combo_mask(i) & dead == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> Vec<Card> {
        ["Kh", "9d", "4c", "2s"].iter().map(|c| Card::parse(c).unwrap()).collect()
    }

    /// Weight of the weakest decile relative to the strongest decile of combos.
    fn weak_to_strong(lik: &[f32], pct: &[f32]) -> f32 {
        let (mut strong, mut weak, mut ns, mut nw) = (0.0, 0.0, 0, 0);
        for i in 0..NUM_COMBOS {
            if pct[i] < 0.1 {
                strong += lik[i];
                ns += 1;
            } else if pct[i] > 0.9 && pct[i] < 1.0 {
                weak += lik[i];
                nw += 1;
            }
        }
        (weak / nw as f32) / (strong / ns as f32)
    }

    #[test]
    fn aggression_favours_strong_hands_and_checks_remove_them() {
        let info = board_info(&board());
        let prof = ModelStore::default().profile("x");
        let rp = RangeParams::DEFAULT;
        let bet = postflop_likelihood(ActionKind::Raise, true, 0, 0.66, Street::Turn, &prof, &info.pct, &rp);
        assert!(weak_to_strong(&bet, &info.pct) < 0.6, "a bet must favour strong hands");
        let check = postflop_likelihood(ActionKind::Check, false, 0, 0.0, Street::Turn, &prof, &info.pct, &rp);
        assert!(weak_to_strong(&check, &info.pct) > 1.0, "a check must remove strong hands");
    }

    #[test]
    fn size_scaled_floor_lets_overbets_narrow_the_range() {
        let info = board_info(&board());
        let prof = ModelStore::default().profile("x");
        let flat = RangeParams { aggr_floor: 0.12, size_exp: 1.0, ..RangeParams::DEFAULT };
        let scaled = RangeParams { floor_size_exp: 0.56, ..flat };
        let shove = |rp: &RangeParams| postflop_likelihood(ActionKind::AllIn, true, 0, 40.0, Street::Turn, &prof, &info.pct, rp);
        let flat_ratio = weak_to_strong(&shove(&flat), &info.pct);
        let scaled_ratio = weak_to_strong(&shove(&scaled), &info.pct);
        // Old form (0102): a 40x-pot shove barely separates weak from strong hands.
        assert!(flat_ratio > 0.15, "{flat_ratio}");
        assert!(scaled_ratio < flat_ratio / 3.0, "flat {flat_ratio} vs scaled {scaled_ratio}");
        // The defaults keep the original form exactly.
        assert_eq!(shove(&RangeParams { floor_size_exp: 0.0, ..flat }), shove(&flat));
    }

    #[test]
    fn size_scaled_soft_width_concentrates_huge_shoves_on_the_top_hands() {
        let info = board_info(&board());
        let prof = ModelStore::default().profile("x");
        // The live fitted set on 2026-09-26 (range_params.v1), before the new term.
        let live = RangeParams {
            soft_width: 0.2375,
            soft_min: 0.0725,
            bluff_min: 0.28,
            size_anchor: 0.816,
            size_exp: 1.156,
            bluff_keep: 1.0,
            aggr_floor: 0.119,
            floor_size_exp: 0.75,
            bluff_size_exp: 1.469,
            ..RangeParams::DEFAULT
        };
        let sharp = RangeParams { soft_size_exp: 0.5, ..live };
        let top_share = |lik: &[f32]| {
            let total: f32 = (0..NUM_COMBOS).filter(|&i| info.pct[i] < 1.0).map(|i| lik[i]).sum();
            (0..NUM_COMBOS).filter(|&i| info.pct[i] < 0.02).map(|i| lik[i]).sum::<f32>() / total
        };
        let shove = |rp: &RangeParams, size: f64| postflop_likelihood(ActionKind::AllIn, true, 0, size, Street::Turn, &prof, &info.pct, rp);
        // A 100x-pot shove: the old form spreads over roughly the top soft_min of hands.
        let (old, new) = (top_share(&shove(&live, 100.0)), top_share(&shove(&sharp, 100.0)));
        assert!(new > old * 1.5, "top-2% share {old:.3} -> {new:.3}");
        // At or below the anchor size nothing changes, and 0 keeps the original form everywhere.
        assert_eq!(shove(&live, 0.5), shove(&sharp, 0.5));
        assert_eq!(shove(&RangeParams { soft_size_exp: 0.0, ..sharp }, 100.0), shove(&live, 100.0));
    }
}

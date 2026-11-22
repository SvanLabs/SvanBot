//! v10.0.2's range model: every live seat's range, narrowed action by action from a full one, by
//! soft cuts at the share of hands its profile plays that way.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::cards::{Card, CardMask, NUM_COMBOS, Range};
use super::preflop::{self, soft_top};
use super::profile::{Profile, position_group};
use super::situation::{ActionKind, Position, Situation, Street};
use super::strengths::{exact_strengths, river_strengths};

/// The range model's knobs, at v10.0.2's defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RangeParams {
    pub soft_width: f32,
    pub soft_min: f32,
    pub bluff_min: f32,
    pub bluff_max: f32,
    pub size_anchor: f32,
    pub size_exp: f32,
    pub bluff_keep: f32,
    pub aggr_floor: f32,
    pub call_width: f32,
    pub call_raise_trap: f32,
    pub call_floor: f32,
    pub check_trap: f32,
    pub check_frac: f32,
    pub pre_floor: f32,
    pub raise_width: f32,
    pub limp_width: f32,
    pub limp_trap: f32,
    pub limp_trap_frac: f32,
    pub call_open_width: f32,
    pub call_open_trap: f32,
    pub call_open_trap_frac: f32,
    pub cold_call_width: f32,
    pub pre_check_trap: f32,
    pub limp_positional: f32,
    pub floor_size_exp: f32,
    pub bluff_size_exp: f32,
    pub soft_size_exp: f32,
}

impl RangeParams {
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
    };
}

fn soft_top_p(pct: f32, frac: f32, rp: &RangeParams) -> f32 {
    soft_top_p_min(pct, frac, rp.soft_width, rp.soft_min)
}

fn soft_top_p_min(pct: f32, frac: f32, soft_width: f32, soft_min: f32) -> f32 {
    let frac = frac.clamp(0.0, 1.0);
    let t = (frac * soft_width).max(soft_min);
    1.0 / (1.0 + ((pct - frac) / t).exp())
}

/// Where each combo of `range` sits by `strength`, strongest first, weighted by the range.
pub fn range_percentiles(range: &Range, strength: &[f32]) -> Vec<f32> {
    let mut idx: Vec<usize> = (0..NUM_COMBOS).filter(|&i| range.w[i] > 0.0).collect();
    idx.sort_by(|&a, &b| {
        strength[b]
            .partial_cmp(&strength[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
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

/// The percentiles of every combo with a strength, against all of them.
pub fn global_pct(strength: &[f32]) -> Vec<f32> {
    let mut full = Range::full();
    for i in 0..NUM_COMBOS {
        if strength[i] <= 0.0 {
            full.w[i] = 0.0;
        }
    }
    range_percentiles(&full, strength)
}

pub(super) fn position_open_factor(pos: Position) -> f32 {
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

const SIZE_TELL_REF: f64 = 0.66;
const SIZE_TELL_MAX_FACTOR: f64 = 4.0;

fn size_tell_factor(tell: f32, size_frac: f64, street: Street) -> f32 {
    if tell == 0.0 || street != Street::River {
        return 1.0;
    }
    (size_frac.max(0.05) / SIZE_TELL_REF)
        .powf(-f64::from(tell))
        .clamp(1.0 / SIZE_TELL_MAX_FACTOR, SIZE_TELL_MAX_FACTOR) as f32
}

/// A board's combo strengths and their percentiles.
pub struct BoardInfo {
    pub strength: Vec<f32>,
    pub pct: Vec<f32>,
}

/// Board infos already worked out. v10 kept them in a process-wide cache; this one belongs to its
/// caller, and since each is a pure function of its board, the cache changes no result.
#[derive(Default)]
pub struct Boards {
    seen: BTreeMap<(CardMask, usize), Arc<BoardInfo>>,
}

impl Boards {
    /// The strengths v10 used with exact strengths on: the preflop ranking, every run-out on a
    /// flop or turn, and the showdown shares on a river.
    pub fn info(&mut self, board: &[Card]) -> Arc<BoardInfo> {
        let mask = board.iter().fold(0, |m, c| m | c.bit());
        let key = (mask, board.len());
        if let Some(info) = self.seen.get(&key) {
            return Arc::clone(info);
        }
        let strength: Vec<f32> = match board.len() {
            0 => preflop::table()
                .combo_percentile
                .iter()
                .map(|p| 1.0 - p)
                .collect(),
            3 | 4 => exact_strengths(board),
            _ => river_strengths(mask),
        };
        let pct = global_pct(&strength);
        let info = Arc::new(BoardInfo { strength, pct });
        if self.seen.len() > 16_384 {
            self.seen.clear();
        }
        self.seen.insert(key, Arc::clone(&info));
        info
    }
}

/// How much more each combo is to take this action than the range as a whole.
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
    let st = street.index() - 1;
    let bluff = prof
        .river_bluff
        .clamp(rp.bluff_min, rp.bluff_max.max(rp.bluff_min));
    let mut out = vec![1.0f32; NUM_COMBOS];
    if aggr {
        let a = if facing == 0 {
            prof.bet_first[st]
        } else {
            prof.raise_vs_bet[st]
        }
        .clamp(0.02, 0.95);
        let anchor = f64::from(rp.size_anchor);
        let size_scale = if size_frac > anchor {
            (anchor / size_frac).powf(f64::from(rp.size_exp)) as f32
        } else {
            1.0
        };
        let tell = size_tell_factor(prof.size_tell, size_frac, street);
        let value = a * (1.0 - bluff) * size_scale * tell;
        let bluff_share = (a * bluff * size_scale.powf(rp.bluff_size_exp) * tell
            / (1.0 - value).max(0.05))
        .min(1.0);
        let floor = if rp.floor_size_exp == 0.0 {
            rp.aggr_floor
        } else {
            rp.aggr_floor * size_scale.powf(rp.floor_size_exp)
        };
        let soft_min = if rp.soft_size_exp == 0.0 {
            rp.soft_min
        } else {
            rp.soft_min * size_scale.powf(rp.soft_size_exp)
        };
        for i in 0..NUM_COMBOS {
            let v = soft_top_p_min(pct[i], value, rp.soft_width, soft_min);
            out[i] = v + (1.0 - v) * bluff_share * rp.bluff_keep + floor;
        }
    } else if kind == ActionKind::Call || kind == ActionKind::AllIn {
        let f = prof.fold_to_bet(street, size_frac) as f32;
        let r = prof.raise_vs_bet[st];
        let width = ((1.0 - f) * rp.call_width).min(1.0);
        for i in 0..NUM_COMBOS {
            out[i] = soft_top_p(pct[i], width, rp)
                * (1.0 - rp.call_raise_trap * soft_top_p(pct[i], r, rp))
                + rp.call_floor;
        }
    } else if kind == ActionKind::Check {
        let a = prof.bet_first[st];
        for i in 0..NUM_COMBOS {
            out[i] = 1.0 - rp.check_trap * soft_top_p(pct[i], a * rp.check_frac, rp);
        }
    }
    out
}

/// Every live opponent's range, each read with `profile`.
pub fn estimate_ranges(
    sit: &Situation,
    profile: &Profile,
    rp: &RangeParams,
    boards: &mut Boards,
) -> BTreeMap<usize, Range> {
    let dead = sit.hole[0].bit() | sit.hole[1].bit() | sit.board.iter().fold(0, |m, c| m | c.bit());
    let seats: Vec<usize> = sit.live_opponents().map(|p| p.seat).collect();
    replay_ranges(sit, &seats, profile, dead, rp, boards)
}

/// The range the table reads into hero's own line.
pub fn perceived_range(
    sit: &Situation,
    profile: &Profile,
    rp: &RangeParams,
    boards: &mut Boards,
) -> Range {
    let dead = sit.board.iter().fold(0, |m, c| m | c.bit());
    replay_ranges(sit, &[sit.hero_seat], profile, dead, rp, boards)
        .remove(&sit.hero_seat)
        .unwrap_or_else(Range::full)
}

fn replay_ranges(
    sit: &Situation,
    seats: &[usize],
    prof: &Profile,
    dead: CardMask,
    rp: &RangeParams,
    boards: &mut Boards,
) -> BTreeMap<usize, Range> {
    let mut ranges: BTreeMap<usize, Range> = BTreeMap::new();
    for &seat in seats {
        let mut range = Range::full();
        range.remove_dead(dead);
        ranges.insert(seat, range);
    }
    let pct = &preflop::table().combo_percentile;
    let floor = rp.pre_floor;
    let mut raises = 0;
    for rec in sit.history.iter().filter(|r| r.street == Street::Preflop) {
        let aggr = rec.aggressive();
        if let Some(range) = ranges.get_mut(&rec.seat) {
            let pos = sit.position_of(rec.seat);
            if aggr {
                let frac = match raises {
                    0 => prof.open_pos[position_group(pos)],
                    1 => prof.three_bet,
                    _ => (prof.four_bet * prof.three_bet * 4.0).max(0.02),
                } * rp.raise_width;
                let frac = frac.clamp(0.02, 0.95);
                apply(range, |i| soft_top_p(pct[i], frac, rp) + floor);
            } else if matches!(rec.kind, ActionKind::Call | ActionKind::AllIn) {
                let (width, trap, trap_frac) = match raises {
                    0 => {
                        let generic = prof.vpip * position_open_factor(pos).min(1.2);
                        let own = prof.vpip_pos[position_group(pos)];
                        let w = rp.limp_positional.clamp(0.0, 1.0);
                        let v = (((1.0 - w) * generic + w * own) * rp.limp_width).clamp(0.05, 1.0);
                        (v, rp.limp_trap, prof.open_raise * rp.limp_trap_frac)
                    }
                    1 => {
                        let c = ((prof.call_open + prof.three_bet) * rp.call_open_width)
                            .clamp(0.04, 1.0);
                        let t = prof.three_bet * rp.call_open_trap_frac;
                        (c, rp.call_open_trap, t)
                    }
                    _ => {
                        let c = (prof.three_bet * rp.cold_call_width).clamp(0.03, 0.6);
                        (c, 0.5, prof.four_bet * 0.5)
                    }
                };
                apply(range, |i| {
                    soft_top_p(pct[i], width, rp) * (1.0 - trap * soft_top_p(pct[i], trap_frac, rp))
                        + floor
                });
            } else if rec.kind == ActionKind::Check {
                let pfr = prof.pfr;
                apply(range, |i| {
                    1.0 - rp.pre_check_trap * soft_top_p(pct[i], pfr, rp)
                });
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
        let info = boards.info(&sit.board[..street.board_len()]);
        for rec in sit.history.iter().filter(|r| r.street == street) {
            let Some(range) = ranges.get_mut(&rec.seat) else {
                continue;
            };
            if rec.kind == ActionKind::Fold {
                continue;
            }
            let base = (rec.pot_before - rec.to_call_before).max(1) as f64;
            let size_frac = if rec.aggressive() {
                (rec.to - rec.bet_before - rec.to_call_before) as f64
                    / (rec.pot_before + rec.to_call_before).max(1) as f64
            } else {
                rec.to_call_before as f64 / base
            };
            let l = postflop_likelihood(
                rec.kind,
                rec.aggressive(),
                rec.to_call_before,
                size_frac,
                street,
                prof,
                &info.pct,
                rp,
            );
            apply(range, |i| l[i]);
        }
    }
    for range in ranges.values_mut() {
        if range.is_empty() {
            *range = Range::full();
            range.remove_dead(dead);
        }
        range.normalize();
    }
    ranges
}

/// How much more each combo is to raise to `to` from hero's seat than hero's range as a whole.
pub fn aggressive_likelihood(
    sit: &Situation,
    prof: &Profile,
    to: i64,
    rp: &RangeParams,
    boards: &mut Boards,
) -> Vec<f32> {
    let cur = sit.current_bet();
    let hero_bet = sit.hero().bet;
    if sit.street == Street::Preflop {
        let frac = super::continuation::preflop_raise_fraction(sit, prof, to);
        let frac = frac.clamp(0.015, 0.95);
        let pct = &preflop::table().combo_percentile;
        return (0..NUM_COMBOS)
            .map(|i| soft_top_p(pct[i], frac, rp) + rp.pre_floor * 0.5)
            .collect();
    }
    let info = boards.info(&sit.board);
    let facing = cur - hero_bet;
    let pot_after_call = (sit.pot + facing).max(1) as f64;
    let size_frac = (to - cur) as f64 / pot_after_call;
    postflop_likelihood(
        ActionKind::Raise,
        true,
        facing,
        size_frac,
        sit.street,
        prof,
        &info.pct,
        rp,
    )
}

/// The part of `range` that goes on, the top `keep` of it by `strength`, softly.
pub fn continuing(range: &Range, strength: &[f32], keep: f32) -> Range {
    let pct = range_percentiles(range, strength);
    let mut r = range.clone();
    apply(&mut r, |i| soft_top(pct[i], keep) + 0.002);
    r
}

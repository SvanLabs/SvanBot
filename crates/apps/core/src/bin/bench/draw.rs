//! The deal-rejection loop against a branch-free draw (0354).
//!
//! `SharedDeals::new` deals each missing board card by drawing a card index from the whole deck and
//! retrying while the card is already out (`sampler::deal_random`): the one branch in the deal path
//! whose outcome is random. 0336 removed the setup around the deal (memoized ranks, preallocated
//! blocks), so what the rejection loop itself still costs is the open question, and 0339's
//! sub-question 3 — from the operator's SIMD plan — proposes a popcount-select over the free mask
//! as the replacement.
//!
//! `bench draw [--repeat R]` measures the draws against each other *inside one process*: each round
//! sweeps every arm over fixed-size sub-blocks in turn, so an arm's blocks sit next to its
//! neighbours' in time, and the round's values are paired. `ab` compares whole
//! processes and its per-cell 95% interval on this shared box is ±10–17% (0335 baseline), which
//! cannot resolve a few percent of a draw that is itself a few percent of the deal; blocks a few
//! hundred microseconds apart see the same box, so the ratio here is tight enough to decide. The
//! `reject_again` null pair prints its own band, which is the resolution to read the rest against.
//!
//! Every arm deals two cards into a mask that carries the 5 dead cards (hero and a flop) plus 2
//! bits per opponent combo, sampled from the same half-weighted range and the same cards the `micro`
//! suite's deal case uses — the bit count is what sets the collision rate the loop sees.
//!
//! - `reject` — the shipped loop, copied verbatim; `reject_again` is the same arm at the other end
//!   of the round, the null pair: its ratio to `reject` is the instrument's own resolution.
//! - `select_loop` — one uniform draw over the *free* cards, then the j-th of them by clearing the
//!   lowest set bit j times: no retry, but a data-dependent trip count.
//! - `select_pdep` — the same select in one `pdep` (BMI2, runtime-detected; without it the twin
//!   runs in its place). Its block is its own `#[target_feature]` function: an out-of-line `pdep`
//!   call per card would cost more than the draw.
//! - `probe` — the shipped loop's first draw with the collision test removed: the floor a perfectly
//!   predicted draw reaches, so `probe - reject` is the branch with its retries and
//!   `select - probe` is the select's own arithmetic.
//!
//! Nothing here is used in play.

use super::top_range;
use serde_json::{Map, Value, json};
use std::time::Instant;
use sv10_core::cards::{Card, CardMask};
use sv10_core::equity::ComboSampler;
use sv10_core::range::combo_mask;
use sv10_rng::rngs::SmallRng;
use sv10_rng::{RngExt, SeedableRng};
use sv10_stats::moments::{mean_half_width, t975};

/// The 52 cards a draw picks from.
const FREE52: CardMask = (1u64 << 52) - 1;
/// Deal-start masks per sub-block; a sub-block sweeps them `PASSES` times, two cards per mask.
const MASKS: usize = 4_096;
const PASSES: usize = 16;
/// Sub-blocks per arm per round: within a round the arms are swept in turn, so each arm's blocks sit
/// next to its neighbours' in time, and a scheduling event lands on one sub-block of one arm instead
/// of on a whole round (the box carries the fleet, the learner and other agents' builds, 0337).
const SUBS: usize = 16;
/// Rounds a cell runs when `--repeat` is not given: with `SUBS` sub-blocks a round is ~50 ms, so a
/// run costs a couple of seconds and the interval is what the answer rests on.
pub const ROUNDS: usize = 40;
/// The arms, in JSON order: `reject` is the reference every ratio is against.
const ARMS: [&str; 5] = ["reject", "reject_again", "select_loop", "select_pdep", "probe"];
/// The order the arms run in: reversed every round, with the two `reject` blocks as far apart as a
/// sweep allows, so the null pair and every ratio see both ends of the box's drift.
const ORDER_EVEN: [usize; 5] = [0, 2, 3, 4, 1];
const ORDER_ODD: [usize; 5] = [1, 4, 3, 2, 0];

/// `sampler::deal_random` as shipped: draw a card index, retry while the card is already out.
#[inline]
fn reject(rng: &mut SmallRng, used: &mut CardMask) -> Card {
    loop {
        let c = rng.random_range(0..52u8);
        let bit = 1u64 << c;
        if *used & bit == 0 {
            *used |= bit;
            return Card(c);
        }
    }
}

/// One draw over the free cards, then the j-th of them (`trailing_zeros`-style, as the plan
/// suggests): every card is free, at the cost of a trip count that is random too.
#[inline]
fn select_loop(rng: &mut SmallRng, used: &mut CardMask) -> Card {
    let mut free = !*used & FREE52;
    let mut j = rng.random_range(0..free.count_ones() as u8);
    while j > 0 {
        free &= free - 1;
        j -= 1;
    }
    let bit = free.isolate_lowest_one();
    *used |= bit;
    Card(bit.trailing_zeros() as u8)
}

/// The same select, in one `pdep`. Out of line so the self-check has a call to make; the timed path
/// is [`block_pdep`], which inlines these two instructions.
///
/// SAFETY: the caller has checked `bmi2` at runtime (`_pdep_u64` is `#[target_feature]`).
#[cfg(target_arch = "x86_64")]
fn select_pdep(rng: &mut SmallRng, used: &mut CardMask) -> Card {
    let free = !*used & FREE52;
    let j = rng.random_range(0..free.count_ones() as u8);
    let bit = unsafe { core::arch::x86_64::_pdep_u64(1u64 << j, free) };
    *used |= bit;
    Card(bit.trailing_zeros() as u8)
}

#[cfg(not(target_arch = "x86_64"))]
fn select_pdep(rng: &mut SmallRng, used: &mut CardMask) -> Card {
    select_loop(rng, used)
}

fn bmi2_available() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("bmi2")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

/// The shipped loop with its collision test removed: one draw per card, nothing data-dependent. It
/// deals cards that can collide on purpose — it is the floor, not a candidate.
#[inline]
fn probe(rng: &mut SmallRng, used: &mut CardMask) -> Card {
    let c = rng.random_range(0..52u8);
    *used |= 1u64 << c;
    Card(c)
}

/// A checksum of the cards a block dealt, so no draw can be optimized away.
#[inline]
fn folded(acc: u64, a: u8, b: u8) -> u64 {
    acc.wrapping_add((u64::from(a) << 8) ^ u64::from(b))
}

/// One block: `PASSES` sweeps over `masks`, two cards per mask with `draw`. Returns the time per
/// card (ns) and the checksum.
fn block(draw: impl Fn(&mut SmallRng, &mut CardMask) -> Card, seed: u64, masks: &[CardMask]) -> (f64, u64) {
    let mut rng = SmallRng::seed_from_u64(seed);
    let mut acc = 0u64;
    let t = Instant::now();
    for _ in 0..PASSES {
        for &m in masks {
            let mut used = m;
            let a = draw(&mut rng, &mut used);
            let b = draw(&mut rng, &mut used);
            acc = folded(acc, a.0, b.0);
        }
    }
    (t.elapsed().as_secs_f64() * 1e9 / (2 * PASSES * masks.len()) as f64, acc)
}

/// [`block`] with the BMI2 select inlined: `#[target_feature]` is what lets the compiler inline
/// `_pdep_u64` at all, and a per-card call would cost more than the draw it measures.
///
/// SAFETY: the caller has checked `bmi2` at runtime.
#[target_feature(enable = "bmi2")]
#[cfg(target_arch = "x86_64")]
unsafe fn block_pdep(seed: u64, masks: &[CardMask]) -> (f64, u64) {
    let mut rng = SmallRng::seed_from_u64(seed);
    let mut acc = 0u64;
    let t = Instant::now();
    for _ in 0..PASSES {
        for &m in masks {
            let mut used = m;
            let (mut a, mut b) = (0u8, 0u8);
            for slot in [&mut a, &mut b] {
                let free = !used & FREE52;
                let j = rng.random_range(0..free.count_ones() as u8);
                let bit = core::arch::x86_64::_pdep_u64(1u64 << j, free);
                used |= bit;
                *slot = bit.trailing_zeros() as u8;
            }
            acc = folded(acc, a, b);
        }
    }
    (t.elapsed().as_secs_f64() * 1e9 / (2 * PASSES * masks.len()) as f64, acc)
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn block_pdep(seed: u64, masks: &[CardMask]) -> (f64, u64) {
    block(select_loop, seed, masks)
}

/// The BMI2 arm must be the twin's algorithm — same stream in, same card out, always a free card —
/// or the comparison measures two different draws. Every run checks it (the bench bin has no test
/// harness of its own, 0226, so the check lives in the tool and holds in the build being measured).
/// One deck is enough: it deals every card `start` leaves free, and the masks the timed arms use keep
/// at least 39 cards free (`used` holds at most 13 bits) with two cards per mask, so a mask that
/// fills up — where a draw over the free cards has no card to give, and `random_range(0..0)` panics
/// — is not reachable there.
fn pdep_agrees_with_twin(start: CardMask) -> bool {
    if !bmi2_available() {
        return true;
    }
    let (mut a, mut b) = (SmallRng::seed_from_u64(0x0354), SmallRng::seed_from_u64(0x0354));
    let (mut ua, mut ub) = (start, start);
    (0..(FREE52 & !start).count_ones()).all(|n| {
        let x = select_pdep(&mut a, &mut ua);
        let y = select_loop(&mut b, &mut ub);
        x == y && ua == ub && ua.count_ones() == start.count_ones() + n + 1
    })
}

/// Deal-start masks: hero (`Ah Kd`) and a flop (`7s 8s 2c`) are out, plus `k` opponent combos
/// sampled from the half-weighted range the `micro` deal case uses, so the masks carry the same
/// 5 + 2k set bits the real loop's `used` carries.
fn start_masks(k: usize, count: usize) -> Vec<CardMask> {
    let dead = ["Ah", "Kd", "7s", "8s", "2c"].iter().fold(0, |m, s| m | Card::parse(s).expect("a literal card").bit());
    let sampler = ComboSampler::new(&top_range(0.5), dead);
    let mut rng = SmallRng::seed_from_u64(0x0354);
    (0..count)
        .map(|_| {
            let mut used = dead;
            for _ in 0..k {
                used |= combo_mask(sampler.sample(&mut rng));
            }
            used
        })
        .collect()
}

fn round4(x: f64) -> f64 {
    (x * 1e4).round() / 1e4
}

/// The middle value (the mean of the two middle ones for an even count): the summary the 0335
/// baseline table quotes, and the robust half of this tool's verdict (a mean interval also prints).
fn median(xs: &[f64]) -> f64 {
    let mut s = xs.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    if n % 2 == 1 { s[n / 2] } else { 0.5 * (s[n / 2 - 1] + s[n / 2]) }
}

/// One density: every arm over `rounds` rounds, each ratio paired round by round against `reject`.
fn cell(k: usize, rounds: usize, masks: &[CardMask]) -> Value {
    let pdep = bmi2_available();
    let mut ns: [Vec<f64>; 5] = std::array::from_fn(|_| Vec::with_capacity(rounds));
    let mut checksums = [0u64; 5];
    for round in 0..rounds {
        let order = if round % 2 == 0 { ORDER_EVEN } else { ORDER_ODD };
        let mut total = [0.0f64; 5];
        for sub in 0..SUBS {
            let seed = 0x0354_0000 + (round * SUBS + sub) as u64;
            for arm in order {
                let (t, sum) = match (arm, pdep) {
                    (0 | 1, _) => block(reject, seed, masks),
                    (2, _) => block(select_loop, seed, masks),
                    // SAFETY: arm 3 takes this branch only when `pdep` is true, which is
                    // `is_x86_feature_detected!("bmi2")` above — the one feature `block_pdep` enables.
                    (3, true) => unsafe { block_pdep(seed, masks) },
                    (3, false) => block(select_loop, seed, masks),
                    _ => block(probe, seed, masks),
                };
                total[arm] += t;
                checksums[arm] = checksums[arm].wrapping_add(sum);
            }
        }
        for (arm, t) in ns.iter_mut().enumerate() {
            t.push(total[arm] / SUBS as f64);
        }
    }
    let mut out = Map::new();
    out.insert("opponents".into(), json!(k));
    out.insert("cards_per_deal".into(), json!(2));
    for (i, name) in ARMS.iter().enumerate() {
        let mean = ns[i].iter().sum::<f64>() / rounds as f64;
        out.insert(format!("{name}_ns_per_card"), json!(round4(mean)));
        out.insert(format!("{name}_ns_per_deal"), json!(round4(2.0 * mean)));
    }
    let mut checks = Map::new();
    for (i, name) in ARMS.iter().enumerate() {
        checks.insert((*name).into(), json!(format!("{:016x}", checksums[i])));
    }
    out.insert("checksums".into(), Value::Object(checks));
    for (i, name) in ARMS.iter().enumerate().skip(1) {
        // Lower is better; a ratio's interval excluding 1 is a real difference (0335's rule). The
        // interval is the mean's, as `ab` reports it; the median is the robust point
        // estimate, and the null pair's own band is this instrument's resolution.
        let ratios: Vec<f64> = (0..rounds).map(|r| ns[i][r] / ns[0][r]).collect();
        // One round has no spread to build an interval from, and a zero-width one would read as
        // decisive: say so instead.
        if rounds < 2 {
            let r = round4(ratios[0]);
            out.insert(format!("{name}_over_reject"), json!({"ratio": r, "median": r, "ci95": null, "verdict": "one round: no interval"}));
            continue;
        }
        let (mean, half) = mean_half_width(&ratios, t975(rounds - 1));
        let (lo, hi) = (mean - half, mean + half);
        let verdict = if hi < 1.0 {
            "faster"
        } else if lo > 1.0 {
            "slower"
        } else {
            "no measurable change"
        };
        out.insert(
            format!("{name}_over_reject"),
            json!({"ratio": round4(mean), "median": round4(median(&ratios)), "ci95": [round4(lo), round4(hi)], "verdict": verdict}),
        );
    }
    Value::Object(out)
}

/// `bench draw [--repeat R]`: the ablation over the two densities `deal_ns_per_sample_k` reports.
pub fn run(rounds: usize) -> Value {
    let masks = start_masks(1, MASKS);
    let mut out = Map::new();
    out.insert("suite".into(), json!("draw"));
    out.insert("rounds".into(), json!(rounds));
    out.insert("sub_blocks_per_arm_per_round".into(), json!(SUBS));
    out.insert("cards_per_sub_block".into(), json!(2 * MASKS * PASSES));
    out.insert("bmi2".into(), json!(bmi2_available()));
    out.insert("pdep_agrees_with_select_loop".into(), json!(pdep_agrees_with_twin(masks[0])));
    out.insert("k1".into(), cell(1, rounds, &masks));
    out.insert("k2".into(), cell(2, rounds, &start_masks(2, MASKS)));
    Value::Object(out)
}

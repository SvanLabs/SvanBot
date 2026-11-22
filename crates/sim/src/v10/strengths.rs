//! v10.0.2's combo strengths on a board: how each combo ranks against every other, blended from
//! where its made hand sits now and its share at showdown over the run-outs.

use svanbot_core::rng::Rng;

use super::cards::{Card, CardMask, NUM_COMBOS, Range, combo_mask, combos};
use super::draw;
use super::equity::equity_vs_ranges;
use super::eval::eval;

/// Every combo's showdown share on a full board against every other live combo, by card removal.
pub fn river_strengths(board_mask: CardMask) -> Vec<f32> {
    let table = combos();
    let made = made_hands(board_mask);
    let live = made.len() as i64;
    let mut per_card = [0i64; 52];
    for &(_, i) in &made {
        let (a, b) = table.cards[i];
        per_card[usize::from(a.0)] += 1;
        per_card[usize::from(b.0)] += 1;
    }
    let mut out = vec![0f32; NUM_COMBOS];
    let mut below = 0i64;
    let mut below_card = [0i64; 52];
    let mut k = 0;
    while k < made.len() {
        let mut j = k;
        let mut group_card = [0i64; 52];
        while j < made.len() && made[j].0 == made[k].0 {
            let (a, b) = table.cards[made[j].1];
            group_card[usize::from(a.0)] += 1;
            group_card[usize::from(b.0)] += 1;
            j += 1;
        }
        let group = (j - k) as i64;
        for &(_, i) in &made[k..j] {
            let (a, b) = table.cards[i];
            let (a, b) = (usize::from(a.0), usize::from(b.0));
            let total = live - per_card[a] - per_card[b] + 1;
            let wins = below - below_card[a] - below_card[b];
            let ties = group - group_card[a] - group_card[b] + 1;
            out[i] = if total == 0 {
                0.5
            } else {
                ((wins as f64 + ties as f64 * 0.5) / total as f64) as f32
            };
        }
        for c in 0..52 {
            below_card[c] += group_card[c];
        }
        below += group;
        k = j;
    }
    out
}

/// The live combos' hand values on `board_mask`, weakest first; ties keep combo order, as v10's
/// radix sort kept them.
fn made_hands(board_mask: CardMask) -> Vec<(u32, usize)> {
    let mut made: Vec<(u32, usize)> = (0..NUM_COMBOS)
        .filter(|&i| combo_mask(i) & board_mask == 0)
        .map(|i| (eval(combo_mask(i) | board_mask), i))
        .collect();
    made.sort_by_key(|&(value, _)| value);
    made
}

/// Where each combo's made hand sits among the live ones, a tie at its middle.
fn made_percentiles(board_mask: CardMask) -> Vec<f32> {
    let made = made_hands(board_mask);
    let mut out = vec![0f32; NUM_COMBOS];
    let n = made.len() as f32;
    let mut k = 0;
    while k < made.len() {
        let mut j = k;
        while j < made.len() && made[j].0 == made[k].0 {
            j += 1;
        }
        let mid = (k + j) as f32 * 0.5 / n;
        for &(_, i) in &made[k..j] {
            out[i] = mid;
        }
        k = j;
    }
    out
}

/// The strengths on a flop or turn over every run-out: 0.55 of the made percentile and 0.45 of the
/// mean river share.
pub fn exact_strengths(board: &[Card]) -> Vec<f32> {
    let board_mask = board.iter().fold(0, |mask, card| mask | card.bit());
    let rest: Vec<Card> = (0..52)
        .map(Card)
        .filter(|card| board_mask & card.bit() == 0)
        .collect();
    let mut sum = vec![0f64; NUM_COMBOS];
    let mut count = vec![0u32; NUM_COMBOS];
    let mut add = |mask: CardMask| {
        let shares = river_strengths(mask);
        for i in 0..NUM_COMBOS {
            if combo_mask(i) & mask == 0 {
                sum[i] += f64::from(shares[i]);
                count[i] += 1;
            }
        }
    };
    match board.len() {
        4 => rest.iter().for_each(|river| add(board_mask | river.bit())),
        3 => {
            for (k, turn) in rest.iter().enumerate() {
                for river in &rest[k + 1..] {
                    add(board_mask | turn.bit() | river.bit());
                }
            }
        }
        _ => panic!("exact_strengths needs a flop or turn"),
    }
    let made = made_percentiles(board_mask);
    (0..NUM_COMBOS)
        .map(|i| {
            if count[i] == 0 {
                0.0
            } else {
                0.55 * made[i] + 0.45 * (sum[i] / f64::from(count[i])) as f32
            }
        })
        .collect()
}

/// The strengths from a draw of `samples_per_combo` run-outs per combo, or the river shares on a
/// full board.
pub fn combo_strengths(board: &[Card], samples_per_combo: usize, rng: &mut Rng) -> Vec<f32> {
    let board_mask = board.iter().fold(0, |mask, card| mask | card.bit());
    if board.len() == 5 {
        return river_strengths(board_mask);
    }
    let full = Range::full();
    let table = combos();
    let made = made_percentiles(board_mask);
    let base_seed = draw::u64(rng);
    let mut out = vec![0f32; NUM_COMBOS];
    for i in (0..NUM_COMBOS).filter(|&i| combo_mask(i) & board_mask == 0) {
        let (a, b) = table.cards[i];
        let seed = base_seed ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let equity = equity_vs_ranges(
            [a, b],
            board,
            &[&full],
            samples_per_combo,
            &mut Rng::seeded(seed),
        )
        .expect("a per-combo strength draw fills its budget");
        out[i] = 0.55 * made[i] + 0.45 * equity as f32;
    }
    out
}

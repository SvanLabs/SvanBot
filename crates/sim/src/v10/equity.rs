//! v10.0.2's Monte Carlo equity against weighted ranges, and the combo sampler it deals from.
//!
//! v10 split a draw over `chunks` parallel jobs, one seed each drawn up front, and averaged them
//! in chunk order; the jobs here run one after another, which gives the same number.

use svanbot_core::rng::Rng;

use super::cards::{Card, CardMask, NUM_COMBOS, Range, combo_mask};
use super::draw;
use super::eval::eval;

/// Attempts a rejection-sampled deal may take per sample before it gives up.
pub(super) const ATTEMPTS_PER_SAMPLE: usize = 200;

/// Draws combos of a range, in proportion to their weights, with dead cards removed.
pub struct ComboSampler {
    /// The running total of the weights, one per live combo.
    cum: Vec<f64>,
    /// The combo behind each entry of `cum`.
    idx: Vec<u16>,
    total: f64,
}

impl ComboSampler {
    /// A sampler over `range` without the combos that touch `dead`.
    pub fn new(range: &Range, dead: CardMask) -> Self {
        let mut cum = Vec::with_capacity(NUM_COMBOS);
        let mut idx = Vec::with_capacity(NUM_COMBOS);
        let mut total = 0.0;
        for (i, &weight) in range.w.iter().enumerate() {
            let weight = f64::from(weight);
            if weight > 0.0 && combo_mask(i) & dead == 0 {
                total += weight;
                cum.push(total);
                idx.push(i as u16);
            }
        }
        Self { cum, idx, total }
    }

    /// Whether no combo has weight: the caller falls back to a full range.
    pub fn is_empty(&self) -> bool {
        self.idx.is_empty()
    }

    /// One combo, by the inverse of the running total at one uniform draw. v10 searched a guided
    /// window; the partition point is the same.
    pub fn sample(&self, rng: &mut Rng) -> usize {
        let x = draw::f64(rng) * self.total;
        let below = self.cum.partition_point(|&c| c <= x);
        usize::from(self.idx[below.min(self.idx.len() - 1)])
    }
}

/// A card not yet in `used`, drawn uniformly and added to it.
pub(super) fn deal_random(rng: &mut Rng, used: &mut CardMask) -> Card {
    loop {
        let card = draw::below(52, rng) as u8;
        let bit = 1 << card;
        if *used & bit == 0 {
            *used |= bit;
            return Card(card);
        }
    }
}

/// The samplers for `opponents`, a full range standing in for one with nothing left.
fn samplers(opponents: &[&Range], dead: CardMask) -> Vec<ComboSampler> {
    let full = Range::full();
    opponents
        .iter()
        .map(|range| {
            let sampler = ComboSampler::new(range, dead);
            if sampler.is_empty() {
                ComboSampler::new(&full, dead)
            } else {
                sampler
            }
        })
        .collect()
}

/// Hero's share of the pot against `opponents`, over `samples` deals. `None` when the draw could
/// not fill its budget, rather than a mean over fewer deals than were asked for.
pub fn equity_vs_ranges(
    hero: [Card; 2],
    board: &[Card],
    opponents: &[&Range],
    samples: usize,
    rng: &mut Rng,
) -> Option<f64> {
    let hero_mask = hero[0].bit() | hero[1].bit();
    let board_mask = board.iter().fold(0, |mask, card| mask | card.bit());
    let dead = hero_mask | board_mask;
    let samplers = samplers(opponents, dead);
    let missing = 5 - board.len();
    let (mut won, mut n, mut attempts) = (0.0, 0, 0);
    let mut opp_masks = vec![0u64; samplers.len()];
    'deal: while n < samples && attempts < samples * ATTEMPTS_PER_SAMPLE {
        attempts += 1;
        let mut used = dead;
        for (mask, sampler) in opp_masks.iter_mut().zip(&samplers) {
            let combo = combo_mask(sampler.sample(rng));
            if combo & used != 0 {
                continue 'deal;
            }
            used |= combo;
            *mask = combo;
        }
        let mut full_board = board_mask;
        for _ in 0..missing {
            full_board |= deal_random(rng, &mut used).bit();
        }
        let hero_value = eval(hero_mask | full_board);
        let (mut best, mut at_best) = (0, 0);
        for &mask in &opp_masks {
            let value = eval(mask | full_board);
            if value > best {
                best = value;
                at_best = 1;
            } else if value == best {
                at_best += 1;
            }
        }
        if hero_value > best {
            won += 1.0;
        } else if hero_value == best {
            won += 1.0 / (f64::from(at_best) + 1.0);
        }
        n += 1;
    }
    (samples > 0 && n == samples).then(|| won / n as f64)
}

/// [`equity_vs_ranges`] cut into `chunks`, one seed each drawn from `rng`, averaged by the
/// samples each chunk took. `None` unless every chunk filled its share.
pub fn equity_vs_ranges_chunked(
    hero: [Card; 2],
    board: &[Card],
    opponents: &[&Range],
    samples: usize,
    chunks: usize,
    rng: &mut Rng,
) -> Option<f64> {
    if chunks <= 1 {
        return equity_vs_ranges(hero, board, opponents, samples, rng);
    }
    let seeds: Vec<u64> = (0..chunks).map(|_| draw::u64(rng)).collect();
    let (mut sum, mut weight) = (0.0, 0);
    for (i, &seed) in seeds.iter().enumerate() {
        let share = samples / chunks + usize::from(i < samples % chunks);
        let mut chunk_rng = Rng::seeded(seed);
        if let Some(equity) = equity_vs_ranges(hero, board, opponents, share, &mut chunk_rng) {
            sum += equity * share as f64;
            weight += share;
        }
    }
    (samples > 0 && weight == samples).then(|| sum / weight as f64)
}

//! Monte Carlo equity against opponent ranges (0261).

use super::sampler::{ComboSampler, deal_random};
use sv10_cards::cards::Card;
use sv10_cards::eval::eval;
use sv10_cards::range::{Range, combo_mask};
use sv10_rng::{Rng, RngExt};

/// Monte Carlo share of the pot hero wins at showdown against every opponent
/// range simultaneously (ties split). Empty opponent ranges are treated as random.
pub fn equity_vs_ranges<R: Rng>(hero: [Card; 2], board: &[Card], opponents: &[&Range], samples: usize, rng: &mut R) -> f64 {
    let hero_mask = hero[0].bit() | hero[1].bit();
    let board_mask = board.iter().fold(0u64, |m, c| m | c.bit());
    let dead = hero_mask | board_mask;
    let full = Range::full();
    let samplers: Vec<ComboSampler> = opponents
        .iter()
        .map(|r| {
            let s = ComboSampler::new(r, dead);
            if s.is_empty() { ComboSampler::new(&full, dead) } else { s }
        })
        .collect();
    let missing = 5 - board.len();
    let mut won = 0.0;
    let mut n = 0usize;
    let mut attempts = 0usize;
    let mut opp_masks = vec![0u64; samplers.len()];
    while n < samples && attempts < samples * 20 {
        attempts += 1;
        let mut used = dead;
        let mut ok = true;
        for (k, s) in samplers.iter().enumerate() {
            let m = combo_mask(s.sample(rng));
            if m & used != 0 {
                ok = false;
                break;
            }
            used |= m;
            opp_masks[k] = m;
        }
        if !ok {
            continue;
        }
        let mut full_board = board_mask;
        for _ in 0..missing {
            full_board |= deal_random(rng, &mut used).bit();
        }
        let hv = eval(hero_mask | full_board);
        let mut best_opp = 0;
        let mut at_best = 0;
        for &m in &opp_masks {
            let v = eval(m | full_board);
            if v > best_opp {
                best_opp = v;
                at_best = 1;
            } else if v == best_opp {
                at_best += 1;
            }
        }
        if hv > best_opp {
            won += 1.0;
        } else if hv == best_opp {
            won += 1.0 / (at_best as f64 + 1.0);
        }
        n += 1;
    }
    if n == 0 { 0.0 } else { won / n as f64 }
}

/// [`equity_vs_ranges`] split over `chunks` parallel jobs, one seed per chunk drawn from `rng`, averaged by
/// each chunk's sample share; reproducible for a given `rng` and `chunks`. `chunks <= 1` is `equity_vs_ranges`.
pub fn equity_vs_ranges_parallel<R: Rng>(
    hero: [Card; 2],
    board: &[Card],
    opponents: &[&Range],
    samples: usize,
    chunks: usize,
    rng: &mut R,
) -> f64 {
    use rayon::prelude::*;
    use sv10_rng::SeedableRng;
    if chunks <= 1 {
        return equity_vs_ranges(hero, board, opponents, samples, rng);
    }
    let seeds: Vec<u64> = (0..chunks).map(|_| rng.random::<u64>()).collect();
    let (sum, weight) = seeds
        .par_iter()
        .enumerate()
        .map(|(i, seed)| {
            let share = samples / chunks + usize::from(i < samples % chunks);
            let e = equity_vs_ranges(hero, board, opponents, share, &mut sv10_rng::rngs::SmallRng::seed_from_u64(*seed));
            (e * share as f64, share as f64)
        })
        .collect::<Vec<_>>()
        .into_iter()
        .fold((0.0, 0.0), |(a, b), (x, w)| (a + x, b + w));
    if weight > 0.0 { sum / weight } else { 0.0 }
}

//! Monte Carlo equity against opponent ranges (0261).

use super::sampler::{ATTEMPTS_PER_SAMPLE, ComboSampler, deal_random};
use sv10_cards::cards::Card;
use sv10_cards::eval::eval;
use sv10_cards::range::{Range, combo_mask};
use sv10_rng::{Rng, RngExt};

/// Monte Carlo share of the pot hero wins at showdown against every opponent
/// range simultaneously (ties split). Empty opponent ranges are treated as random.
pub fn equity_vs_ranges<R: Rng>(hero: [Card; 2], board: &[Card], opponents: &[&Range], samples: usize, rng: &mut R) -> f64 {
    equity_vs_ranges_counted(hero, board, opponents, samples, rng).0
}

/// [`equity_vs_ranges`] with the number of deals it actually scored: the average is taken over the
/// deals that were accepted, so a draw that runs out of attempts answers from fewer of them, and
/// returning the count is what lets a test see that (#383). `None` is never returned — the equity
/// is `0.0` when a draw accepted nothing at all.
fn equity_vs_ranges_counted<R: Rng>(hero: [Card; 2], board: &[Card], opponents: &[&Range], samples: usize, rng: &mut R) -> (f64, usize) {
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
    while n < samples && attempts < samples * ATTEMPTS_PER_SAMPLE {
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
    (if n == 0 { 0.0 } else { won / n as f64 }, n)
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

#[cfg(test)]
mod tests {
    use super::*;
    use sv10_rng::SeedableRng;

    /// A range with `density` of its combos live and most of them light: the sparse, overlapping
    /// shape that collides hardest, and the same generator `deals/tests.rs` uses.
    fn sparse(rng: &mut sv10_rng::rngs::SmallRng, density: f64) -> Range {
        let mut r = Range::empty();
        for w in r.w.iter_mut() {
            let u: f64 = rng.random();
            *w = if u > density {
                0.0
            } else if u < density * 0.3 {
                1e-4 * rng.random::<f32>()
            } else {
                rng.random::<f32>() * 3.0
            };
        }
        r
    }

    /// `n` distinct cards drawn from `rng`.
    fn cards(rng: &mut sv10_rng::rngs::SmallRng, n: usize) -> Vec<Card> {
        let mut used = 0u64;
        let mut out = Vec::new();
        while out.len() < n {
            let c = rng.random_range(0..52u8);
            if used & (1u64 << c) == 0 {
                used |= 1u64 << c;
                out.push(Card(c));
            }
        }
        out
    }

    /// This is what the decision path falls back to when the shared deal set cannot answer, and it
    /// draws through the same rejection loop: at 20 attempts per sample a nine-handed table over
    /// sparse ranges averaged fewer than the 2500 deals it was asked for (#383).
    #[test]
    fn a_full_ring_table_scores_every_sample_it_asked_for() {
        let samples = 2_500;
        let mut meta = sv10_rng::rngs::SmallRng::seed_from_u64(12_644);
        let dealt = cards(&mut meta, 5);
        let (hero, board) = ([dealt[0], dealt[1]], &dealt[2..]);
        let ranges: Vec<Range> = (0..9).map(|_| sparse(&mut meta, 0.02)).collect();
        let refs: Vec<&Range> = ranges.iter().collect();
        let (_, scored) = equity_vs_ranges_counted(hero, board, &refs, samples, &mut sv10_rng::rngs::SmallRng::seed_from_u64(7));
        assert_eq!(scored, samples, "a nine-handed draw scored {scored} of {samples} deals");
    }
}

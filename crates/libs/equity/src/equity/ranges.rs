//! Monte Carlo equity against opponent ranges (0261).

use super::sampler::{ATTEMPTS_PER_SAMPLE, ComboSampler, deal_random};
use sv10_cards::cards::Card;
use sv10_cards::eval::eval;
use sv10_cards::range::{Range, combo_mask};
use sv10_rng::{Rng, RngExt};

/// Monte Carlo share of the pot hero wins at showdown against every opponent
/// range simultaneously (ties split). Empty opponent ranges are treated as random.
///
/// `None` when the draw cannot answer for the deals it was asked for: it scored none at all, or it
/// ran out of attempts before it scored `samples` of them (#424). A draw that fell short is not a
/// slightly worse number — measured on sparse mutually-colliding ranges, eleven opponents yield
/// 1700 of 2500 samples and thirteen yield 63 — and only the caller knows whether the spot is
/// priced on the answer or merely informed by it. The decision path refuses the spot; offline
/// table builders that cannot be short say so where they call this. The `0.0` this used to answer
/// with read as "hero never wins", a claim no deal supports.
pub fn equity_vs_ranges<R: Rng>(hero: [Card; 2], board: &[Card], opponents: &[&Range], samples: usize, rng: &mut R) -> Option<f64> {
    equity_vs_ranges_counted(hero, board, opponents, samples, rng).0
}

/// [`equity_vs_ranges`] with the number of deals it actually scored: the average is taken over the
/// deals that were accepted, so a draw that runs out of attempts answers from fewer of them, and
/// returning the count is what lets a test see that (#383) and hold the refusal to its reason
/// (#424). `samples == 0` asks for no measurement, and gets none.
fn equity_vs_ranges_counted<R: Rng>(
    hero: [Card; 2],
    board: &[Card],
    opponents: &[&Range],
    samples: usize,
    rng: &mut R,
) -> (Option<f64>, usize) {
    // A hold'em board has at most five cards. Longer is malformed, and has no equity: the deal count below
    // is 5 - board.len(), which wraps in release and never returns (#944).
    if board.len() > 5 {
        return (None, 0);
    }
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
    ((samples > 0 && n == samples).then(|| won / n as f64), n)
}

/// [`equity_vs_ranges`] split over `chunks` parallel jobs, one seed per chunk drawn from `rng`, averaged by
/// each chunk's sample share; reproducible for a given `rng` and `chunks`. `chunks <= 1` is `equity_vs_ranges`.
/// Refuses whenever the chunks together did not fill the ask: a chunk that fell short keeps its full
/// share of the average today, which is how one twelfth of the deals still reads as all of them (#424).
pub fn equity_vs_ranges_parallel<R: Rng>(
    hero: [Card; 2],
    board: &[Card],
    opponents: &[&Range],
    samples: usize,
    chunks: usize,
    rng: &mut R,
) -> Option<f64> {
    use rayon::prelude::*;
    use sv10_rng::SeedableRng;
    if chunks <= 1 {
        return equity_vs_ranges(hero, board, opponents, samples, rng);
    }
    let seeds: Vec<u64> = (0..chunks).map(|_| rng.random::<u64>()).collect();
    let abandon = crate::abandon::current();
    let parts: Vec<(Option<f64>, usize)> = seeds
        .par_iter()
        .enumerate()
        .map(|(i, seed)| {
            let share = samples / chunks + usize::from(i < samples % chunks);
            if crate::abandon::abandoned(&abandon) {
                return (None, share);
            }
            let e = equity_vs_ranges(hero, board, opponents, share, &mut sv10_rng::rngs::SmallRng::seed_from_u64(*seed));
            (e, share)
        })
        .collect();
    // Only chunks that filled their share are averaged, and their shares have to add up to the whole
    // ask: a chunk that refused leaves a hole no other chunk's answer can stand in for.
    let (sum, weight) = parts.iter().fold((0.0, 0usize), |(sum, w), (e, share)| match e {
        Some(v) => (sum + v * *share as f64, w + share),
        None => (sum, w),
    });
    (samples > 0 && weight == samples).then(|| sum / weight as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv10_cards::range::combos;
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

    /// A range holding exactly one combo: the narrowest an opponent's estimate can be, and the
    /// shape that leaves a draw nothing to accept when several opponents are pinned to it.
    fn pinned(x: &str, y: &str) -> Range {
        let (x, y) = (Card::parse(x).unwrap(), Card::parse(y).unwrap());
        let mut r = Range::empty();
        for (i, &(a, b)) in combos().cards.iter().enumerate() {
            if (a, b) == (x, y) || (a, b) == (y, x) {
                r.w[i] = 1.0;
            }
        }
        r
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
        let (equity, scored) = equity_vs_ranges_counted(hero, board, &refs, samples, &mut sv10_rng::rngs::SmallRng::seed_from_u64(7));
        assert_eq!(scored, samples, "a nine-handed draw scored {scored} of {samples} deals");
        assert!(equity.is_some(), "a draw that filled its budget answered {equity:?}");
    }

    /// Twenty opponents pinned to one combo between them: every deal collides with the first
    /// opponent and the draw accepts nothing at all. That used to answer `0.0`, which reads as
    /// "hero never wins" where the truth is that nothing was measured (#424).
    #[test]
    fn a_draw_that_scores_nothing_refuses_instead_of_answering_zero() {
        let hero = [Card::parse("2c").unwrap(), Card::parse("7d").unwrap()];
        let opp = pinned("As", "Ks");
        let refs: Vec<&Range> = (0..20).map(|_| &opp).collect();
        let (equity, scored) = equity_vs_ranges_counted(hero, &[], &refs, 200, &mut sv10_rng::rngs::SmallRng::seed_from_u64(3));
        assert_eq!(scored, 0, "twenty opponents on one combo scored {scored} deals");
        assert_eq!(equity, None, "a draw that scored nothing answered {equity:?}");
    }

    /// The tail of #424's table: fourteen opponents on sparse ranges push the rejection sampler far
    /// enough that it accepts a fraction of the deals it was asked for. The count is the whole
    /// reason the answer is refused — 2,500 samples asked for, seventeen scored, and the old answer
    /// averaged those seventeen as if they were all of them.
    #[test]
    fn a_draw_that_cannot_fill_its_budget_refuses() {
        let samples = 2_500;
        let mut meta = sv10_rng::rngs::SmallRng::seed_from_u64(12_644);
        let dealt = cards(&mut meta, 5);
        let (hero, board) = ([dealt[0], dealt[1]], &dealt[2..]);
        let ranges: Vec<Range> = (0..14).map(|_| sparse(&mut meta, 0.02)).collect();
        let refs: Vec<&Range> = ranges.iter().collect();
        let (equity, scored) = equity_vs_ranges_counted(hero, board, &refs, samples, &mut sv10_rng::rngs::SmallRng::seed_from_u64(7));
        assert!(scored < samples, "this harness has to stay short of the budget to test the refusal; it scored {scored}");
        assert_eq!(equity, None, "a short draw answered {equity:?} from {scored} of {samples} deals");
    }

    #[test]
    fn a_six_card_board_is_refused_instead_of_wrapping_the_deal_count() {
        // A board of six cards is malformed. Its deal count wrapped in release, and the sampler then
        // dealt forever once the deck was spent, holding a decision permit (#944).
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let hero = [Card(0), Card(1)];
            let board: Vec<Card> = (4..10).map(Card).collect();
            let mut rng = sv10_rng::rngs::SmallRng::seed_from_u64(7);
            let _ = tx.send(equity_vs_ranges(hero, &board, &[&Range::full()], 200, &mut rng));
        });
        let out = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("a six-card board hung the sampler");
        assert_eq!(out, None);
    }
}

//! Combo sampler and Monte Carlo card dealing (0261).

use sv10_cards::cards::{Card, CardMask};
use sv10_cards::range::{NUM_COMBOS, Range, combo_mask};
use sv10_rng::{Rng, RngExt};

/// Attempts a rejection-sampled deal may take per sample before it gives up (#383).
///
/// A deal is rejected when two opponents' independently drawn combos share a card, so the acceptance
/// rate falls as opponents are added. Measured (#388), seven opponents or fewer fill every request
/// well inside 20 attempts per sample, eight with a narrow range returned 1510 of 2500 at 20, and ten
/// came back short in every configuration tried. The budget is only spent where a request cannot be
/// filled: a configuration that never exhausts 20 draws exactly the same deals at 200, so this moves
/// no result a six-max table produces.
pub(super) const ATTEMPTS_PER_SAMPLE: usize = 200;

/// Cumulative sampler over a range with fixed dead cards already removed.
pub struct ComboSampler {
    /// Cumulative weights and their combo indices, kept apart so the search scans plain `f64`s.
    cum: Vec<f64>,
    idx: Vec<u16>,
    total: f64,
    /// `guide[k]`: first position whose cumulative weight exceeds `k / GUIDE * total`, so a draw
    /// searches a few entries instead of the whole CDF (same result as a full binary search).
    guide: [u16; GUIDE + 1],
}

const GUIDE: usize = 1024;
/// Fixed search width (0335): `cum` carries this many `+inf` entries past its end, so a draw can
/// always count exactly `WINDOW` entries — two AVX2 compares and a popcount, no loop and no
/// data-dependent branch — whenever the guided window is no wider.
const WINDOW: usize = 8;

impl ComboSampler {
    /// Sampler over `range` with every combo touching `dead` removed.
    pub fn new(range: &Range, dead: CardMask) -> ComboSampler {
        let mut cum = Vec::with_capacity(NUM_COMBOS);
        let mut idx = Vec::with_capacity(NUM_COMBOS);
        let mut acc = 0.0;
        for i in 0..NUM_COMBOS {
            let w = range.w[i] as f64;
            if w > 0.0 && combo_mask(i) & dead == 0 {
                acc += w;
                cum.push(acc);
                idx.push(i as u16);
            }
        }
        let mut guide = [0u16; GUIDE + 1];
        let mut p = 0usize;
        for (k, g) in guide.iter_mut().enumerate() {
            let threshold = k as f64 / GUIDE as f64 * acc;
            while p < cum.len() && cum[p] <= threshold {
                p += 1;
            }
            *g = p as u16;
        }
        cum.resize(cum.len() + WINDOW, f64::INFINITY);
        ComboSampler { cum, idx, total: acc, guide }
    }
    /// Whether no combo has positive weight (callers then fall back to a full range).
    pub fn is_empty(&self) -> bool {
        self.idx.is_empty()
    }
    /// Draw a combo index with probability proportional to its weight (inverse CDF, one uniform draw).
    #[inline]
    pub fn sample<R: Rng>(&self, rng: &mut R) -> usize {
        let u = rng.random::<f64>();
        let x = u * self.total;
        // The bucket comes from `u` (no division); it can differ from `x`'s true bucket only by float
        // rounding, which the one-bucket widening on each side absorbs, so the search result is the
        // same partition point.
        let k = (u * GUIDE as f64) as usize;
        let lo = self.guide[k.saturating_sub(1).min(GUIDE)] as usize;
        let len = self.idx.len();
        let hi = (self.guide[(k + 2).min(GUIDE)] as usize + 1).min(len);
        // Every entry from `hi` on exceeds `x` (the CDF is sorted and `guide[k + 2]`'s entry is
        // past the bucket `x` can fall in), and the padding is `+inf`: counting a fixed window from
        // `lo` gives the same partition point as counting `lo..hi` whenever that window is no wider.
        let below = if hi - lo <= WINDOW {
            let w: &[f64; WINDOW] = self.cum[lo..lo + WINDOW].try_into().expect("padded");
            w.iter().map(|&c| usize::from(c <= x)).sum::<usize>()
        } else {
            self.cum[lo..hi].iter().map(|&c| usize::from(c <= x)).sum::<usize>()
        };
        let pos = (lo + below).min(len - 1);
        self.idx[pos] as usize
    }
}

#[inline]
pub(super) fn deal_random<R: Rng>(rng: &mut R, used: &mut CardMask) -> Card {
    loop {
        let c = rng.random_range(0..52u8);
        let bit = 1u64 << c;
        if *used & bit == 0 {
            *used |= bit;
            return Card(c);
        }
    }
}

#[cfg(test)]
mod sampler_tests {
    use super::*;
    use sv10_cards::range::{NUM_COMBOS, Range};
    use sv10_rng::SeedableRng;

    /// The guided linear search must return exactly what a full binary search over the CDF gives.
    #[test]
    fn guided_search_matches_full_binary_search() {
        let mut rng = sv10_rng::rngs::SmallRng::seed_from_u64(9);
        for trial in 0..40 {
            let mut r = Range::empty();
            for i in 0..NUM_COMBOS {
                // Mix of zero, tiny and heavy weights, like soft top-x ranges.
                let u: f32 = rng.random();
                r.w[i] = if u < 0.3 {
                    0.0
                } else if u < 0.8 {
                    u * 1e-4
                } else {
                    u * (trial as f32 + 1.0)
                };
            }
            let t = trial as usize;
            let dead = Card((t % 52) as u8).bit() | Card(((t * 7 + 3) % 52) as u8).bit();
            let s = ComboSampler::new(&r, dead);
            let mut a = sv10_rng::rngs::SmallRng::seed_from_u64(trial as u64);
            let mut b = a.clone();
            for _ in 0..20_000 {
                let got = s.sample(&mut a);
                let x = b.random::<f64>() * s.total;
                let pos = s.cum[..s.idx.len()].partition_point(|&c| c <= x).min(s.idx.len() - 1);
                assert_eq!(got, s.idx[pos] as usize);
            }
        }
    }
}

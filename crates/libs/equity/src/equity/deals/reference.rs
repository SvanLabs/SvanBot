//! The shared-deals code as it stood before the samples/sec work (0335), frozen as the reference
//! every optimization must reproduce bit for bit: same deals, same ranks, same equities.

use super::super::sampler::{ComboSampler, deal_random};
use sv10_cards::cards::{Card, CardMask};
use sv10_cards::eval::eval;
use sv10_cards::range::{NUM_COMBOS, Range, combo_mask};
use sv10_rng::{Rng, RngExt};

pub(super) struct Reference {
    pub k: usize,
    pub combos: Vec<u16>,
    pub hero_rank: Vec<u32>,
    pub opp_rank: Vec<u32>,
    pub full: Vec<Vec<f32>>,
    pub prior: Vec<f32>,
}
type ComboDeals = (Vec<u16>, Vec<u32>, Vec<u32>, Vec<f32>);

fn completions(board_len: usize) -> usize {
    match board_len {
        5 => 1,
        4 => 44,
        3 => 990,
        _ => usize::MAX,
    }
}

impl Reference {
    fn exact(hero: [Card; 2], board: &[Card], opponents: &[&Range], samples: usize, chunks: usize) -> Option<Reference> {
        if opponents.len() != 1 || board.len() < 3 || board.len() > 5 {
            return None;
        }
        let hero_mask = hero[0].bit() | hero[1].bit();
        let board_mask = board.iter().fold(0u64, |m, c| m | c.bit());
        let dead = hero_mask | board_mask;
        let fallback = Range::full();
        let range =
            if opponents[0].w.iter().enumerate().any(|(i, w)| *w > 0.0 && combo_mask(i) & dead == 0) { opponents[0] } else { &fallback };
        let live: Vec<usize> = (0..NUM_COMBOS).filter(|&i| range.w[i] > 0.0 && combo_mask(i) & dead == 0).collect();
        if live.len().saturating_mul(completions(board.len())) > samples {
            return None;
        }
        let rest: Vec<CardMask> = (0..52u8).map(|c| Card(c).bit()).filter(|b| dead & b == 0).collect();
        // Every deal of one combo: its board completions in a fixed order.
        let deal = |c: usize| -> ComboDeals {
            let m = combo_mask(c);
            let free: Vec<CardMask> = rest.iter().copied().filter(|b| m & b == 0).collect();
            let mut extra = Vec::with_capacity(completions(board.len()));
            match board.len() {
                5 => extra.push(0),
                4 => extra.extend(free.iter().copied()),
                _ => {
                    for x in 0..free.len() {
                        for y in x + 1..free.len() {
                            extra.push(free[x] | free[y]);
                        }
                    }
                }
            }
            let n = extra.len();
            let hero_rank = extra.iter().map(|e| eval(hero_mask | board_mask | e)).collect();
            let opp_rank = extra.iter().map(|e| eval(m | board_mask | e)).collect();
            (vec![c as u16; n], hero_rank, opp_rank, vec![range.w[c]; n])
        };
        let parts: Vec<ComboDeals> = if chunks > 1 {
            use rayon::prelude::*;
            live.par_iter().map(|&c| deal(c)).collect()
        } else {
            live.iter().map(|&c| deal(c)).collect()
        };
        let mut out = Reference {
            k: 1,
            combos: Vec::new(),
            hero_rank: Vec::new(),
            opp_rank: Vec::new(),
            full: vec![range.w.to_vec()],
            prior: Vec::new(),
        };
        for (combos, hero_rank, opp_rank, prior) in parts {
            out.combos.extend(combos);
            out.hero_rank.extend(hero_rank);
            out.opp_rank.extend(opp_rank);
            out.prior.extend(prior);
        }
        Some(out)
    }
    pub fn is_exact(&self) -> bool {
        !self.prior.is_empty()
    }
    pub fn new<R: Rng>(hero: [Card; 2], board: &[Card], opponents: &[&Range], samples: usize, rng: &mut R) -> Reference {
        if let Some(exact) = Reference::exact(hero, board, opponents, samples, 1) {
            return exact;
        }
        let hero_mask = hero[0].bit() | hero[1].bit();
        let board_mask = board.iter().fold(0u64, |m, c| m | c.bit());
        let dead = hero_mask | board_mask;
        let fallback = Range::full();
        let mut ranges: Vec<&Range> = Vec::with_capacity(opponents.len());
        let mut samplers: Vec<ComboSampler> = Vec::with_capacity(opponents.len());
        for r in opponents {
            let s = ComboSampler::new(r, dead);
            if s.is_empty() {
                ranges.push(&fallback);
                samplers.push(ComboSampler::new(&fallback, dead));
            } else {
                ranges.push(*r);
                samplers.push(s);
            }
        }
        let k = samplers.len();
        let missing = 5 - board.len().min(5);
        let mut combos = Vec::with_capacity(samples * k);
        let mut hero_rank = Vec::with_capacity(samples);
        let mut opp_rank = Vec::with_capacity(samples * k);
        let mut dealt = vec![0u16; k];
        let mut attempts = 0usize;
        while hero_rank.len() < samples && attempts < samples * 20 {
            attempts += 1;
            let mut used = dead;
            let mut ok = true;
            for (j, s) in samplers.iter().enumerate() {
                let c = s.sample(rng);
                let m = combo_mask(c);
                if m & used != 0 {
                    ok = false;
                    break;
                }
                used |= m;
                dealt[j] = c as u16;
            }
            if !ok {
                continue;
            }
            let mut full_board = board_mask;
            for _ in 0..missing {
                full_board |= deal_random(rng, &mut used).bit();
            }
            hero_rank.push(eval(hero_mask | full_board));
            for &c in &dealt {
                combos.push(c);
                opp_rank.push(eval(combo_mask(c as usize) | full_board));
            }
        }
        Reference { k, combos, hero_rank, opp_rank, full: ranges.iter().map(|r| r.w.to_vec()).collect(), prior: Vec::new() }
    }
    pub fn new_parallel<R: Rng>(
        hero: [Card; 2],
        board: &[Card],
        opponents: &[&Range],
        samples: usize,
        chunks: usize,
        rng: &mut R,
    ) -> Reference {
        use rayon::prelude::*;
        use sv10_rng::SeedableRng;
        if chunks <= 1 {
            return Reference::new(hero, board, opponents, samples, rng);
        }
        if let Some(exact) = Reference::exact(hero, board, opponents, samples, chunks) {
            return exact;
        }
        let seeds: Vec<u64> = (0..chunks).map(|_| rng.random::<u64>()).collect();
        let parts: Vec<Reference> = seeds
            .par_iter()
            .enumerate()
            .map(|(i, seed)| {
                let share = samples / chunks + usize::from(i < samples % chunks);
                Reference::new(hero, board, opponents, share, &mut sv10_rng::rngs::SmallRng::seed_from_u64(*seed))
            })
            .collect();
        let mut parts = parts.into_iter();
        let mut all = parts.next().expect("at least two chunks");
        for part in parts {
            all.combos.extend(part.combos);
            all.hero_rank.extend(part.hero_rank);
            all.opp_rank.extend(part.opp_rank);
        }
        all
    }
    pub fn len(&self) -> usize {
        self.hero_rank.len()
    }
    pub fn equity(&self, subset: &[(usize, Option<&Range>)]) -> Option<f64> {
        let (mut num, mut den, mut den_sq) = (0.0f64, 0.0f64, 0.0f64);
        let exact = self.is_exact();
        // Each narrowed opponent's reweighting factor per combo, computed once per call instead of
        // once per deal (0230): the same f32 quotient and cast, so every product is bit-identical.
        let ratios: Vec<Option<Vec<f64>>> = subset
            .iter()
            .map(|&(j, narrowed)| {
                narrowed.map(|r| {
                    let full = &self.full[j];
                    (0..NUM_COMBOS).map(|c| if full[c] > 0.0 { (r.w[c] / full[c]) as f64 } else { 0.0 }).collect()
                })
            })
            .collect();
        for d in 0..self.len() {
            let mut w = if exact { self.prior[d] as f64 } else { 1.0f64 };
            let mut best = 0u32;
            let mut at_best = 0u32;
            for (&(j, _), ratio) in subset.iter().zip(&ratios) {
                let c = self.combos[d * self.k + j] as usize;
                if let Some(ratio) = ratio {
                    w *= ratio[c];
                }
                let v = self.opp_rank[d * self.k + j];
                if v > best {
                    best = v;
                    at_best = 1;
                } else if v == best {
                    at_best += 1;
                }
            }
            if w <= 0.0 {
                continue;
            }
            let hv = self.hero_rank[d];
            let share = if hv > best {
                1.0
            } else if hv == best {
                1.0 / (at_best as f64 + 1.0)
            } else {
                0.0
            };
            num += w * share;
            den += w;
            den_sq += w * w;
        }
        // Exact deals need no effective-sample-size guard: any weight at all is the exact answer.
        if den <= 0.0 || (!exact && den * den / den_sq.max(1e-300) < 50.0) {
            return None;
        }
        Some(num / den)
    }
}

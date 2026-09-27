//! Shared exact/Monte Carlo deals across candidates (0261).

use super::sampler::{ComboSampler, deal_random};
use sv10_cards::cards::Card;
use sv10_cards::eval::eval;
use sv10_cards::range::{NUM_COMBOS, Range, combo_mask};
use sv10_rng::{Rng, RngExt};

/// One set of Monte Carlo deals against several opponent ranges, evaluated once and reused.
///
/// Every candidate action in a decision needs hero's equity against a *different* narrowed
/// version of the same ranges (the part that continues). Instead of re-sampling for each, deals
/// are drawn once from the full ranges and each query reweights them by importance ratios
/// `narrowed weight / full weight` of the dealt combos. That costs a few lookups per deal instead
/// of fresh hand evaluations, and compares every candidate on identical deals (common random
/// numbers), which removes sampling noise from the differences between candidates.
pub struct SharedDeals {
    // Fields are crate-visible inside `equity` for the white-box determinism tests in
    // `strengths` (same deals, different chunk counts, must agree); opaque everywhere else.
    pub(super) k: usize,
    /// Dealt combo index per deal and opponent (`n * k`).
    pub(super) combos: Vec<u16>,
    pub(super) hero_rank: Vec<u32>,
    /// Opponent hand ranks per deal (`n * k`).
    pub(super) opp_rank: Vec<u32>,
    /// Full-range weight of each opponent's combos (the sampling distribution).
    pub(super) full: Vec<Vec<f32>>,
    /// Exact deals only (0165): each deal's probability weight, the opponent combo's full-range
    /// weight. Empty for Monte Carlo deals, which were drawn in proportion to it.
    pub(super) prior: Vec<f32>,
}

/// Board completions for a flop (C(45, 2) = 990 once hero's and one opponent's cards are out),
/// turn (44) or river (1), per the number of board cards.
fn completions(board_len: usize) -> usize {
    match board_len {
        5 => 1,
        4 => 44,
        3 => 990,
        _ => usize::MAX,
    }
}

impl SharedDeals {
    /// Heads-up with a flop or later, when every opponent combo times every board completion fits
    /// in `samples`: all of them, each weighted by its combo's full-range weight, so equity is
    /// exact (no sampling noise) and costs no more than the Monte Carlo budget. `None` otherwise.
    fn exact(hero: [Card; 2], board: &[Card], opponents: &[&Range], samples: usize, chunks: usize) -> Option<SharedDeals> {
        if opponents.len() != 1 || board.len() < 3 || board.len() > 5 {
            return None;
        }
        let hero_mask = hero[0].bit() | hero[1].bit();
        let board_mask = board.iter().fold(0u64, |m, c| m | c.bit());
        let dead = hero_mask | board_mask;
        let fallback = Range::full();
        let range =
            if opponents[0].w.iter().enumerate().any(|(i, w)| *w > 0.0 && combo_mask(i) & dead == 0) { opponents[0] } else { &fallback };
        let takes = |i: usize| range.w[i] > 0.0 && combo_mask(i) & dead == 0;
        // Count before collecting (0335): the learner's small budgets fail this check on almost every
        // flop and turn, and used to build the combo list first.
        let n = completions(board.len());
        if (0..NUM_COMBOS).filter(|&i| takes(i)).count().saturating_mul(n) > samples {
            return None;
        }
        let live: Vec<usize> = (0..NUM_COMBOS).filter(|&i| takes(i)).collect();
        let rest: Vec<u8> = (0..52u8).filter(|&c| dead & Card(c).bit() == 0).collect();
        // Hero's rank depends only on the board completion, not on the opponent's combo (0335): one
        // evaluation per completion instead of one per (combo, completion) — 1,081 instead of up to
        // 990,000 on a flop. Indexed by card (turn) or by card pair (flop); the same values.
        let base = hero_mask | board_mask;
        let mut hero_by = vec![0u32; if board.len() == 3 { 52 * 52 } else { 52 }];
        match board.len() {
            5 => hero_by[0] = eval(base),
            4 => rest.iter().for_each(|&c| hero_by[c as usize] = eval(base | Card(c).bit())),
            _ => {
                for (i, &x) in rest.iter().enumerate() {
                    for &y in &rest[i + 1..] {
                        hero_by[x as usize * 52 + y as usize] = eval(base | Card(x).bit() | Card(y).bit());
                    }
                }
            }
        }
        // Every deal of one combo, in a fixed order, written into its own block of the output.
        let fill = |c: usize, combos: &mut [u16], hero_rank: &mut [u32], opp_rank: &mut [u32], prior: &mut [f32]| {
            let m = combo_mask(c);
            let mut free = [0u8; 52];
            let mut nf = 0;
            for &x in &rest {
                if m & Card(x).bit() == 0 {
                    free[nf] = x;
                    nf += 1;
                }
            }
            let free = &free[..nf];
            combos.fill(c as u16);
            prior.fill(range.w[c]);
            let own = m | board_mask;
            match board.len() {
                5 => {
                    hero_rank[0] = hero_by[0];
                    opp_rank[0] = eval(own);
                }
                4 => {
                    for (t, &x) in free.iter().enumerate() {
                        hero_rank[t] = hero_by[x as usize];
                        opp_rank[t] = eval(own | Card(x).bit());
                    }
                }
                _ => {
                    let mut t = 0;
                    for (i, &x) in free.iter().enumerate() {
                        for &y in &free[i + 1..] {
                            hero_rank[t] = hero_by[x as usize * 52 + y as usize];
                            opp_rank[t] = eval(own | Card(x).bit() | Card(y).bit());
                            t += 1;
                        }
                    }
                }
            }
        };
        let total = live.len() * n;
        let mut out = SharedDeals {
            k: 1,
            combos: vec![0; total],
            hero_rank: vec![0; total],
            opp_rank: vec![0; total],
            full: vec![range.w.to_vec()],
            prior: vec![0.0; total],
        };
        if chunks > 1 {
            use rayon::prelude::*;
            out.combos
                .par_chunks_mut(n)
                .zip(out.hero_rank.par_chunks_mut(n))
                .zip(out.opp_rank.par_chunks_mut(n))
                .zip(out.prior.par_chunks_mut(n))
                .zip(live.par_iter())
                .for_each(|((((c, h), o), p), &combo)| fill(combo, c, h, o, p));
        } else {
            for (i, &combo) in live.iter().enumerate() {
                let r = i * n..(i + 1) * n;
                fill(combo, &mut out.combos[r.clone()], &mut out.hero_rank[r.clone()], &mut out.opp_rank[r.clone()], &mut out.prior[r]);
            }
        }
        Some(out)
    }

    /// Whether these deals are the exact enumeration rather than Monte Carlo.
    pub fn is_exact(&self) -> bool {
        !self.prior.is_empty()
    }

    /// Draw `samples` deals of the opponents' combos from `opponents` (empty ranges become full) and
    /// the rest of the board, rejecting card collisions, and rank every hand once.
    pub fn new<R: Rng>(hero: [Card; 2], board: &[Card], opponents: &[&Range], samples: usize, rng: &mut R) -> SharedDeals {
        if let Some(exact) = SharedDeals::exact(hero, board, opponents, samples, 1) {
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
        // Ranks already computed for this board (0335): hero's rank depends only on the missing
        // board cards, so on the river it is one evaluation, on the turn one per river card and on
        // the flop one per turn-and-river pair; on the river an opponent's rank depends only on its
        // combo. `u32::MAX` marks "not yet evaluated" (no hand evaluates to it). Same values, fewer
        // evaluations; the random stream is untouched.
        const UNSEEN: u32 = u32::MAX;
        let mut hero_memo = vec![
            UNSEEN;
            match missing {
                0 => 1,
                1 => 52,
                2 => 52 * 52,
                _ => 0,
            }
        ];
        let mut opp_memo = vec![UNSEEN; if missing == 0 { NUM_COMBOS } else { 0 }];
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
            let mut key = 0usize;
            for _ in 0..missing {
                let card = deal_random(rng, &mut used);
                full_board |= card.bit();
                key = key * 52 + card.0 as usize;
            }
            let hv = match hero_memo.get_mut(key) {
                Some(v) if missing <= 2 => {
                    if *v == UNSEEN {
                        *v = eval(hero_mask | full_board);
                    }
                    *v
                }
                _ => eval(hero_mask | full_board),
            };
            hero_rank.push(hv);
            for &c in &dealt {
                combos.push(c);
                let v = match opp_memo.get_mut(c as usize) {
                    Some(v) => {
                        if *v == UNSEEN {
                            *v = eval(combo_mask(c as usize) | full_board);
                        }
                        *v
                    }
                    None => eval(combo_mask(c as usize) | full_board),
                };
                opp_rank.push(v);
            }
        }
        SharedDeals { k, combos, hero_rank, opp_rank, full: ranges.iter().map(|r| r.w.to_vec()).collect(), prior: Vec::new() }
    }

    /// [`SharedDeals::new`] split over `chunks` parallel jobs: one seed per chunk is drawn from `rng`, each
    /// chunk deals its share with its own stream, and the deals are joined in chunk order, so the
    /// result depends only on `rng` and `chunks`, never on thread scheduling. `chunks <= 1` is `new`.
    pub fn new_parallel<R: Rng>(
        hero: [Card; 2],
        board: &[Card],
        opponents: &[&Range],
        samples: usize,
        chunks: usize,
        rng: &mut R,
    ) -> SharedDeals {
        use rayon::prelude::*;
        use sv10_rng::SeedableRng;
        if chunks <= 1 {
            return SharedDeals::new(hero, board, opponents, samples, rng);
        }
        if let Some(exact) = SharedDeals::exact(hero, board, opponents, samples, chunks) {
            return exact;
        }
        let seeds: Vec<u64> = (0..chunks).map(|_| rng.random::<u64>()).collect();
        let parts: Vec<SharedDeals> = seeds
            .par_iter()
            .enumerate()
            .map(|(i, seed)| {
                let share = samples / chunks + usize::from(i < samples % chunks);
                SharedDeals::new(hero, board, opponents, share, &mut sv10_rng::rngs::SmallRng::seed_from_u64(*seed))
            })
            .collect();
        let total: usize = parts.iter().map(SharedDeals::len).sum();
        let mut parts = parts.into_iter();
        let mut all = parts.next().expect("at least two chunks");
        // Grow once to the joined size instead of doubling through every chunk (0335).
        all.combos.reserve_exact(total * all.k - all.combos.len());
        all.hero_rank.reserve_exact(total - all.hero_rank.len());
        all.opp_rank.reserve_exact(total * all.k - all.opp_rank.len());
        for part in parts {
            all.combos.extend(part.combos);
            all.hero_rank.extend(part.hero_rank);
            all.opp_rank.extend(part.opp_rank);
        }
        all
    }

    /// Number of accepted deals.
    pub fn len(&self) -> usize {
        self.hero_rank.len()
    }

    /// Whether no deal was accepted.
    pub fn is_empty(&self) -> bool {
        self.hero_rank.is_empty()
    }

    /// Hero's pot share against the opponents in `subset` (indices into the construction
    /// order), each optionally narrowed to a sub-range of its full range. Returns `None` when
    /// the narrowed ranges carry too little weight in the deals to estimate (effective sample
    /// size under 50).
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
        // Hero's share on a tie with `n` opponents, the same quotient the loop used to compute per deal.
        let tie: Vec<f64> = (0..=subset.len()).map(|n| 1.0 / (n as f64 + 1.0)).collect();
        let k = self.k;
        for (d, (combos, ranks)) in self.combos.chunks_exact(k).zip(self.opp_rank.chunks_exact(k)).enumerate() {
            let mut w = if exact { self.prior[d] as f64 } else { 1.0f64 };
            let mut best = 0u32;
            let mut at_best = 0u32;
            for (&(j, _), ratio) in subset.iter().zip(&ratios) {
                let c = combos[j] as usize;
                if let Some(ratio) = ratio {
                    w *= ratio[c];
                }
                let v = ranks[j];
                at_best = if v > best { 1 } else { at_best + u32::from(v == best) };
                best = best.max(v);
            }
            // Branch-free (0335): whether hero wins, ties or loses is a coin flip for the branch
            // predictor, and so is a zero weight under a narrowed range. A zero weight adds exact
            // zeros (the sums only ever hold non-negative values), so the totals are the ones the
            // skipping loop produced; a NaN weight still propagates as before.
            let w = if w <= 0.0 { 0.0 } else { w };
            let hv = self.hero_rank[d];
            let share = [0.0, tie[at_best as usize], 1.0][usize::from(hv >= best) + usize::from(hv > best)];
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

#[cfg(test)]
mod reference;
#[cfg(test)]
mod tests;

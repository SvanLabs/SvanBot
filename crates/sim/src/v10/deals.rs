//! v10.0.2's shared deals: one set of deals against every opponent's range, kept with each
//! seat's hand value, so the decision prices every candidate on the same cards. An equity against
//! a narrowed range reweights the deals by the narrowed weight over the full one.
//!
//! With one opponent and few enough combos, the deals are every completion of the board for every
//! combo instead, weighted by the range: exact, with no draw at all.

use svanbot_core::rng::Rng;

use super::cards::{Card, NUM_COMBOS, Range, combo_mask};
use super::draw;
use super::equity::{ATTEMPTS_PER_SAMPLE, ComboSampler, deal_random};
use super::eval::eval;

/// The deals, `k` opponent combos to each.
pub struct SharedDeals {
    k: usize,
    combos: Vec<u16>,
    hero_rank: Vec<u32>,
    opp_rank: Vec<u32>,
    /// Each opponent's full range, which a narrowed one is divided by.
    full: Vec<Vec<f32>>,
    /// Each exact deal's weight; empty for sampled deals.
    prior: Vec<f32>,
}

/// The board completions one combo has on a board of `board_len` cards.
fn completions(board_len: usize) -> usize {
    match board_len {
        5 => 1,
        4 => 44,
        3 => 990,
        _ => usize::MAX,
    }
}

impl SharedDeals {
    /// Every completion for every combo of the one opponent's range, if that is no more than
    /// `samples` deals.
    fn exact(
        hero: [Card; 2],
        board: &[Card],
        opponents: &[&Range],
        samples: usize,
    ) -> Option<Self> {
        if opponents.len() != 1 || !(3..=5).contains(&board.len()) {
            return None;
        }
        let hero_mask = hero[0].bit() | hero[1].bit();
        let board_mask = board.iter().fold(0, |mask, card| mask | card.bit());
        let dead = hero_mask | board_mask;
        let fallback = Range::full();
        let live = |range: &Range, i: usize| range.w[i] > 0.0 && combo_mask(i) & dead == 0;
        let range = if (0..NUM_COMBOS).any(|i| live(opponents[0], i)) {
            opponents[0]
        } else {
            &fallback
        };
        let n = completions(board.len());
        let live: Vec<usize> = (0..NUM_COMBOS).filter(|&i| live(range, i)).collect();
        if live.len().saturating_mul(n) > samples {
            return None;
        }
        let rest: Vec<u8> = (0..52).filter(|&c| dead & Card(c).bit() == 0).collect();
        let base = hero_mask | board_mask;
        let mut hero_by = vec![0; if board.len() == 3 { 52 * 52 } else { 52 }];
        match board.len() {
            5 => hero_by[0] = eval(base),
            4 => {
                for &c in &rest {
                    hero_by[usize::from(c)] = eval(base | Card(c).bit());
                }
            }
            _ => {
                for (i, &x) in rest.iter().enumerate() {
                    for &y in &rest[i + 1..] {
                        hero_by[usize::from(x) * 52 + usize::from(y)] =
                            eval(base | Card(x).bit() | Card(y).bit());
                    }
                }
            }
        }
        let total = live.len() * n;
        let mut out = Self {
            k: 1,
            combos: Vec::with_capacity(total),
            hero_rank: Vec::with_capacity(total),
            opp_rank: Vec::with_capacity(total),
            full: vec![range.w.to_vec()],
            prior: Vec::with_capacity(total),
        };
        for &combo in &live {
            let mask = combo_mask(combo);
            let free: Vec<u8> = rest
                .iter()
                .copied()
                .filter(|&x| mask & Card(x).bit() == 0)
                .collect();
            let own = mask | board_mask;
            let mut push = |hero: u32, opp: u32| {
                out.combos.push(combo as u16);
                out.prior.push(range.w[combo]);
                out.hero_rank.push(hero);
                out.opp_rank.push(opp);
            };
            match board.len() {
                5 => push(hero_by[0], eval(own)),
                4 => {
                    for &x in &free {
                        push(hero_by[usize::from(x)], eval(own | Card(x).bit()));
                    }
                }
                _ => {
                    for (i, &x) in free.iter().enumerate() {
                        for &y in &free[i + 1..] {
                            let hero = hero_by[usize::from(x) * 52 + usize::from(y)];
                            push(hero, eval(own | Card(x).bit() | Card(y).bit()));
                        }
                    }
                }
            }
        }
        Some(out)
    }

    /// Whether these are the exact deals.
    pub fn is_exact(&self) -> bool {
        !self.prior.is_empty()
    }

    /// `samples` deals against `opponents`, or the exact deals when they are few enough.
    pub fn new(
        hero: [Card; 2],
        board: &[Card],
        opponents: &[&Range],
        samples: usize,
        rng: &mut Rng,
    ) -> Self {
        if let Some(exact) = Self::exact(hero, board, opponents, samples) {
            return exact;
        }
        let hero_mask = hero[0].bit() | hero[1].bit();
        let board_mask = board.iter().fold(0, |mask, card| mask | card.bit());
        let dead = hero_mask | board_mask;
        let fallback = Range::full();
        let mut ranges: Vec<&Range> = Vec::with_capacity(opponents.len());
        let mut samplers = Vec::with_capacity(opponents.len());
        for &range in opponents {
            let sampler = ComboSampler::new(range, dead);
            if sampler.is_empty() {
                ranges.push(&fallback);
                samplers.push(ComboSampler::new(&fallback, dead));
            } else {
                ranges.push(range);
                samplers.push(sampler);
            }
        }
        let k = samplers.len();
        let missing = 5 - board.len().min(5);
        let mut out = Self {
            k,
            combos: Vec::with_capacity(samples * k),
            hero_rank: Vec::with_capacity(samples),
            opp_rank: Vec::with_capacity(samples * k),
            full: ranges.iter().map(|range| range.w.to_vec()).collect(),
            prior: Vec::new(),
        };
        let mut dealt = vec![0u16; k];
        let mut attempts = 0;
        'deal: while out.hero_rank.len() < samples && attempts < samples * ATTEMPTS_PER_SAMPLE {
            attempts += 1;
            let mut used = dead;
            for (slot, sampler) in dealt.iter_mut().zip(&samplers) {
                let combo = sampler.sample(rng);
                let mask = combo_mask(combo);
                if mask & used != 0 {
                    continue 'deal;
                }
                used |= mask;
                *slot = combo as u16;
            }
            let mut full_board = board_mask;
            for _ in 0..missing {
                full_board |= deal_random(rng, &mut used).bit();
            }
            out.hero_rank.push(eval(hero_mask | full_board));
            for &combo in &dealt {
                out.combos.push(combo);
                out.opp_rank
                    .push(eval(combo_mask(usize::from(combo)) | full_board));
            }
        }
        out
    }

    /// [`SharedDeals::new`] cut into `chunks`, one seed each drawn from `rng`, joined in chunk
    /// order.
    pub fn new_chunked(
        hero: [Card; 2],
        board: &[Card],
        opponents: &[&Range],
        samples: usize,
        chunks: usize,
        rng: &mut Rng,
    ) -> Self {
        if chunks <= 1 {
            return Self::new(hero, board, opponents, samples, rng);
        }
        if let Some(exact) = Self::exact(hero, board, opponents, samples) {
            return exact;
        }
        let seeds: Vec<u64> = (0..chunks).map(|_| draw::u64(rng)).collect();
        let mut all: Option<Self> = None;
        for (i, &seed) in seeds.iter().enumerate() {
            let share = samples / chunks + usize::from(i < samples % chunks);
            let part = Self::new(hero, board, opponents, share, &mut Rng::seeded(seed));
            match &mut all {
                None => all = Some(part),
                Some(all) => {
                    all.combos.extend(part.combos);
                    all.hero_rank.extend(part.hero_rank);
                    all.opp_rank.extend(part.opp_rank);
                }
            }
        }
        all.expect("at least two chunks")
    }

    /// How many deals there are.
    pub fn len(&self) -> usize {
        self.hero_rank.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.hero_rank.is_empty()
    }

    /// Hero's share against the opponents in `subset`, each by its index and, when narrowed, its
    /// narrowed range. `None` when the deals' weight is too thin to answer: no weight, or fewer
    /// than 50 effective deals from a draw.
    pub fn equity(&self, subset: &[(usize, Option<&Range>)]) -> Option<f64> {
        let exact = self.is_exact();
        let ratios: Vec<Option<Vec<f64>>> = subset
            .iter()
            .map(|&(j, narrowed)| {
                narrowed.map(|range| {
                    let full = &self.full[j];
                    (0..NUM_COMBOS)
                        .map(|c| {
                            if full[c] > 0.0 {
                                f64::from(range.w[c] / full[c])
                            } else {
                                0.0
                            }
                        })
                        .collect()
                })
            })
            .collect();
        let tie: Vec<f64> = (0..=subset.len()).map(|n| 1.0 / (n as f64 + 1.0)).collect();
        let (mut num, mut den, mut den_sq) = (0.0f64, 0.0f64, 0.0f64);
        let deals = self.combos.chunks_exact(self.k);
        for (d, (combos, ranks)) in deals.zip(self.opp_rank.chunks_exact(self.k)).enumerate() {
            let mut w = if exact { f64::from(self.prior[d]) } else { 1.0 };
            let (mut best, mut at_best) = (0u32, 0u32);
            for (&(j, _), ratio) in subset.iter().zip(&ratios) {
                if let Some(ratio) = ratio {
                    w *= ratio[usize::from(combos[j])];
                }
                let value = ranks[j];
                at_best = if value > best {
                    1
                } else {
                    at_best + u32::from(value == best)
                };
                best = best.max(value);
            }
            let w = if w <= 0.0 { 0.0 } else { w };
            let hero = self.hero_rank[d];
            let share = [0.0, tie[at_best as usize], 1.0]
                [usize::from(hero >= best) + usize::from(hero > best)];
            num += w * share;
            den += w;
            den_sq += w * w;
        }
        if den <= 0.0 || (!exact && den * den / den_sq.max(1e-300) < 50.0) {
            return None;
        }
        Some(num / den)
    }
}

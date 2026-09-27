//! Settlement: uncalled refunds, main and side pots, odd chips, and luck-free expected nets.

use super::Hand;
use sv10_cards::cards::Card;
use sv10_cards::eval::eval;
use sv10_rng::{Rng, RngExt};

impl Hand {
    /// Payouts (uncalled refund, then main and side pots) if the board ran out as `runout`.
    pub(super) fn settle(&self, runout: &[Card; 5]) -> Vec<i64> {
        let board_mask = runout.iter().fold(0u64, |m, c| m | c.bit());
        let invested: Vec<i64> = self.seats.iter().map(|s| s.invested).collect();
        let folded: Vec<bool> = self.seats.iter().map(|s| s.folded).collect();
        let values: Vec<u32> = self.seats.iter().map(|s| eval(board_mask | s.hole[0].bit() | s.hole[1].bit())).collect();
        split_pots(&invested, &folded, &values, self.button)
    }

    /// Net chips per seat with the luck of an all-in runout removed: when betting closed before the
    /// river with two or more players left, the net is averaged over every possible runout of the
    /// unseen cards (all dealt hole cards are known here), or over `max_runouts` random runouts
    /// when there are more. Its expectation equals `net()`'s, with far less variance; otherwise it
    /// is `net()`. `max_runouts` is a cap on sampled runouts and zero is not a usable one — an
    /// average over no runouts is not an estimate of anything — so it is clamped to one runout.
    pub fn expected_net<R: Rng>(&self, max_runouts: usize, rng: &mut R) -> Vec<f64> {
        // One runout is the cheapest sample of the expectation there is, so a zero budget buys that
        // sample rather than dividing by zero runouts and returning a NaN for every seat (#6). The
        // clamp belongs on the bound and not on the mean below: guarding the division would turn a
        // zero count into a plausible-looking number rather than making it unreachable.
        let max_runouts = max_runouts.max(1);
        let realized: Vec<f64> = self.net().iter().map(|&x| x as f64).collect();
        let closed_at = self.history.last().map(|r| r.street.board_len()).unwrap_or(0);
        if !self.showdown() || closed_at >= 5 {
            return realized;
        }
        let known = &self.runout[..closed_at];
        let mut dead = known.iter().fold(0u64, |m, c| m | c.bit());
        for s in &self.seats {
            dead |= s.hole[0].bit() | s.hole[1].bit();
        }
        let unseen: Vec<Card> = (0..52u8).map(Card).filter(|c| dead & c.bit() == 0).collect();
        let missing = 5 - closed_at;
        let before: Vec<i64> = self.seats.iter().zip(&self.payouts).map(|(s, p)| s.stack - p - s.start_stack).collect();
        let mut sums = vec![0f64; self.seats.len()];
        let mut count = 0usize;
        let mut runout = self.runout;
        let mut add = |cards: &[Card], sums: &mut Vec<f64>| {
            runout[closed_at..].copy_from_slice(cards);
            for (k, p) in self.settle(&runout).into_iter().enumerate() {
                sums[k] += p as f64;
            }
        };
        let total = binomial(unseen.len(), missing);
        if total <= max_runouts as u64 {
            let mut idx: Vec<usize> = (0..missing).collect();
            let mut cards = vec![Card(0); missing];
            loop {
                for (k, &i) in idx.iter().enumerate() {
                    cards[k] = unseen[i];
                }
                add(&cards, &mut sums);
                count += 1;
                // Next combination in lexicographic order.
                let mut k = missing;
                while k > 0 && idx[k - 1] == unseen.len() - missing + k - 1 {
                    k -= 1;
                }
                if k == 0 {
                    break;
                }
                idx[k - 1] += 1;
                for m in k..missing {
                    idx[m] = idx[m - 1] + 1;
                }
            }
        } else {
            let mut pool = unseen.clone();
            for _ in 0..max_runouts {
                for k in 0..missing {
                    let j = rng.random_range(k..pool.len());
                    pool.swap(k, j);
                }
                add(&pool[..missing], &mut sums);
                count += 1;
            }
        }
        before.iter().zip(sums).map(|(b, s)| *b as f64 + s / count as f64).collect()
    }

    /// Chance correction (AIVAT-style) for `seat`, in chips: for the turn and the river card, when it
    /// was dealt with betting still open and `seat` still in, how much the actual card moved `seat`'s
    /// value against the average unseen card: `pot × (share(actual) − mean share)`, where the pot is
    /// the pot before the street's first action and the share is `seat`'s showdown share of the
    /// players still in (averaged over the river when correcting the turn). Everything but the card
    /// is fixed before the card is dealt, so the correction has expectation exactly zero for any hand
    /// and `expected_net − chance_correction` is an unbiased, lower-variance outcome for evaluation.
    pub fn chance_correction(&self, seat: usize) -> f64 {
        use super::{ActionKind, Street};
        let holes: u64 = self.seats.iter().fold(0, |m, s| m | s.hole[0].bit() | s.hole[1].bit());
        let mut total = 0.0;
        for (k, street) in [(3usize, Street::Turn), (4, Street::River)] {
            let Some(first) = self.history.iter().find(|r| r.street == street) else { continue };
            let folded_before =
                |i: usize| self.history.iter().any(|r| r.seat == i && r.kind == ActionKind::Fold && r.street.index() < street.index());
            let live: Vec<u64> = (0..self.seats.len())
                .filter(|&i| !folded_before(i))
                .map(|i| self.seats[i].hole[0].bit() | self.seats[i].hole[1].bit())
                .collect();
            if folded_before(seat) || live.len() < 2 {
                continue;
            }
            let mine = self.seats[seat].hole[0].bit() | self.seats[seat].hole[1].bit();
            let known = self.runout[..k].iter().fold(0u64, |m, c| m | c.bit());
            let unseen: Vec<u64> = (0..52u8).map(|c| 1u64 << c).filter(|b| (holes | known) & b == 0).collect();
            // Showdown share of `seat` on a complete five-card board.
            let share = |board: u64| -> f64 {
                let values: Vec<u32> = live.iter().map(|h| eval(board | h)).collect();
                let hero = eval(board | mine);
                let best = *values.iter().max().unwrap_or(&0);
                if hero < best { 0.0 } else { 1.0 / values.iter().filter(|&&v| v == best).count() as f64 }
            };
            let value = |card: u64| -> f64 {
                if k == 4 {
                    share(known | card)
                } else {
                    let rivers: Vec<&u64> = unseen.iter().filter(|&&r| r != card).collect();
                    rivers.iter().map(|&&r| share(known | card | r)).sum::<f64>() / rivers.len().max(1) as f64
                }
            };
            let actual = value(self.runout[k].bit());
            let mean = unseen.iter().map(|&c| value(c)).sum::<f64>() / unseen.len().max(1) as f64;
            total += first.pot_before as f64 * (actual - mean);
        }
        total
    }

    /// Net chips won per seat for a finished hand.
    pub fn net(&self) -> Vec<i64> {
        self.seats.iter().map(|s| s.stack - s.start_stack).collect()
    }

    /// Chips each seat received at settlement (pots won plus uncalled refunds); zeros before then.
    pub fn payouts(&self) -> &[i64] {
        &self.payouts
    }

    /// Whether the hand went to showdown (two or more players never folded).
    pub fn showdown(&self) -> bool {
        self.finished && self.seats.iter().filter(|s| !s.folded).count() > 1
    }
}

fn binomial(n: usize, k: usize) -> u64 {
    if k > n {
        return 0;
    }
    (0..k).fold(1u64, |acc, i| acc * (n - i) as u64 / (i + 1) as u64)
}

/// Payouts per seat of a finished pot: the uncalled excess back to the top contributor, then the
/// main and side pots to the best hand `values` (higher is better) among the seats that are not
/// `folded` and reached each level; odd chips go to the first winner left of `button`. Shared by
/// the simulator's settlement and the luck-adjustment of stored hands (`sv10_model::allin`).
///
/// A pot no seat can win is void. With every seat `folded` there is no winner to pay — a state play
/// cannot reach, because the hand ends at one live seat (`advance_if_needed`) — and a void pot goes
/// back to the seats that paid into it: each seat gets exactly its own wagers, so the hand moves no
/// chips and settlement stays zero-sum. Paying nobody instead would destroy the pot.
pub fn split_pots(invested: &[i64], folded: &[bool], values: &[u32], button: usize) -> Vec<i64> {
    let n = invested.len();
    let live: Vec<usize> = (0..n).filter(|&i| !folded[i]).collect();
    if live.is_empty() {
        return invested.to_vec();
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(invested[i]));
    let top = order[0];
    let second = order.get(1).map_or(0, |&i| invested[i]);
    let excess = invested[top] - second;
    let mut invested = invested.to_vec();
    let mut payouts = vec![0i64; n];
    if excess > 0 {
        invested[top] -= excess;
        payouts[top] += excess;
    }
    if live.len() == 1 {
        payouts[live[0]] += invested.iter().sum::<i64>();
        return payouts;
    }
    let mut levels: Vec<i64> = invested.iter().copied().filter(|&x| x > 0).collect();
    levels.sort();
    levels.dedup();
    let mut prev = 0;
    for level in levels {
        let contributors = invested.iter().filter(|&&x| x >= level).count() as i64;
        let slice = (level - prev) * contributors;
        prev = level;
        let eligible: Vec<usize> = live.iter().copied().filter(|&i| invested[i] >= level).collect();
        let pool = if eligible.is_empty() { live.clone() } else { eligible };
        let best = pool.iter().map(|&i| values[i]).max().unwrap();
        let mut winners: Vec<usize> = pool.into_iter().filter(|&i| values[i] == best).collect();
        // Odd chips go to the first winner left of the button.
        winners.sort_by_key(|&i| (i + n - button - 1) % n);
        let share = slice / winners.len() as i64;
        let mut rem = slice - share * winners.len() as i64;
        for &w in &winners {
            payouts[w] += share + if rem > 0 { 1 } else { 0 };
            rem -= 1;
        }
    }
    payouts
}

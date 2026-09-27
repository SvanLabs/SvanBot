//! All-in luck of a stored hand (0213): when betting closed before the river with two or more
//! players left and every one of their hands was shown, the cards still to come are luck. The
//! all-in EV of a seat is its payout averaged over every possible runout of those cards (the
//! engine's own pot splitting, [`sv10_engine::engine::split_pots`]) minus what it put in; the
//! difference to its actual net is the luck removed (PokerTracker's "all-in adjusted" winnings).
//!
//! The pot is rebuilt from the action history (blinds from each seat's first bet), and a hand is
//! adjusted only when that rebuild reproduces the seat's stored net on the real board, so a
//! summary that misses a contribution is reported as [`AllInLuck::Unverifiable`], never guessed.

use crate::model::HandSummary;
use sv10_cards::cards::Card;
use sv10_cards::eval::eval;
use sv10_engine::engine::{ActionKind, split_pots};
use sv10_rng::rngs::SmallRng;
use sv10_rng::{RngExt, SeedableRng};

/// What all-in luck a hand held for one seat.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AllInLuck {
    /// No all-in runout decided the seat's result (it folded, won uncontested or saw the river
    /// with betting open): its net is its EV net.
    NotAllIn,
    /// Betting closed before the river with every live hand shown: the seat's net averaged over
    /// the runouts, and its actual net.
    Adjusted {
        /// All-in EV net in chips.
        expected: f64,
        /// Actual net in chips.
        actual: i64,
    },
    /// An all-in runout with a hand not shown, a board not dealt out, or a rebuilt pot that does
    /// not reproduce the stored net: left unadjusted.
    Unverifiable,
}

impl AllInLuck {
    /// Chips to add to the actual net to get the all-in EV net (0 unless adjusted).
    pub fn adjustment(self) -> f64 {
        match self {
            AllInLuck::Adjusted { expected, actual } => expected - actual as f64,
            _ => 0.0,
        }
    }
}

/// Chips each seat put in, from the history: per street the most it had bet (its blind is the
/// street bet before its first preflop action).
fn invested(hand: &HandSummary, seat: usize) -> i64 {
    let mut total = 0;
    let mut street = None;
    let mut street_max = 0;
    for r in hand.history.iter().filter(|r| r.seat == seat) {
        if street != Some(r.street) {
            total += street_max;
            street = Some(r.street);
            street_max = 0;
        }
        street_max = street_max.max(r.bet_before).max(r.to);
    }
    total + street_max
}

/// The all-in luck `hero_seat` had in `hand`, given its hole cards (from the hand if shown) and
/// stored `hero_net`. Runouts are enumerated when there are at most `max_runouts`, otherwise
/// `max_runouts` are sampled with `seed`.
pub fn hero_all_in_luck(
    hand: &HandSummary,
    hero_seat: usize,
    hero_hole: Option<[Card; 2]>,
    hero_net: i64,
    max_runouts: usize,
    seed: u64,
) -> AllInLuck {
    let Some(last) = hand.history.last() else { return AllInLuck::NotAllIn };
    let closed_at = last.street.board_len();
    let folded = |seat: usize| hand.history.iter().any(|r| r.seat == seat && r.kind == ActionKind::Fold);
    let mut seats: Vec<usize> = hand.players.iter().map(|(s, _)| *s).collect();
    seats.sort_unstable();
    seats.dedup();
    let live: Vec<usize> = seats.iter().copied().filter(|&s| !folded(s)).collect();
    if closed_at >= 5 || live.len() < 2 || !live.contains(&hero_seat) {
        return AllInLuck::NotAllIn;
    }
    let hole_of =
        |seat: usize| hand.shown.iter().find(|(s, _)| *s == seat).map(|(_, h)| *h).or(if seat == hero_seat { hero_hole } else { None });
    let holes: Vec<Option<[Card; 2]>> = seats.iter().map(|&s| if live.contains(&s) { hole_of(s) } else { None }).collect();
    if hand.board.len() < 5 || seats.iter().zip(&holes).any(|(s, h)| live.contains(s) && h.is_none()) {
        return AllInLuck::Unverifiable;
    }
    let invested: Vec<i64> = seats.iter().map(|&s| invested(hand, s)).collect();
    let folded_flags: Vec<bool> = seats.iter().map(|s| !live.contains(s)).collect();
    // The button's place among the dealt-in seats (odd chips only).
    let button = seats.iter().rposition(|&s| s <= hand.button).unwrap_or(seats.len() - 1);
    let hero = seats.iter().position(|&s| s == hero_seat).expect("hero is live, so dealt in");
    let payout = |board: &[Card]| {
        let mask = board.iter().fold(0u64, |m, c| m | c.bit());
        let values: Vec<u32> = holes.iter().map(|h| h.map_or(0, |h| eval(mask | h[0].bit() | h[1].bit()))).collect();
        split_pots(&invested, &folded_flags, &values, button)[hero]
    };
    if (payout(&hand.board[..5]) - invested[hero] - hero_net).abs() > 1 {
        return AllInLuck::Unverifiable;
    }
    let mut dead = hand.board[..closed_at].iter().fold(0u64, |m, c| m | c.bit());
    for h in holes.iter().flatten() {
        dead |= h[0].bit() | h[1].bit();
    }
    let unseen: Vec<Card> = (0..52u8).map(Card).filter(|c| dead & c.bit() == 0).collect();
    let missing = 5 - closed_at;
    let mut board = [Card(0); 5];
    board[..closed_at].copy_from_slice(&hand.board[..closed_at]);
    let (mut sum, mut count) = (0f64, 0usize);
    if binomial(unseen.len(), missing) <= max_runouts as u64 {
        let mut idx: Vec<usize> = (0..missing).collect();
        loop {
            for (k, &i) in idx.iter().enumerate() {
                board[closed_at + k] = unseen[i];
            }
            sum += payout(&board) as f64;
            count += 1;
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
        let mut rng = SmallRng::seed_from_u64(seed);
        let mut pool = unseen;
        for _ in 0..max_runouts.max(1) {
            for k in 0..missing {
                let j = rng.random_range(k..pool.len());
                pool.swap(k, j);
                board[closed_at + k] = pool[k];
            }
            sum += payout(&board) as f64;
            count += 1;
        }
    }
    AllInLuck::Adjusted { expected: sum / count as f64 - invested[hero] as f64, actual: hero_net }
}

fn binomial(n: usize, k: usize) -> u64 {
    (0..k).fold(1u64, |acc, i| acc * (n - i) as u64 / (i + 1) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv10_engine::engine::{Action, Hand};

    fn summary(h: &Hand) -> HandSummary {
        HandSummary {
            players: (0..h.seats.len()).map(|i| (i, format!("p{i}"))).collect(),
            button: h.button,
            bb: h.bb,
            history: h.history.clone(),
            board: h.board.clone(),
            stacks: h.seats.iter().enumerate().map(|(i, s)| (i, s.start_stack)).collect(),
            shown: if h.showdown() {
                h.seats.iter().enumerate().filter(|(_, s)| !s.folded).map(|(i, s)| (i, s.hole)).collect()
            } else {
                vec![]
            },
        }
    }

    /// Play `actions` from a fresh hand; returns it finished.
    fn play(stacks: &[i64], seed: u64, actions: &[Action]) -> Hand {
        let mut rng = SmallRng::seed_from_u64(seed);
        let mut h = Hand::new(stacks, 0, 10, 20, &mut rng);
        for a in actions {
            h.apply(*a).unwrap();
        }
        while h.actor().is_some() {
            h.apply(if h.legal().can_check { Action::Check } else { Action::Call }).unwrap();
        }
        h
    }

    #[test]
    fn flop_and_turn_all_ins_match_the_engines_exact_expected_net() {
        let mut checked = 0;
        for seed in 0..40 {
            // Three-handed: button raises, blinds call; on the flop (or the turn) the small blind
            // shoves, the big blind shoves over it and the button calls: a main pot and a side pot,
            // betting closed with two cards (or one) to come.
            let stacks = [3_000, 1_500, 2_200];
            let turn = seed % 2 == 1;
            let mut actions = vec![Action::RaiseTo(60), Action::Call, Action::Call];
            if turn {
                actions.extend([Action::Check, Action::Check, Action::Check]);
            }
            actions.extend([Action::AllIn, Action::AllIn, Action::Call]);
            let h = play(&stacks, seed, &actions);
            assert!(h.showdown());
            let mut rng = SmallRng::seed_from_u64(1);
            let exact = h.expected_net(2_000, &mut rng);
            let s = summary(&h);
            for seat in 0..3 {
                let luck = hero_all_in_luck(&s, seat, None, h.net()[seat], 2_000, 9);
                let AllInLuck::Adjusted { expected, actual } = luck else { panic!("seed {seed} seat {seat}: {luck:?}") };
                assert_eq!(actual, h.net()[seat]);
                assert!((expected - exact[seat]).abs() < 1e-6, "seed {seed} seat {seat}: {expected} vs {}", exact[seat]);
                checked += 1;
            }
        }
        assert_eq!(checked, 120);
    }

    #[test]
    fn a_preflop_all_in_is_sampled_close_to_the_engine() {
        let h = play(&[2_000, 2_000], 3, &[Action::AllIn]);
        let s = summary(&h);
        let mut rng = SmallRng::seed_from_u64(2);
        let engine = h.expected_net(20_000, &mut rng);
        let luck = hero_all_in_luck(&s, 0, None, h.net()[0], 20_000, 5);
        let AllInLuck::Adjusted { expected, .. } = luck else { panic!("{luck:?}") };
        // Two independent 20k-runout samples of a 4,000-chip pot agree within a few percent of it.
        assert!((expected - engine[0]).abs() < 120.0, "{expected} vs {}", engine[0]);
        assert!((luck.adjustment() - (expected - h.net()[0] as f64)).abs() < 1e-9);
    }

    #[test]
    fn river_showdowns_folds_and_hidden_hands_are_not_adjusted() {
        // Checked down to the river: no runout luck to remove.
        let h = play(&[2_000, 2_000], 4, &[Action::Call]);
        assert!(h.showdown());
        let s = summary(&h);
        assert_eq!(hero_all_in_luck(&s, 0, None, h.net()[0], 2_000, 1), AllInLuck::NotAllIn);
        // The hero folded to a shove.
        let h = play(&[2_000, 2_000, 2_000], 6, &[Action::AllIn, Action::Fold, Action::Call]);
        let s = summary(&h);
        assert_eq!(hero_all_in_luck(&s, 1, None, h.net()[1], 2_000, 1), AllInLuck::NotAllIn);
        assert!(matches!(hero_all_in_luck(&s, 0, None, h.net()[0], 2_000, 1), AllInLuck::Adjusted { .. }));
        // A live villain's cards not shown: unverifiable, as is a net the rebuilt pot cannot reproduce.
        let mut hidden = s.clone();
        hidden.shown.retain(|(seat, _)| *seat == 0);
        assert_eq!(hero_all_in_luck(&hidden, 0, None, h.net()[0], 2_000, 1), AllInLuck::Unverifiable);
        let hero_hole = s.shown.iter().find(|(seat, _)| *seat == 0).map(|(_, c)| *c);
        let mut only_villain = s.clone();
        only_villain.shown.retain(|(seat, _)| *seat != 0);
        assert!(matches!(hero_all_in_luck(&only_villain, 0, hero_hole, h.net()[0], 2_000, 1), AllInLuck::Adjusted { .. }));
        assert_eq!(hero_all_in_luck(&s, 0, None, h.net()[0] + 500, 2_000, 1), AllInLuck::Unverifiable);
    }
}

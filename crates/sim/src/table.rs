//! The table: a deal drawn from a seed, and one hand played out on it, each seat deciding at the
//! spot it sees. The rules are `core`'s ([`Hand`]); the table deals the cards, shows each seat
//! its spot with the lines of the seats still in, and pays the pots at the end.
//!
//! A decision that is not legal is played as the nearest action that is, as the bot does at the
//! venue: a raise is kept inside the range, and what cannot be played steps down to a call, a
//! check, then a fold.

use svanbot_core::card::{Card, CardSet, Rank, Suit};
use svanbot_core::eval::strength;
use svanbot_core::rng::Rng;
use svanbot_core::rules::{Action, Blinds, Hand, Legal, Street};
use svanbot_core::spot::{Act, ActKind, Spot};

/// The cards of one hand: each seat's hole cards, and the board in the order it comes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Deal {
    pub holes: Vec<CardSet>,
    pub board: [Card; 5],
}

impl Deal {
    /// The community cards out on `street`.
    pub fn shown(&self, street: Street) -> CardSet {
        let out = match street {
            Street::Preflop => 0,
            Street::Flop => 3,
            Street::Turn => 4,
            Street::River => 5,
        };
        self.board[..out].iter().copied().collect()
    }
}

/// Deals `seats` hands and a board from `rng`.
pub fn deal(seats: usize, rng: &mut Rng) -> Deal {
    let mut deck: Vec<Card> = Rank::ALL
        .into_iter()
        .flat_map(|rank| Suit::ALL.map(|suit| Card::new(rank, suit)))
        .collect();
    let mut draw = || deck.swap_remove(rng.below(deck.len() as u64) as usize);
    let holes = (0..seats)
        .map(|_| [draw(), draw()].into_iter().collect())
        .collect();
    let board = [draw(), draw(), draw(), draw(), draw()];
    Deal { holes, board }
}

/// Plays one hand on `deal`, with `stacks` in table order, and returns the chips each seat won
/// or lost. `decide` is asked for the action of a seat at its spot.
pub fn play(
    deal: &Deal,
    stacks: &[u32],
    button: usize,
    blinds: Blinds,
    mut decide: impl FnMut(usize, &Spot) -> Action,
) -> Vec<i64> {
    let seats = stacks.len();
    let mut hand = Hand::new(stacks, button, blinds);
    let mut lines: Vec<Vec<Act>> = vec![Vec::new(); seats];
    let mut committed = vec![0; seats];
    let mut street = hand.street();
    while let (Some(seat), Some(legal)) = (hand.to_act(), hand.legal()) {
        if hand.street() != street {
            street = hand.street();
            committed.fill(0);
        }
        let board = deal.shown(street);
        let folded = hand.folded();
        let pot = hand.pot();
        let spot = Spot {
            hole: deal.holes[seat],
            board,
            pot,
            legal,
            opponents: folded.iter().filter(|&&out| !out).count() - 1,
            stack: hand.stacks()[seat],
            committed: committed[seat],
            big_blind: blinds.big,
            lines: (0..seats)
                .filter(|&other| other != seat && !folded[other])
                .map(|other| lines[other].clone())
                .collect(),
        };
        let action = nearest(decide(seat, &spot), legal);
        let before = hand.committed()[seat];
        if hand.act(action).is_err() {
            break;
        }
        let added = hand.committed()[seat] - before;
        committed[seat] += added;
        lines[seat].push(Act {
            street,
            board,
            // The blinds are a bet before the flop, as the venue's frames have it.
            facing: legal.call.is_some() || street == Street::Preflop,
            kind: match action {
                Action::Fold => ActKind::Fold,
                Action::Check => ActKind::Check,
                Action::Call => ActKind::Call,
                Action::RaiseTo(_) => ActKind::Bet { added, pot },
            },
        });
    }
    let all = deal.shown(Street::River);
    let paid = hand.payouts(|seat| strength(deal.holes[seat] | all));
    paid.iter()
        .zip(hand.committed())
        .map(|(&paid, put_in)| i64::from(paid) - i64::from(put_in))
        .collect()
}

/// The legal action nearest to `wanted`.
fn nearest(wanted: Action, legal: Legal) -> Action {
    let passive = match legal.call {
        Some(_) => Action::Call,
        None => Action::Check,
    };
    match wanted {
        Action::Fold | Action::Check if legal.call.is_none() => Action::Check,
        Action::Fold | Action::Check => Action::Fold,
        Action::Call => passive,
        Action::RaiseTo(to) => match legal.raise {
            Some(range) => Action::RaiseTo(to.clamp(range.min, range.max)),
            None => passive,
        },
    }
}

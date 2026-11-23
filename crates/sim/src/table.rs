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
use svanbot_core::spot::{Act, Spot};

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

/// The whole table at a seat's turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seen {
    pub button: usize,
    /// The community cards out, in the order they came.
    pub board: Vec<Card>,
    /// Every seat, in table order.
    pub seats: Vec<SeatSeen>,
    /// Every action of the hand so far, in order. The blinds are not in it.
    pub record: Vec<Record>,
}

/// One seat as the table shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeatSeen {
    /// The chips behind.
    pub stack: u32,
    /// The chips in on this street, a blind included.
    pub bet: u32,
    pub folded: bool,
}

/// One action, with the table as it stood before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub seat: usize,
    pub street: Street,
    /// The action as it was played: the nearest legal one to what was asked.
    pub action: Action,
    /// The seat's street bet after it; 0 for a fold or a check.
    pub to: u32,
    /// The pot, this street's bets included.
    pub pot_before: u32,
    /// What the seat owed: the street's top bet less its own.
    pub owed_before: u32,
    pub bet_before: u32,
    /// Whether it put the seat's last chip in.
    pub all_in: bool,
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
    let (hand, _, _) = bet(
        deal,
        stacks,
        button,
        blinds,
        |seat, spot, _| decide(seat, spot),
        |_, _| {},
    );
    let all = deal.shown(Street::River);
    let paid = hand.payouts(|seat| strength(deal.holes[seat] | all));
    paid.iter()
        .zip(hand.committed())
        .map(|(&paid, put_in)| i64::from(paid) - i64::from(put_in))
        .collect()
}

/// The most run-outs a seat's expected payout is averaged over. Two cards to come are at most
/// 1,081, so from the flop on every run-out is counted, and before the flop a sample is drawn.
pub const RUNOUTS: usize = 1_500;

/// Plays one hand as [`play`] does, with the luck of the board taken out (the chance-node terms
/// of AIVAT: Burch et al., 2018). Each time cards come, a seat's result is corrected by how far
/// they moved its expected payout, with the chips as they stood then:
///
/// `corrected = won - sum over each street's cards of (expected after - expected before)`
///
/// The expected payout is the average over the run-outs of the board, every one of them or
/// [`RUNOUTS`] drawn from `seed`. The correction has mean zero whatever the seats do, so the
/// corrected result is as fair as the dealt one and swings far less. A hand all in before the
/// river is paid its share of the run-outs.
pub fn play_corrected(
    deal: &Deal,
    stacks: &[u32],
    button: usize,
    blinds: Blinds,
    seed: u64,
    mut decide: impl FnMut(usize, &Spot) -> Action,
) -> Vec<f64> {
    play_seen(deal, stacks, button, blinds, seed, |seat, spot, _| {
        decide(seat, spot)
    })
}

/// [`play_corrected`], with each seat also shown the whole table ([`Seen`]): what an opponent
/// that reads every seat, as v10.0.2 did, decides from. Only the sim can show it.
pub fn play_seen(
    deal: &Deal,
    stacks: &[u32],
    button: usize,
    blinds: Blinds,
    seed: u64,
    decide: impl FnMut(usize, &Spot, &Seen) -> Action,
) -> Vec<f64> {
    play_observed(deal, stacks, button, blinds, seed, decide, |_, _| {})
}

/// Plays a corrected hand and delivers the public betting history after the last action.
pub fn play_observed(
    deal: &Deal,
    stacks: &[u32],
    button: usize,
    blinds: Blinds,
    seed: u64,
    decide: impl FnMut(usize, &Spot, &Seen) -> Action,
    finished: impl FnOnce(&[Vec<Act>], &[Record]),
) -> Vec<f64> {
    let (hand, last, mut stood) = bet(deal, stacks, button, blinds, decide, finished);
    let all = deal.shown(Street::River);
    let paid = hand.payouts(|seat| strength(deal.holes[seat] | all));
    let mut won: Vec<f64> = paid
        .iter()
        .zip(hand.committed())
        .map(|(&paid, put_in)| f64::from(paid) - f64::from(put_in))
        .collect();
    // The cards that came with the betting over are met with the chips as they ended.
    if hand.is_showdown() && last != Street::River {
        stood.push((hand, last, Street::River));
    }
    let mut rng = Rng::seeded(seed);
    for (hand, before, after) in &stood {
        let was = expected(hand, deal, deal.shown(*before), &mut rng);
        let came = expected(hand, deal, deal.shown(*after), &mut rng);
        for (won, (was, came)) in won.iter_mut().zip(was.iter().zip(&came)) {
            *won -= came - was;
        }
    }
    won
}

/// Each seat's expected payout if `hand` were checked down from the `shown` cards.
fn expected(hand: &Hand, deal: &Deal, shown: CardSet, rng: &mut Rng) -> Vec<f64> {
    let held = deal.holes.iter().fold(shown, |cards, &hole| cards | hole);
    let deck: Vec<Card> = Rank::ALL
        .into_iter()
        .flat_map(|rank| Suit::ALL.map(|suit| Card::new(rank, suit)))
        .filter(|&card| !held.contains(card))
        .collect();
    let mut paid = vec![0.0; deal.holes.len()];
    let mut count = 0.0;
    let mut pay = |full: CardSet| {
        let chips = hand.payouts(|seat| strength(deal.holes[seat] | full));
        for (total, chips) in paid.iter_mut().zip(chips) {
            *total += f64::from(chips);
        }
        count += 1.0;
    };
    match 5 - shown.len() {
        0 => pay(shown),
        1 => {
            for &card in &deck {
                pay(shown | [card].into_iter().collect());
            }
        }
        2 => {
            for (i, &first) in deck.iter().enumerate() {
                for &second in &deck[i + 1..] {
                    pay(shown | [first, second].into_iter().collect());
                }
            }
        }
        missing => {
            for _ in 0..RUNOUTS {
                let mut rest = deck.clone();
                let mut full = shown;
                for _ in 0..missing {
                    full.insert(rest.swap_remove(rng.below(rest.len() as u64) as usize));
                }
                pay(full);
            }
        }
    }
    for total in &mut paid {
        *total /= count;
    }
    paid
}

/// Plays the betting of one hand. It returns the hand, the last street that was bet, and, for
/// each time cards came with betting still to follow, the hand as it stood and the streets before
/// and after.
fn bet(
    deal: &Deal,
    stacks: &[u32],
    button: usize,
    blinds: Blinds,
    mut decide: impl FnMut(usize, &Spot, &Seen) -> Action,
    finished: impl FnOnce(&[Vec<Act>], &[Record]),
) -> (Hand, Street, Vec<(Hand, Street, Street)>) {
    let seats = stacks.len();
    let mut stood = Vec::new();
    let mut hand = Hand::new(stacks, button, blinds);
    let mut lines: Vec<Vec<Act>> = vec![Vec::new(); seats];
    let mut street = hand.street();
    let mut record = Vec::new();
    // What each seat had in when the street began: before the flop nothing, so a blind is a bet.
    let mut street_start = vec![0; seats];
    // How late a seat acts after the flop: the button's left is first, and the button is last.
    let late = |seat: usize| (seat + seats - button - 1) % seats;
    while let (Some(seat), Some(legal)) = (hand.to_act(), hand.legal()) {
        if hand.street() != street {
            stood.push((hand.clone(), street, hand.street()));
            street = hand.street();
            street_start = hand.committed();
        }
        let board = deal.shown(street);
        let folded = hand.folded();
        let pot = hand.pot();
        let bets: Vec<u32> = hand
            .committed()
            .iter()
            .zip(&street_start)
            .map(|(total, start)| total - start)
            .collect();
        let totals = hand.committed();
        let before = totals[seat];
        let spot = Spot {
            hole: deal.holes[seat],
            board,
            pot,
            // What a call can win: each seat's chips up to what ours will be, so a short call
            // is never priced against a side pot it cannot win.
            callable_pot: legal
                .call
                .map(|call| svanbot_core::spot::callable_pot(&totals, before + call)),
            legal,
            opponents: folded.iter().filter(|&&out| !out).count() - 1,
            stack: hand.stacks()[seat],
            // The street bet, a posted blind included: a raise-to is this plus what it adds.
            committed: bets[seat],
            big_blind: blinds.big,
            in_position: (0..seats)
                .filter(|&other| !folded[other])
                .all(|other| late(other) <= late(seat)),
            lines: (0..seats)
                .filter(|&other| other != seat && !folded[other])
                .map(|other| lines[other].clone())
                .collect(),
            // The sim's seats are its own, but a trace read from it can still name one.
            seats: (0..seats)
                .filter(|&other| other != seat && !folded[other])
                .collect(),
        };
        let seen = Seen {
            button,
            board: deal.board[..board.len()].to_vec(),
            seats: (0..seats)
                .map(|at| SeatSeen {
                    stack: hand.stacks()[at],
                    bet: bets[at],
                    folded: folded[at],
                })
                .collect(),
            record: record.clone(),
        };
        let action = nearest(decide(seat, &spot, &seen), legal);
        if hand.act(action).is_err() {
            break;
        }
        let put_in = !matches!(action, Action::Fold | Action::Check);
        record.push(Record {
            seat,
            street,
            action,
            to: if put_in {
                bets[seat] + hand.committed()[seat] - before
            } else {
                0
            },
            pot_before: pot,
            owed_before: bets.iter().max().copied().unwrap_or(0) - bets[seat],
            bet_before: bets[seat],
            all_in: put_in && hand.stacks()[seat] == 0,
        });
        lines[seat].push(Act::played(&spot, action));
    }
    finished(&lines, &record);
    (hand, street, stood)
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

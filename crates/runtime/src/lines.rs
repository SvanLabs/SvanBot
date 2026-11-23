//! The line each seat plays in a hand: every action, with its street, the board, whether a bet
//! faced it and what it put in. Built from the venue's frames, live by the session and from stored
//! hands by [`crate::history`], so both see the same lines.
//!
//! The venue's optional `contribution_delta`, `pot_before` and `to_call_before` are used when they
//! come. Without them the line falls back on the action's `amount` and the pot it has counted.

use std::collections::{BTreeMap, BTreeSet};

use svanbot_core::card::{Card, CardSet};
use svanbot_core::rules::{Chips, Street};
use svanbot_core::spot::{Act, ActKind};

use crate::wire::Inbound;

/// The lines of one hand.
#[derive(Clone, Debug, Default)]
pub struct Lines {
    board: CardSet,
    /// The board in the order it was dealt, for display; `board` is the set for equity.
    board_order: Vec<Card>,
    street: Option<Street>,
    pot: Chips,
    /// Whether a bet stands on this street. The blinds are one preflop.
    bet: bool,
    acts: BTreeMap<usize, Vec<Act>>,
    /// Every action in the order it came, with its seat, for a reader that needs the table's
    /// turn order as well as each seat's own line.
    events: Vec<(usize, Act)>,
    folded: BTreeSet<usize>,
    /// Each seat's chips in on this street.
    committed: BTreeMap<usize, Chips>,
    small_blind: Chips,
    big_blind: Chips,
    dealer: usize,
}

impl Lines {
    /// Takes one frame of the hand.
    pub fn on(&mut self, frame: &Inbound) {
        match frame {
            Inbound::HandStart {
                small_blind,
                big_blind,
                dealer_seat,
                ..
            } => {
                *self = Self {
                    street: Some(Street::Preflop),
                    pot: small_blind + big_blind,
                    bet: true,
                    small_blind: *small_blind,
                    big_blind: *big_blind,
                    dealer: *dealer_seat,
                    ..Self::default()
                };
            }
            Inbound::CommunityCards { cards } => {
                for card in cards {
                    // A replayed frame must not deal the same card twice.
                    if !self.board.contains(*card) {
                        self.board_order.push(*card);
                    }
                }
                self.board |= cards.iter().copied().collect();
                self.street = Some(match self.board.len() {
                    0..=3 => Street::Flop,
                    4 => Street::Turn,
                    _ => Street::River,
                });
                self.bet = false;
                self.committed.clear();
            }
            Inbound::PlayerAction {
                seat,
                action,
                amount,
                detail,
                ..
            } => {
                let Some(street) = self.street else {
                    return;
                };
                let added = detail.added.unwrap_or(match action.as_str() {
                    "fold" | "check" => 0,
                    _ => *amount,
                });
                let pot = detail.pot_before.unwrap_or(self.pot);
                let kind = match action.as_str() {
                    "fold" => ActKind::Fold,
                    "check" => ActKind::Check,
                    "call" => ActKind::Call,
                    // An all-in for no more than the call is a call.
                    "all_in" if detail.to_call.is_some_and(|call| added <= call) => ActKind::Call,
                    _ => ActKind::Bet { added, pot },
                };
                let facing = match (detail.to_call, kind) {
                    (Some(call), _) => call > 0,
                    (None, ActKind::Fold | ActKind::Call) => true,
                    (None, ActKind::Check) => false,
                    (None, ActKind::Bet { .. }) => self.bet,
                };
                let act = Act {
                    street,
                    board: self.board,
                    facing,
                    kind,
                };
                self.acts.entry(*seat).or_default().push(act);
                self.events.push((*seat, act));
                self.pot = pot + added;
                *self.committed.entry(*seat).or_default() += added;
                match kind {
                    ActKind::Bet { .. } => self.bet = true,
                    ActKind::Fold => {
                        self.folded.insert(*seat);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// The seat's actions in this hand, oldest first.
    pub fn of(&self, seat: usize) -> &[Act] {
        self.acts.get(&seat).map_or(&[], Vec::as_slice)
    }

    /// Every seat that has acted, lowest first.
    pub fn seats(&self) -> impl Iterator<Item = usize> + '_ {
        self.acts.keys().copied()
    }

    /// Every action in the hand, in the order it came, with the seat that took it.
    pub fn events(&self) -> impl Iterator<Item = (usize, Act)> + '_ {
        self.events.iter().copied()
    }

    /// The seat's chips in on this street, its posted blind included before the flop. `dealt` is
    /// every seat dealt into the hand, folded or not, which places the blinds: the venue posts the
    /// small blind left of the dealer and the big blind two left, and heads up the dealer posts the
    /// small one (two left of the dealer would be the dealer itself).
    pub fn committed(&self, seat: usize, dealt: &[usize]) -> Chips {
        let acted = self.committed.get(&seat).copied().unwrap_or(0);
        if self.street != Some(Street::Preflop) {
            return acted;
        }
        let mut order: Vec<usize> = dealt.to_vec();
        order.sort_unstable();
        order.dedup();
        // Seat order from the dealer's left round to the dealer.
        order.sort_by_key(|&at| (at + 6 - self.dealer % 6 - 1) % 6);
        let (small, big) = match order.as_slice() {
            [first, second] if order.contains(&self.dealer) => {
                if *second == self.dealer {
                    (*second, *first)
                } else {
                    (*first, *second)
                }
            }
            [first, second, ..] => (*first, *second),
            _ => return acted,
        };
        let posted = if seat == small {
            self.small_blind
        } else if seat == big {
            self.big_blind
        } else {
            0
        };
        acted + posted
    }

    /// How late `seat` acts after the flop: the button's left is 0, and the button is last.
    /// The venue seats six at most.
    pub fn turn_order(&self, seat: usize) -> usize {
        (seat + 6 - self.dealer % 6 - 1) % 6
    }

    /// The hand's big blind.
    pub fn big_blind(&self) -> Chips {
        self.big_blind
    }

    /// Whether the seat has folded.
    pub fn folded(&self, seat: usize) -> bool {
        self.folded.contains(&seat)
    }

    /// The community cards so far.
    pub fn board(&self) -> CardSet {
        self.board
    }

    /// The community cards in the order they were dealt.
    pub fn board_order(&self) -> &[Card] {
        &self.board_order
    }

    /// The street the hand is on, while one is in progress.
    pub fn street(&self) -> Option<Street> {
        self.street
    }

    /// The pot as the actions have counted it.
    pub fn pot(&self) -> Chips {
        self.pot
    }

    /// The dealer's seat.
    pub fn dealer(&self) -> usize {
        self.dealer
    }
}

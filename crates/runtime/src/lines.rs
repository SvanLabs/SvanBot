//! The line each seat plays in a hand: every action, with its street, the board, whether a bet
//! faced it and what it put in. Built from the venue's frames, live by the session and from stored
//! hands by [`crate::history`], so both see the same lines.
//!
//! The venue's optional `contribution_delta`, `pot_before` and `to_call_before` are used when they
//! come. Without them the line falls back on the action's `amount` and the pot it has counted.

use std::collections::{BTreeMap, BTreeSet};

use svanbot_core::card::CardSet;
use svanbot_core::rules::Street;
use svanbot_core::spot::{Act, ActKind};

use crate::wire::Inbound;

/// The lines of one hand.
#[derive(Clone, Debug, Default)]
pub struct Lines {
    board: CardSet,
    street: Option<Street>,
    pot: u32,
    /// Whether a bet stands on this street. The blinds are one preflop.
    bet: bool,
    acts: BTreeMap<usize, Vec<Act>>,
    folded: BTreeSet<usize>,
    /// Each seat's chips in on this street.
    committed: BTreeMap<usize, u32>,
    big_blind: u32,
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
                    big_blind: *big_blind,
                    dealer: *dealer_seat,
                    ..Self::default()
                };
            }
            Inbound::CommunityCards { cards } => {
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
                self.acts.entry(*seat).or_default().push(Act {
                    street,
                    board: self.board,
                    facing,
                    kind,
                });
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

    /// The seat's chips in on this street, from its actions. Preflop blinds are not counted.
    pub fn committed(&self, seat: usize) -> u32 {
        self.committed.get(&seat).copied().unwrap_or(0)
    }

    /// How late `seat` acts after the flop: the button's left is 0, and the button is last.
    /// The venue seats six at most.
    pub fn turn_order(&self, seat: usize) -> usize {
        (seat + 6 - self.dealer % 6 - 1) % 6
    }

    /// The hand's big blind.
    pub fn big_blind(&self) -> u32 {
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
}

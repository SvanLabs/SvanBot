//! The rules of one hand of no-limit hold'em as the venue plays them
//! (<https://docs.openpoker.ai/llms-full.txt>, "Game Rules" and "Valid Actions"): the blinds, whose
//! turn it is, the legal actions with raise-to amounts, and, in [`pots`], the side pots and the
//! payouts. There is no rake.
//!
//! The rules hold no cards. The caller deals them, and passes the strengths in at the showdown.

mod pots;

use std::fmt;

use crate::card::CardSet;

/// A count of chips: a stack, a bet, a pot or a payout. 64 bits wide, so no sum of them a table
/// can make comes near its limit; the venue's 10/20 game with 2,000-chip stacks uses a sliver.
pub type Chips = u64;

/// The forced bets. The venue plays 10/20.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blinds {
    pub small: Chips,
    pub big: Chips,
}

/// A betting round, named by the cards that open it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Street {
    Preflop,
    Flop,
    Turn,
    River,
}

impl Street {
    /// The streets in the order they are played.
    pub const ALL: [Self; 4] = [Self::Preflop, Self::Flop, Self::Turn, Self::River];

    /// The street `board` is on: the one the community cards open.
    pub fn of(board: CardSet) -> Self {
        match board.len() {
            0 => Self::Preflop,
            3 => Self::Flop,
            4 => Self::Turn,
            _ => Self::River,
        }
    }

    /// The street's name, as the store keeps it and the page shows it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Preflop => "preflop",
            Self::Flop => "flop",
            Self::Turn => "turn",
            Self::River => "river",
        }
    }

    /// The street called `name`, or `None` when no street is.
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|street| street.name() == name)
    }
}

/// What a seat does when it is its turn. A raise names the total the seat's bet on this street
/// rises to, not the increase, as the venue's `amount` does. An all-in is the raise to the top of
/// the range, or a call for less than the bet when the stack is short.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Fold,
    Check,
    Call,
    RaiseTo(Chips),
}

/// The totals a raise may name, both included. When the stack cannot reach a full raise, both are
/// the stack: the only raise left is all in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RaiseRange {
    pub min: Chips,
    pub max: Chips,
}

/// The actions open to the seat whose turn it is. Folding is always open. Checking is open when
/// there is nothing to call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Legal {
    /// The chips a call puts in: the bet less what the seat has in, or its whole stack if that is
    /// less. `None` when there is nothing to call.
    pub call: Option<Chips>,
    /// `None` when the seat may not raise: its stack covers no more than a call, a short all-in
    /// has not reopened the betting to it, or nobody is left to call a raise.
    pub raise: Option<RaiseRange>,
}

impl Legal {
    /// The legal action nearest to a requested action.
    pub fn nearest(self, wanted: Action) -> Action {
        let passive = if self.call.is_some() {
            Action::Call
        } else {
            Action::Check
        };
        match wanted {
            Action::Fold | Action::Check if self.call.is_none() => Action::Check,
            Action::Fold | Action::Check => Action::Fold,
            Action::Call => passive,
            Action::RaiseTo(to) => match self.raise {
                Some(range) => Action::RaiseTo(to.clamp(range.min, range.max)),
                None => passive,
            },
        }
    }
}

/// An action the rules refuse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleError {
    /// The hand has no seat left to act.
    HandOver,
    /// The action is not among the legal ones.
    Illegal { action: Action, legal: Legal },
}

impl fmt::Display for RuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HandOver => write!(f, "the hand has no seat left to act"),
            Self::Illegal { action, legal } => write!(f, "{action:?} is not legal: {legal:?}"),
        }
    }
}

impl std::error::Error for RuleError {}

#[derive(Clone, Debug)]
struct Seat {
    /// Chips behind.
    stack: Chips,
    /// Chips put in on this street.
    street: Chips,
    /// Chips put in this hand.
    total: Chips,
    folded: bool,
    /// The bet this seat faced after it last acted on this street. `None` until it acts.
    matched: Option<Chips>,
}

impl Seat {
    fn can_act(&self) -> bool {
        !self.folded && self.stack > 0
    }

    fn put_in(&mut self, chips: Chips) {
        let chips = chips.min(self.stack);
        self.stack -= chips;
        self.street += chips;
        self.total += chips;
    }
}

/// One hand, from the blinds to the showdown, for two to six seats in table order.
#[derive(Clone, Debug)]
pub struct Hand {
    seats: Vec<Seat>,
    button: usize,
    blinds: Blinds,
    street: Street,
    to_act: Option<usize>,
    /// The most any seat has in on this street.
    bet: Chips,
    /// The increase the last full raise made. The next raise must add at least as much.
    full_raise: Chips,
}

impl Hand {
    /// Posts the blinds and finds the first seat to act. Each stack is what that seat brings to
    /// the hand, and must be more than 0. Heads-up the button posts the small blind.
    pub fn new(stacks: &[Chips], button: usize, blinds: Blinds) -> Self {
        assert!(stacks.len() >= 2, "a hand needs two seats");
        assert!(button < stacks.len(), "the button sits at a seat");
        let n = stacks.len();
        let seats = stacks
            .iter()
            .map(|&stack| Seat {
                stack,
                street: 0,
                total: 0,
                folded: false,
                matched: None,
            })
            .collect();
        let mut hand = Self {
            seats,
            button,
            blinds,
            street: Street::Preflop,
            to_act: None,
            bet: blinds.big,
            full_raise: blinds.big,
        };
        let (small, big) = match n {
            2 => (button, (button + 1) % n),
            _ => ((button + 1) % n, (button + 2) % n),
        };
        hand.seats[small].put_in(blinds.small);
        hand.seats[big].put_in(blinds.big);
        hand.to_act = hand.next_to_act(big);
        if hand.to_act.is_none() {
            hand.close_street();
        }
        hand
    }

    /// The seat whose turn it is, or `None` once the hand is over.
    pub fn to_act(&self) -> Option<usize> {
        self.to_act
    }

    /// The street being bet.
    pub fn street(&self) -> Street {
        self.street
    }

    /// The button's seat.
    pub fn button(&self) -> usize {
        self.button
    }

    /// The chips each seat has put in this hand.
    pub fn committed(&self) -> Vec<Chips> {
        self.seats.iter().map(|seat| seat.total).collect()
    }

    /// The chips each seat has behind.
    pub fn stacks(&self) -> Vec<Chips> {
        self.seats.iter().map(|seat| seat.stack).collect()
    }

    /// Whether each seat has folded.
    pub fn folded(&self) -> Vec<bool> {
        self.seats.iter().map(|seat| seat.folded).collect()
    }

    /// The chips in the middle.
    pub fn pot(&self) -> Chips {
        self.seats.iter().map(|seat| seat.total).sum()
    }

    /// Whether the hand ended with two or more seats left in, so the cards decide it.
    pub fn is_showdown(&self) -> bool {
        self.to_act.is_none() && self.seats.iter().filter(|seat| !seat.folded).count() >= 2
    }

    /// The actions open to the seat whose turn it is, or `None` once the hand is over.
    pub fn legal(&self) -> Option<Legal> {
        let at = self.to_act?;
        let seat = &self.seats[at];
        let to_call = self.bet.saturating_sub(seat.street);
        let call = (to_call > 0).then(|| to_call.min(seat.stack));
        let reopened = seat
            .matched
            .is_none_or(|matched| self.bet - matched >= self.full_raise);
        let someone_to_call = self
            .seats
            .iter()
            .enumerate()
            .any(|(i, other)| i != at && other.can_act());
        let raise = (seat.stack > to_call && reopened && someone_to_call).then(|| {
            let max = seat.street + seat.stack;
            RaiseRange {
                min: (self.bet + self.full_raise).min(max),
                max,
            }
        });
        Some(Legal { call, raise })
    }

    /// Plays `action` for the seat whose turn it is.
    pub fn act(&mut self, action: Action) -> Result<(), RuleError> {
        let at = self.to_act.ok_or(RuleError::HandOver)?;
        let legal = self.legal().ok_or(RuleError::HandOver)?;
        let allowed = match action {
            Action::Fold => true,
            Action::Check => legal.call.is_none(),
            Action::Call => legal.call.is_some(),
            Action::RaiseTo(to) => legal
                .raise
                .is_some_and(|range| (range.min..=range.max).contains(&to)),
        };
        if !allowed {
            return Err(RuleError::Illegal { action, legal });
        }
        let seat = &mut self.seats[at];
        match action {
            Action::Fold => seat.folded = true,
            Action::Check => {}
            Action::Call => seat.put_in(self.bet - seat.street),
            Action::RaiseTo(to) => {
                seat.put_in(to - seat.street);
                // A short all-in raises the bet but not the size the next raise must add.
                self.full_raise = self.full_raise.max(to - self.bet);
                self.bet = to;
            }
        }
        self.seats[at].matched = Some(self.bet);
        let left_in = self.seats.iter().filter(|seat| !seat.folded).count();
        self.to_act = match left_in {
            1 => None,
            _ => self.next_to_act(at),
        };
        if self.to_act.is_none() && left_in > 1 {
            self.close_street();
        }
        Ok(())
    }

    /// The first seat after `from`, going round the table, that still has to act on this street.
    fn next_to_act(&self, from: usize) -> Option<usize> {
        let n = self.seats.len();
        let can_act = self.seats.iter().filter(|seat| seat.can_act()).count();
        (1..=n).map(|step| (from + step) % n).find(|&i| {
            let seat = &self.seats[i];
            seat.can_act() && (seat.street < self.bet || (seat.matched.is_none() && can_act >= 2))
        })
    }

    /// Ends the street's betting, and opens the next street that has betting left in it. With
    /// fewer than two seats able to bet, the streets run out to the showdown.
    fn close_street(&mut self) {
        while self.to_act.is_none() && self.street != Street::River {
            self.street = match self.street {
                Street::Preflop => Street::Flop,
                Street::Flop => Street::Turn,
                Street::Turn | Street::River => Street::River,
            };
            for seat in &mut self.seats {
                seat.street = 0;
                seat.matched = None;
            }
            self.bet = 0;
            self.full_raise = self.blinds.big;
            self.to_act = self.next_to_act(self.button);
        }
    }
}

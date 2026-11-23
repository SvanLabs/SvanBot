//! The spot: a point where a bot must act, as the decision sees it.

use crate::card::CardSet;
use crate::rules::{Legal, Street};

/// What a decision knows when the bot must act.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spot {
    /// The bot's hole cards.
    pub hole: CardSet,
    /// The community cards, zero to five.
    pub board: CardSet,
    /// The chips in the middle, the bets on this street included.
    pub pot: u32,
    /// Chips already in the pot the bot could win by calling, excluding excess side-pot chips.
    /// `None` when the caller does not know the hand contributions.
    pub callable_pot: Option<u32>,
    /// The actions open to the bot.
    pub legal: Legal,
    /// How many opponents are still in the hand.
    pub opponents: usize,
    /// The bot's chips behind.
    pub stack: u32,
    /// The chips the bot has put in on this street already: a raise-to is this plus what it adds.
    pub committed: u32,
    /// The hand's big blind, the unit a pot is sized in.
    pub big_blind: u32,
    /// Whether the bot acts last on the streets after the flop: of the seats still in, it is the
    /// last one round the table from the button's left.
    pub in_position: bool,
    /// What each opponent still in the hand has done in it so far, one line per opponent. Empty
    /// when the caller did not follow the hand.
    pub lines: Vec<Vec<Act>>,
    /// The seat each entry of [`Spot::lines`] came from, in the same order: `seats[i]` is the seat
    /// of the opponent whose line is `lines[i]`. A reader matches a seat to the table, so a trace
    /// names its opponents by seat rather than by a position in a list. One entry per line.
    pub seats: Vec<usize>,
}

/// Chips already in the pot at or below the caller's final contribution, folded seats included.
/// `totals` are the whole hand's contributions before the call, not just this street's bets.
pub fn callable_pot(totals: &[u32], cap: u32) -> u32 {
    totals.iter().map(|&chips| chips.min(cap)).sum()
}

/// One action a player took, as the table saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Act {
    pub street: Street,
    /// The community cards when they acted.
    pub board: CardSet,
    /// Whether a bet faced them.
    pub facing: bool,
    pub kind: ActKind,
}

/// What a player did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActKind {
    Fold,
    Check,
    Call,
    /// A bet or raise: the chips it added, and the pot before it.
    Bet {
        added: u32,
        pot: u32,
    },
}

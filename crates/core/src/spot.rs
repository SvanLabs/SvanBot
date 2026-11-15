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
    /// The actions open to the bot.
    pub legal: Legal,
    /// How many opponents are still in the hand.
    pub opponents: usize,
    /// The bot's chips behind.
    pub stack: u32,
    /// What each opponent still in the hand has done in it so far, one line per opponent. Empty
    /// when the caller did not follow the hand.
    pub lines: Vec<Vec<Act>>,
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

//! The spot: a point where a bot must act, as the decision sees it.

use crate::card::CardSet;
use crate::rules::Legal;

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
}

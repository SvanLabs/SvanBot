//! The hand evaluator: the strength of the best five of five to seven cards.
//!
//! It follows OMPEval (<https://github.com/zekyll/OMPEval>). A set of cards folds into a [`Key`]
//! by addition, one card at a time, so a board's key is made once and each player's two cards
//! cost two adds and one table read. The key is three sums:
//! - a weight per rank, chosen so that no two sets of up to seven ranks share a sum. A perfect
//!   hash maps the sum to the strength of those ranks without a flush.
//! - a counter per suit, each starting at 3, so a suit holding five or more cards sets the top
//!   bit of its nibble. That is the flush check.
//! - a 13-bit rank mask per suit, which reads the flush's strength from a table of 8,192.
//!
//! The tables are built in-process the first time a strength is read, from the rules alone, so
//! the evaluator needs no file and no build script.

mod tables;

use std::sync::LazyLock;

use crate::card::{Card, CardSet};

/// The ranking a strength falls in, lowest first. A royal flush is the highest straight flush.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    HighCard,
    OnePair,
    TwoPair,
    ThreeOfAKind,
    Straight,
    Flush,
    FullHouse,
    FourOfAKind,
    StraightFlush,
}

impl Category {
    const ALL: [Self; 9] = [
        Self::HighCard,
        Self::OnePair,
        Self::TwoPair,
        Self::ThreeOfAKind,
        Self::Straight,
        Self::Flush,
        Self::FullHouse,
        Self::FourOfAKind,
        Self::StraightFlush,
    ];
}

/// How the best five cards rank at showdown: the higher strength wins, and equal strengths split.
/// There are 7,462 strengths, numbered from 0 (7-5-4-3-2 offsuit) to 7,461 (a royal flush).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Strength(u16);

impl Strength {
    /// The first strength in each category: the number of distinct five-card hands in the
    /// categories below it.
    const CATEGORY_STARTS: [u16; 9] = [0, 1277, 4137, 4995, 5853, 5863, 7140, 7296, 7452];

    /// The ranking this strength falls in.
    pub fn category(self) -> Category {
        let at = Self::CATEGORY_STARTS.partition_point(|&start| start <= self.0);
        Category::ALL[at - 1]
    }
}

/// The weight each rank adds to a key, deuce first. No two sets of up to seven ranks, with at most
/// four of each, share a sum. The first twelve came from a greedy search for the smallest weight
/// that keeps the sums apart; the ace's is one more than the largest sum the other twelve reach,
/// which keeps them apart without a search. `tests/eval.rs` checks every such set.
pub const RANK_WEIGHTS: [u32; 13] = [
    1, 5, 24, 112, 521, 2247, 9244, 30823, 103066, 250154, 667453, 1526359, 8107796,
];

/// The suit counters before any card: 3 in each nibble, so a fifth card of a suit reaches 8.
const SUIT_COUNTERS: u32 = 0x3333;
/// The top bit of each suit's nibble: set once that suit holds five cards.
const FLUSH_BITS: u32 = 0x8888;

/// Cards folded into the form the evaluator reads. Adding a card is three additions, so a board's
/// key is made once and each player's cards are added to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    ranks: u32,
    suits: u32,
    /// Bit `16 * suit + rank` for each card.
    cards: u64,
}

impl Key {
    /// The key of no cards.
    pub const EMPTY: Self = Self {
        ranks: 0,
        suits: SUIT_COUNTERS,
        cards: 0,
    };

    /// The key of these cards.
    pub fn of(cards: CardSet) -> Self {
        cards.iter().fold(Self::EMPTY, Self::add)
    }

    /// This key with `card` added. The key must not already hold the card.
    #[must_use]
    pub const fn add(self, card: Card) -> Self {
        let rank = card.rank() as u32;
        let suit = card.suit() as u32;
        let bit = 1 << (16 * suit + rank);
        debug_assert!(self.cards & bit == 0, "a card added twice");
        Self {
            ranks: self.ranks + RANK_WEIGHTS[rank as usize],
            suits: self.suits + (1 << (4 * suit)),
            cards: self.cards | bit,
        }
    }

    /// The strength of the best five cards. The key must hold five, six or seven cards.
    pub fn strength(self) -> Strength {
        debug_assert!(
            (5..=7).contains(&self.cards.count_ones()),
            "a strength is read from five to seven cards"
        );
        let tables = &*TABLES;
        let flush = self.suits & FLUSH_BITS;
        if flush == 0 {
            return Strength(tables.ranks.get(self.ranks));
        }
        // Seven cards hold at most one suit of five, and a flush beats any hand it does not
        // contain: four of a kind or a full house would need two more cards outside the suit.
        let suit = (flush.trailing_zeros() - 3) / 4;
        let mask = (self.cards >> (16 * suit)) & 0x1fff;
        Strength(tables.flushes[mask as usize])
    }
}

/// The strength of the best five of `cards`, which must hold five, six or seven.
pub fn strength(cards: CardSet) -> Strength {
    Key::of(cards).strength()
}

static TABLES: LazyLock<tables::Tables> = LazyLock::new(tables::Tables::build);

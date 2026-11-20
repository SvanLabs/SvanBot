//! Cards, and the venue's two-character notation for them: a rank (`2`–`9`, `T`, `J`, `Q`, `K`,
//! `A`), then a suit (`c`, `d`, `h`, `s`). `Ah` is the ace of hearts.

use std::fmt;
use std::str::FromStr;

/// A card's rank, deuce lowest and ace highest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rank {
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    Ten,
    Jack,
    Queen,
    King,
    Ace,
}

impl Rank {
    /// Every rank, lowest first.
    pub const ALL: [Self; 13] = [
        Self::Two,
        Self::Three,
        Self::Four,
        Self::Five,
        Self::Six,
        Self::Seven,
        Self::Eight,
        Self::Nine,
        Self::Ten,
        Self::Jack,
        Self::Queen,
        Self::King,
        Self::Ace,
    ];

    const NOTATION: [char; 13] = [
        '2', '3', '4', '5', '6', '7', '8', '9', 'T', 'J', 'Q', 'K', 'A',
    ];

    fn from_notation(c: char) -> Option<Self> {
        let at = Self::NOTATION.iter().position(|&n| n == c)?;
        Some(Self::ALL[at])
    }
}

/// A card's suit. Suits have no order in hold'em; the one here only gives each card its index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Suit {
    Clubs,
    Diamonds,
    Hearts,
    Spades,
}

impl Suit {
    /// Every suit, in index order.
    pub const ALL: [Self; 4] = [Self::Clubs, Self::Diamonds, Self::Hearts, Self::Spades];

    const NOTATION: [char; 4] = ['c', 'd', 'h', 's'];

    fn from_notation(c: char) -> Option<Self> {
        let at = Self::NOTATION.iter().position(|&n| n == c)?;
        Some(Self::ALL[at])
    }
}

/// One of the 52 cards. Cards order by rank, then by suit.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Card(u8);

impl Card {
    /// The card of this rank and suit.
    pub const fn new(rank: Rank, suit: Suit) -> Self {
        Self(rank as u8 * 4 + suit as u8)
    }

    /// This card's rank.
    pub const fn rank(self) -> Rank {
        Rank::ALL[(self.0 / 4) as usize]
    }

    /// This card's suit.
    pub const fn suit(self) -> Suit {
        Suit::ALL[(self.0 % 4) as usize]
    }
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rank = Rank::NOTATION[self.rank() as usize];
        let suit = Suit::NOTATION[self.suit() as usize];
        write!(f, "{rank}{suit}")
    }
}

impl fmt::Debug for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Card({self})")
    }
}

/// Text that is not in the venue's card notation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseCardError {
    text: String,
}

impl fmt::Display for ParseCardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} is not in the card notation: a rank, then a suit, as in Ah, each card once",
            self.text
        )
    }
}

impl std::error::Error for ParseCardError {}

impl FromStr for Card {
    type Err = ParseCardError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut chars = text.chars();
        let card = match (chars.next(), chars.next(), chars.next()) {
            (Some(rank), Some(suit), None) => {
                Rank::from_notation(rank).zip(Suit::from_notation(suit))
            }
            _ => None,
        };
        card.map(|(rank, suit)| Self::new(rank, suit))
            .ok_or_else(|| ParseCardError {
                text: text.to_owned(),
            })
    }
}

/// A set of cards, such as a board, a player's hole cards or what is left of the deck.
///
/// Ordered by its bits, which is an arbitrary but total and stable order: it is what lets a pure
/// layer key a `BTreeMap` by a set where it may not use a `HashMap` (`clippy.toml`).
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CardSet(u64);

impl CardSet {
    /// The set with no cards.
    pub const EMPTY: Self = Self(0);

    /// Adds `card`, and says whether the set lacked it.
    pub fn insert(&mut self, card: Card) -> bool {
        let had = self.contains(card);
        self.0 |= 1 << card.0;
        !had
    }

    /// Whether the set holds `card`.
    pub const fn contains(self, card: Card) -> bool {
        self.0 & (1 << card.0) != 0
    }

    /// How many cards the set holds.
    pub const fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    /// Whether the set holds no card.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether the two sets share no card.
    pub const fn is_disjoint(self, other: Self) -> bool {
        self.0 & other.0 == 0
    }

    /// The cards in the set, lowest first: by rank, then by suit.
    pub fn iter(self) -> impl Iterator<Item = Card> {
        let mut bits = self.0;
        std::iter::from_fn(move || {
            let index = bits.trailing_zeros();
            (bits != 0).then(|| {
                bits &= bits - 1;
                Card(index as u8)
            })
        })
    }
}

impl std::ops::BitOr for CardSet {
    type Output = Self;

    /// The cards in either set.
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl FromIterator<Card> for CardSet {
    fn from_iter<I: IntoIterator<Item = Card>>(cards: I) -> Self {
        let mut set = Self::EMPTY;
        for card in cards {
            set.insert(card);
        }
        set
    }
}

impl std::ops::BitOrAssign for CardSet {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

impl fmt::Display for CardSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.iter().try_for_each(|card| write!(f, "{card}"))
    }
}

impl fmt::Debug for CardSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CardSet({self})")
    }
}

impl FromStr for CardSet {
    type Err = ParseCardError;

    /// Reads cards written one after another, as in `AhKhQh`. A card named twice is refused.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refused = || ParseCardError {
            text: text.to_owned(),
        };
        let chars: Vec<char> = text.chars().collect();
        let mut set = Self::EMPTY;
        for pair in chars.chunks(2) {
            let card: Card = pair
                .iter()
                .collect::<String>()
                .parse()
                .map_err(|_| refused())?;
            if !set.insert(card) {
                return Err(refused());
            }
        }
        Ok(set)
    }
}

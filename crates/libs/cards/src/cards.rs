//! Card encoding (`suit * 13 + rank`), text parsing and 52-bit card masks.

use std::fmt;

/// Rank letters, index = rank (0 = deuce .. 12 = ace).
pub const RANK_CHARS: &[u8; 13] = b"23456789TJQKA";
/// Suit letters, index = suit (clubs, diamonds, hearts, spades).
pub const SUIT_CHARS: &[u8; 4] = b"cdhs";

/// A card encoded as `suit * 13 + rank` (rank 0 = deuce .. 12 = ace), so a
/// card's bit in a [`CardMask`] groups each suit into its own 13-bit lane.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Card(pub u8);

/// A set of cards as a 52-bit mask: bit `card.0` is set for each card.
pub type CardMask = u64;

impl Card {
    /// The card of `rank` (0..13) and `suit` (0..4).
    pub fn new(rank: u8, suit: u8) -> Card {
        debug_assert!(rank < 13 && suit < 4);
        Card(suit * 13 + rank)
    }
    /// Rank 0 (deuce) .. 12 (ace).
    pub fn rank(self) -> u8 {
        self.0 % 13
    }
    /// Suit 0..4 in [`SUIT_CHARS`] order.
    pub fn suit(self) -> u8 {
        self.0 / 13
    }
    /// This card's bit in a [`CardMask`].
    pub fn bit(self) -> CardMask {
        1u64 << self.0
    }
    /// Parse two-letter text such as `"Ah"` or `"td"` (rank case-insensitive); `None` otherwise.
    pub fn parse(s: &str) -> Option<Card> {
        let b = s.as_bytes();
        if b.len() != 2 {
            return None;
        }
        let r = RANK_CHARS.iter().position(|&c| c == b[0].to_ascii_uppercase())?;
        let su = SUIT_CHARS.iter().position(|&c| c == b[1].to_ascii_lowercase())?;
        Some(Card::new(r as u8, su as u8))
    }
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", RANK_CHARS[self.rank() as usize] as char, SUIT_CHARS[self.suit() as usize] as char)
    }
}

/// Parse every card, or `None` if any fails.
pub fn parse_cards<S: AsRef<str>>(items: &[S]) -> Option<Vec<Card>> {
    items.iter().map(|s| Card::parse(s.as_ref())).collect()
}

/// The mask holding all `cards`.
pub fn mask_of(cards: &[Card]) -> CardMask {
    cards.iter().fold(0, |m, c| m | c.bit())
}

/// The cards in `mask`, lowest card index first.
pub fn cards_in(mask: CardMask) -> impl Iterator<Item = Card> {
    let mut m = mask;
    std::iter::from_fn(move || {
        if m == 0 {
            None
        } else {
            let i = m.trailing_zeros() as u8;
            m &= m - 1;
            Some(Card(i))
        }
    })
}

impl serde::Serialize for Card {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for Card {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Card, D::Error> {
        let s = <String as serde::Deserialize>::deserialize(d)?;
        Card::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("bad card {s}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_roundtrip_all_52() {
        for i in 0..52u8 {
            let c = Card(i);
            assert_eq!(Card::parse(&c.to_string()), Some(c));
        }
        assert_eq!(Card::parse("Ah"), Some(Card::new(12, 2)));
        assert_eq!(Card::parse("2c"), Some(Card::new(0, 0)));
        assert_eq!(Card::parse("??"), None);
    }

    #[test]
    fn mask_iteration() {
        let cs = parse_cards(&["Ah", "Kd", "2c"]).unwrap();
        let m = mask_of(&cs);
        let mut back: Vec<Card> = cards_in(m).collect();
        back.sort();
        let mut cs2 = cs.clone();
        cs2.sort();
        assert_eq!(back, cs2);
    }
}

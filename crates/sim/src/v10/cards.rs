//! v10.0.2's cards: a card is `suit * 13 + rank` and a set of them a 52-bit mask, the 1,326
//! two-card combos run in v10's order, and a range weighs each combo. Every ported table indexes
//! by these, and the draws pick card numbers, so the encoding is v10's and not `core`'s.

use std::fmt;

/// The ranks, deuce first, as text.
pub const RANK_CHARS: &[u8; 13] = b"23456789TJQKA";
/// The suits as text, in v10's order.
pub const SUIT_CHARS: &[u8; 4] = b"cdhs";

/// A card: `suit * 13 + rank`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Card(pub u8);

/// A set of cards: bit `n` is card `n`.
pub type CardMask = u64;

impl Card {
    /// The card of `rank` (0 is a deuce) and `suit`.
    pub fn new(rank: u8, suit: u8) -> Self {
        debug_assert!(rank < 13 && suit < 4);
        Self(suit * 13 + rank)
    }

    /// Its rank, 0 for a deuce to 12 for an ace.
    pub fn rank(self) -> u8 {
        self.0 % 13
    }

    /// Its suit, in the order of [`SUIT_CHARS`].
    pub fn suit(self) -> u8 {
        self.0 / 13
    }

    /// Its bit in a [`CardMask`].
    pub fn bit(self) -> CardMask {
        1 << self.0
    }

    /// The card a text such as `Ah` names.
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.len() != 2 {
            return None;
        }
        let rank = RANK_CHARS
            .iter()
            .position(|&c| c == bytes[0].to_ascii_uppercase())?;
        let suit = SUIT_CHARS
            .iter()
            .position(|&c| c == bytes[1].to_ascii_lowercase())?;
        Some(Self::new(rank as u8, suit as u8))
    }
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rank = RANK_CHARS[usize::from(self.rank())] as char;
        let suit = SUIT_CHARS[usize::from(self.suit())] as char;
        write!(f, "{rank}{suit}")
    }
}

/// The mask of `cards`.
pub fn mask_of(cards: &[Card]) -> CardMask {
    cards.iter().fold(0, |mask, card| mask | card.bit())
}

/// How many two-card combos a deck holds.
pub const NUM_COMBOS: usize = 1326;

/// The combos, lower card first, and the combo of each pair of cards.
pub struct ComboTable {
    pub cards: [(Card, Card); NUM_COMBOS],
    /// `index[a][b]`: the combo of cards `a` and `b`, or `u16::MAX` when `a == b`.
    pub index: [[u16; 52]; 52],
}

static COMBOS: ComboTable = {
    let mut cards = [(Card(0), Card(0)); NUM_COMBOS];
    let mut index = [[u16::MAX; 52]; 52];
    let mut i = 0;
    let mut a = 0u8;
    while a < 52 {
        let mut b = a + 1;
        while b < 52 {
            cards[i] = (Card(a), Card(b));
            index[a as usize][b as usize] = i as u16;
            index[b as usize][a as usize] = i as u16;
            i += 1;
            b += 1;
        }
        a += 1;
    }
    ComboTable { cards, index }
};

static COMBO_MASKS: [CardMask; NUM_COMBOS] = {
    let mut masks = [0; NUM_COMBOS];
    let mut i = 0;
    while i < NUM_COMBOS {
        let (a, b) = COMBOS.cards[i];
        masks[i] = (1 << a.0) | (1 << b.0);
        i += 1;
    }
    masks
};

/// The combo table.
pub fn combos() -> &'static ComboTable {
    &COMBOS
}

/// The combo of `a` and `b`, or `None` for one card twice or a number past the deck.
pub fn combo_index(a: Card, b: Card) -> Option<usize> {
    match *COMBOS.index.get(usize::from(a.0))?.get(usize::from(b.0))? {
        u16::MAX => None,
        i => Some(usize::from(i)),
    }
}

/// The cards of combo `i`, as a mask.
pub fn combo_mask(i: usize) -> CardMask {
    COMBO_MASKS[i]
}

/// The preflop class of two cards: 0 to 12 the pairs, then 78 suited and 78 offsuit classes.
pub fn hand_class(a: Card, b: Card) -> usize {
    let (hi, lo) = if a.rank() >= b.rank() { (a, b) } else { (b, a) };
    let (h, l) = (usize::from(hi.rank()), usize::from(lo.rank()));
    if h == l {
        return h;
    }
    let k = h * (h - 1) / 2 + l;
    if hi.suit() == lo.suit() {
        13 + k
    } else {
        13 + 78 + k
    }
}

/// A class's name, such as `AKs`, `T2o` or `AA`.
pub fn class_name(class: usize) -> String {
    let rank = |i: usize| RANK_CHARS[i] as char;
    if class < 13 {
        return format!("{}{}", rank(class), rank(class));
    }
    let (k, suited) = if class < 91 {
        (class - 13, true)
    } else {
        (class - 91, false)
    };
    let mut h = 1;
    while (h + 1) * h / 2 <= k {
        h += 1;
    }
    let l = k - h * (h - 1) / 2;
    format!("{}{}{}", rank(h), rank(l), if suited { 's' } else { 'o' })
}

/// How many combos a class holds.
pub fn class_combos(class: usize) -> f64 {
    if class < 13 {
        6.0
    } else if class < 91 {
        4.0
    } else {
        12.0
    }
}

/// A weight on each combo.
#[derive(Clone, Debug, PartialEq)]
pub struct Range {
    pub w: Box<[f32; NUM_COMBOS]>,
}

impl Default for Range {
    fn default() -> Self {
        Self::full()
    }
}

impl Range {
    /// Every combo at weight 1.
    pub fn full() -> Self {
        Self {
            w: Box::new([1.0; NUM_COMBOS]),
        }
    }

    /// Every combo at weight 0.
    pub fn empty() -> Self {
        Self {
            w: Box::new([0.0; NUM_COMBOS]),
        }
    }

    /// Zero every combo that holds a card of `dead`.
    pub fn remove_dead(&mut self, dead: CardMask) {
        for (i, weight) in self.w.iter_mut().enumerate() {
            if combo_mask(i) & dead != 0 {
                *weight = 0.0;
            }
        }
    }

    /// The sum of the weights.
    pub fn total(&self) -> f64 {
        self.w.iter().map(|&x| f64::from(x)).sum()
    }

    /// Scale the weights to sum to 1, unless they sum to 0.
    pub fn normalize(&mut self) {
        let total = self.total();
        if total > 0.0 {
            for x in self.w.iter_mut() {
                *x = (f64::from(*x) / total) as f32;
            }
        }
    }

    /// Whether the weights sum to nothing.
    pub fn is_empty(&self) -> bool {
        self.total() <= 1e-9
    }
}

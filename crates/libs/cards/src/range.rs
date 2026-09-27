//! The 1,326 two-card combos (compile-time tables), preflop hand classes and weighted ranges.

use crate::cards::{Card, CardMask, RANK_CHARS};

/// Number of distinct two-card combos (52 choose 2).
pub const NUM_COMBOS: usize = 1326;

/// All 1326 two-card combos, indexable in both directions.
pub struct ComboTable {
    /// Combo index → its two cards (lower card index first).
    pub cards: [(Card, Card); NUM_COMBOS],
    /// Card index pair (either order) → combo index; `u16::MAX` on the diagonal, which
    /// [`combo_index`] reports as `None` instead of returning it as an index.
    pub index: [[u16; 52]; 52],
}

/// The combo table, built at compile time (no lazy initialization on hot paths).
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

/// Card mask of every combo, at compile time.
static COMBO_MASKS: [CardMask; NUM_COMBOS] = {
    let mut m = [0u64; NUM_COMBOS];
    let mut i = 0;
    while i < NUM_COMBOS {
        let (a, b) = COMBOS.cards[i];
        m[i] = (1u64 << a.0) | (1u64 << b.0);
        i += 1;
    }
    m
};

/// The compile-time combo table.
pub fn combos() -> &'static ComboTable {
    &COMBOS
}

/// Combo index of two distinct cards, in either order; `None` when the pair is not a legal hand
/// (the same card twice, or an index outside the 52-card deck). The table's `u16::MAX` diagonal is
/// an encoding, not an index, so it can never come back as one.
#[inline]
pub fn combo_index(a: Card, b: Card) -> Option<usize> {
    match *COMBOS.index.get(a.0 as usize)?.get(b.0 as usize)? {
        u16::MAX => None,
        i => Some(i as usize),
    }
}

/// Card mask of combo `i`.
#[inline]
pub fn combo_mask(i: usize) -> CardMask {
    COMBO_MASKS[i]
}

/// Preflop hand class 0..169: pairs 0..13 (by rank), then suited and offsuit
/// non-pairs. The id is only an identifier; strength ordering lives in `preflop`.
pub fn hand_class(a: Card, b: Card) -> usize {
    let (hi, lo) = if a.rank() >= b.rank() { (a, b) } else { (b, a) };
    let (h, l) = (hi.rank() as usize, lo.rank() as usize);
    if h == l {
        return h;
    }
    // Unordered pairs (h > l) map to 0..78.
    let k = h * (h - 1) / 2 + l;
    if hi.suit() == lo.suit() { 13 + k } else { 13 + 78 + k }
}

/// Conventional name of a hand class: `"AA"`, `"AKs"`, `"72o"`.
pub fn class_name(class: usize) -> String {
    let r = |i: usize| RANK_CHARS[i] as char;
    if class < 13 {
        return format!("{}{}", r(class), r(class));
    }
    let (k, suited) = if class < 91 { (class - 13, true) } else { (class - 91, false) };
    let mut h = 1;
    while (h + 1) * h / 2 <= k {
        h += 1;
    }
    let l = k - h * (h - 1) / 2;
    format!("{}{}{}", r(h), r(l), if suited { 's' } else { 'o' })
}

/// Number of combos in a class: 6 for pairs, 4 suited, 12 offsuit.
pub fn class_combos(class: usize) -> f64 {
    if class < 13 {
        6.0
    } else if class < 91 {
        4.0
    } else {
        12.0
    }
}

/// Weighted distribution over the 1326 hole-card combos.
#[derive(Clone)]
pub struct Range {
    /// Weight per combo index (unnormalized unless [`Range::normalize`] was called).
    pub w: Box<[f32; NUM_COMBOS]>,
}

impl Default for Range {
    fn default() -> Self {
        Range::full()
    }
}

impl Range {
    /// Every combo with weight 1.
    pub fn full() -> Range {
        Range { w: Box::new([1.0; NUM_COMBOS]) }
    }
    /// Every combo with weight 0.
    pub fn empty() -> Range {
        Range { w: Box::new([0.0; NUM_COMBOS]) }
    }
    /// Zero every combo that uses a card in `dead`.
    pub fn remove_dead(&mut self, dead: CardMask) {
        for i in 0..NUM_COMBOS {
            if combo_mask(i) & dead != 0 {
                self.w[i] = 0.0;
            }
        }
    }
    /// Sum of all weights.
    pub fn total(&self) -> f64 {
        self.w.iter().map(|&x| x as f64).sum()
    }
    /// Scale weights to sum to 1 (no-op for an all-zero range).
    pub fn normalize(&mut self) {
        let t = self.total();
        if t > 0.0 {
            for x in self.w.iter_mut() {
                *x = (*x as f64 / t) as f32;
            }
        }
    }
    /// Whether the total weight is effectively zero.
    pub fn is_empty(&self) -> bool {
        self.total() <= 1e-9
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combo_table_is_bijective() {
        let t = combos();
        for (i, &(a, b)) in t.cards.iter().enumerate() {
            assert!(a < b);
            assert_eq!(combo_index(a, b), Some(i));
            assert_eq!(combo_index(b, a), Some(i));
        }
    }

    /// Every slot the compile-time build never writes is `u16::MAX`, and `combo_index` must report
    /// that encoding as `None` rather than hand 65535 to a caller that will index a 1326-entry
    /// table with it.
    #[test]
    fn the_sentinel_never_comes_back_as_an_index() {
        for c in 0..52u8 {
            assert_eq!(combo_index(Card(c), Card(c)), None, "the diagonal, card {c}");
        }
        assert_eq!(combo_index(Card(0), Card(52)), None, "one card past the deck");
        assert_eq!(combo_index(Card(255), Card(0)), None, "a byte that is not a card at all");
    }

    /// A pair that is a legal hand still resolves, so the rejection above is not the whole function.
    #[test]
    fn a_real_pair_still_has_an_index() {
        let a = Card::parse("Ah").unwrap();
        let b = Card::parse("Kd").unwrap();
        assert_eq!(combo_index(a, b), combo_index(b, a));
        assert!(combo_index(a, b).is_some_and(|i| i < NUM_COMBOS));
    }

    #[test]
    fn classes_cover_1326_combos() {
        let mut counts = [0usize; 169];
        for &(a, b) in combos().cards.iter() {
            counts[hand_class(a, b)] += 1;
        }
        for (c, &n) in counts.iter().enumerate() {
            assert_eq!(n as f64, class_combos(c), "class {}", class_name(c));
        }
        let ak = hand_class(Card::parse("Ah").unwrap(), Card::parse("Kh").unwrap());
        assert_eq!(class_name(ak), "AKs");
        let t2 = hand_class(Card::parse("2d").unwrap(), Card::parse("Tc").unwrap());
        assert_eq!(class_name(t2), "T2o");
        assert_eq!(class_name(12), "AA");
    }
}

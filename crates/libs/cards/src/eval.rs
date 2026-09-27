//! The 7-card hand evaluator (exhaustively verified; see the ignored test in `sv10-core`) and
//! hand categories.

use crate::cards::CardMask;

/// Totally ordered hand strength: category in bits 20..24, then up to five
/// 4-bit rank nibbles. Higher is better; equal values tie.
pub type HandValue = u32;

/// Category of a hand with no pair.
pub const HIGH_CARD: u32 = 0;
/// Category: one pair.
pub const PAIR: u32 = 1;
/// Category: two pair.
pub const TWO_PAIR: u32 = 2;
/// Category: three of a kind.
pub const TRIPS: u32 = 3;
/// Category: straight (wheel included).
pub const STRAIGHT: u32 = 4;
/// Category: flush.
pub const FLUSH: u32 = 5;
/// Category: full house.
pub const FULL_HOUSE: u32 = 6;
/// Category: four of a kind.
pub const QUADS: u32 = 7;
/// Category: straight flush.
pub const STRAIGHT_FLUSH: u32 = 8;

/// The category (`HIGH_CARD` .. `STRAIGHT_FLUSH`) of a [`HandValue`].
pub fn category(v: HandValue) -> u32 {
    v >> 20
}

/// Human-readable category names, indexed by category.
pub const CATEGORY_NAMES: [&str; 9] =
    ["high card", "pair", "two pair", "trips", "straight", "flush", "full house", "quads", "straight flush"];

/// The highest straight's top rank in a 13-bit rank set, if any.
#[inline]
fn straight_high(ranks: u32) -> Option<u32> {
    // Bit 0 is the ace-low slot, bit i+1 is rank i. Bit b of `run` is set when slots b..b+4 are all
    // present, so its highest set bit is the lowest slot of the best straight, whose top card is
    // rank b + 3: four ANDs and one LZCNT instead of a loop over the ten windows (0230; the test
    // below checks every rank set against that loop).
    let ext = (ranks << 1) | ((ranks >> 12) & 1);
    let run = ext & (ext >> 1) & (ext >> 2) & (ext >> 3) & (ext >> 4);
    if run == 0 { None } else { Some(31 - run.leading_zeros() + 3) }
}

#[inline]
fn top_ranks(mut ranks: u32, n: usize) -> u32 {
    let mut out = 0;
    for i in 0..n {
        let r = 31 - ranks.leading_zeros();
        ranks &= !(1 << r);
        out |= r << (4 * (4 - i));
    }
    out
}

/// Evaluate the best five-card hand contained in `mask` (5 to 7 cards).
#[inline]
pub fn eval(mask: CardMask) -> HandValue {
    let s = [(mask & 0x1FFF) as u32, ((mask >> 13) & 0x1FFF) as u32, ((mask >> 26) & 0x1FFF) as u32, ((mask >> 39) & 0x1FFF) as u32];
    for &sm in &s {
        if sm.count_ones() >= 5 {
            if let Some(h) = straight_high(sm) {
                return (STRAIGHT_FLUSH << 20) | (h << 16);
            }
            return (FLUSH << 20) | top_ranks(sm, 5);
        }
    }
    let all = s[0] | s[1] | s[2] | s[3];
    let quads = s[0] & s[1] & s[2] & s[3];
    if quads != 0 {
        let q = 31 - quads.leading_zeros();
        let k = 31 - (all & !(1 << q)).leading_zeros();
        return (QUADS << 20) | (q << 16) | (k << 12);
    }
    // Ranks present in at least 3 / at least 2 suits.
    let ge2 = (s[0] & s[1]) | (s[0] & s[2]) | (s[0] & s[3]) | (s[1] & s[2]) | (s[1] & s[3]) | (s[2] & s[3]);
    let ge3 = (s[0] & s[1] & s[2]) | (s[0] & s[1] & s[3]) | (s[0] & s[2] & s[3]) | (s[1] & s[2] & s[3]);
    if ge3 != 0 {
        let t = 31 - ge3.leading_zeros();
        let rest_pairs = ge2 & !(1 << t);
        if rest_pairs != 0 {
            let p = 31 - rest_pairs.leading_zeros();
            return (FULL_HOUSE << 20) | (t << 16) | (p << 12);
        }
    }
    if let Some(h) = straight_high(all) {
        return (STRAIGHT << 20) | (h << 16);
    }
    if ge3 != 0 {
        let t = 31 - ge3.leading_zeros();
        let kick = top_ranks(all & !(1 << t), 2) >> 8;
        return (TRIPS << 20) | (t << 16) | kick;
    }
    if ge2.count_ones() >= 2 {
        let p1 = 31 - ge2.leading_zeros();
        let p2 = 31 - (ge2 & !(1 << p1)).leading_zeros();
        let k = 31 - (all & !(1 << p1) & !(1 << p2)).leading_zeros();
        return (TWO_PAIR << 20) | (p1 << 16) | (p2 << 12) | (k << 8);
    }
    if ge2 != 0 {
        let p = 31 - ge2.leading_zeros();
        let kick = top_ranks(all & !(1 << p), 3) >> 4;
        return (PAIR << 20) | (p << 16) | kick;
    }
    (HIGH_CARD << 20) | top_ranks(all, 5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::{Card, mask_of, parse_cards};
    use std::collections::HashSet;

    fn ev(cards: &[&str]) -> HandValue {
        eval(mask_of(&parse_cards(cards).unwrap()))
    }

    #[test]
    fn ordering_spot_checks() {
        assert!(ev(&["Ah", "Kh", "Qh", "Jh", "Th"]) > ev(&["9h", "Kh", "Qh", "Jh", "Th"]));
        assert_eq!(category(ev(&["Ah", "2d", "3c", "4s", "5h"])), STRAIGHT);
        assert!(ev(&["Ah", "2d", "3c", "4s", "5h"]) < ev(&["6h", "2d", "3c", "4s", "5h"]));
        assert_eq!(category(ev(&["As", "2s", "3s", "4s", "5s", "Kd", "Kc"])), STRAIGHT_FLUSH);
        // Two trips make a full house using the lower trips as the pair.
        assert_eq!(ev(&["Ah", "Ad", "Ac", "Kh", "Kd", "Kc", "2s"]) >> 12, (FULL_HOUSE << 8) | (12 << 4) | 11);
        // Three pairs: best two pairs plus best kicker (which may be the third pair).
        assert!(ev(&["Ah", "Ad", "Kh", "Kd", "Qh", "Qd", "2s"]) > ev(&["Ah", "Ad", "Kh", "Kd", "Jh", "Jd", "2s"]));
        // Kicker matters for a pair.
        assert!(ev(&["Ah", "Ad", "Kh", "7d", "5c"]) > ev(&["As", "Ac", "Qh", "7d", "5c"]));
        assert_eq!(ev(&["Ah", "Ad", "Kh", "7d", "5c", "2s", "3s"]), ev(&["Ah", "Ad", "Kh", "7d", "5c"]));
    }

    fn five_card_hands(mut f: impl FnMut(CardMask)) {
        for a in 0..52u8 {
            for b in a + 1..52 {
                for c in b + 1..52 {
                    for d in c + 1..52 {
                        for e in d + 1..52 {
                            f(Card(a).bit() | Card(b).bit() | Card(c).bit() | Card(d).bit() | Card(e).bit());
                        }
                    }
                }
            }
        }
    }

    /// The window loop `straight_high` replaced (0230), kept as the reference.
    fn straight_high_loop(ranks: u32) -> Option<u32> {
        let ext = (ranks << 1) | ((ranks >> 12) & 1);
        let mut top = 13;
        while top >= 4 {
            if (ext >> (top - 4)) & 0x1F == 0x1F {
                return Some(top - 1);
            }
            top -= 1;
        }
        None
    }

    #[test]
    fn straight_detection_matches_the_window_loop_on_every_rank_set() {
        for ranks in 0..1u32 << 13 {
            assert_eq!(straight_high(ranks), straight_high_loop(ranks), "ranks {ranks:013b}");
        }
    }

    #[test]
    fn exhaustive_five_card_category_counts_and_classes() {
        let mut counts = [0u64; 9];
        let mut distinct = HashSet::new();
        five_card_hands(|m| {
            let v = eval(m);
            counts[category(v) as usize] += 1;
            distinct.insert(v);
        });
        assert_eq!(counts, [1302540, 1098240, 123552, 54912, 10200, 5108, 3744, 624, 40]);
        assert_eq!(distinct.len(), 7462);
    }

    #[test]
    #[ignore = "~134M evaluations; run with --ignored"]
    fn exhaustive_seven_card_category_counts() {
        use rayon::prelude::*;
        let counts = (0..52u8)
            .into_par_iter()
            .map(|a| {
                let mut counts = [0u64; 9];
                for b in a + 1..52 {
                    for c in b + 1..52 {
                        for d in c + 1..52 {
                            for e in d + 1..52 {
                                for f in e + 1..52 {
                                    for g in f + 1..52 {
                                        let m = (1u64 << a) | (1 << b) | (1 << c) | (1 << d) | (1 << e) | (1 << f) | (1 << g);
                                        counts[category(eval(m)) as usize] += 1;
                                    }
                                }
                            }
                        }
                    }
                }
                counts
            })
            .reduce(
                || [0u64; 9],
                |mut x, y| {
                    for i in 0..9 {
                        x[i] += y[i];
                    }
                    x
                },
            );
        assert_eq!(counts, [23294460, 58627800, 31433400, 6461620, 6180020, 4047644, 3473184, 224848, 41584]);
    }
}

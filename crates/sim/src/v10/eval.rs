//! v10.0.2's seven-card evaluator. A value is its category in bits 20 and up and its ranks below,
//! four bits each, so a higher value is a better hand and equal values split.

use super::cards::CardMask;

/// A hand's value: higher wins.
pub type HandValue = u32;

pub const HIGH_CARD: u32 = 0;
pub const PAIR: u32 = 1;
pub const TWO_PAIR: u32 = 2;
pub const TRIPS: u32 = 3;
pub const STRAIGHT: u32 = 4;
pub const FLUSH: u32 = 5;
pub const FULL_HOUSE: u32 = 6;
pub const QUADS: u32 = 7;
pub const STRAIGHT_FLUSH: u32 = 8;

/// The category of a value, [`HIGH_CARD`] to [`STRAIGHT_FLUSH`].
pub fn category(value: HandValue) -> u32 {
    value >> 20
}

/// The top rank of the best straight in a rank mask, the ace playing low as well as high.
fn straight_high(ranks: u32) -> Option<u32> {
    let ext = (ranks << 1) | ((ranks >> 12) & 1);
    let run = ext & (ext >> 1) & (ext >> 2) & (ext >> 3) & (ext >> 4);
    (run != 0).then(|| 31 - run.leading_zeros() + 3)
}

/// The `n` highest ranks of a mask, packed from bit 16 down.
fn top_ranks(mut ranks: u32, n: usize) -> u32 {
    let mut out = 0;
    for i in 0..n {
        let rank = 31 - ranks.leading_zeros();
        ranks &= !(1 << rank);
        out |= rank << (4 * (4 - i));
    }
    out
}

fn highest(ranks: u32) -> u32 {
    31 - ranks.leading_zeros()
}

/// The value of the best five of the five to seven cards in `mask`.
pub fn eval(mask: CardMask) -> HandValue {
    let s = [0, 13, 26, 39].map(|shift| ((mask >> shift) & 0x1FFF) as u32);
    for &suit in &s {
        if suit.count_ones() >= 5 {
            if let Some(high) = straight_high(suit) {
                return (STRAIGHT_FLUSH << 20) | (high << 16);
            }
            return (FLUSH << 20) | top_ranks(suit, 5);
        }
    }
    let all = s[0] | s[1] | s[2] | s[3];
    let quads = s[0] & s[1] & s[2] & s[3];
    if quads != 0 {
        let q = highest(quads);
        let kicker = highest(all & !(1 << q));
        return (QUADS << 20) | (q << 16) | (kicker << 12);
    }
    // Ranks held in at least two suits, and in at least three.
    let ge2 = (s[0] & s[1]) | (s[0] & s[2]) | (s[0] & s[3]) | (s[1] & s[2]) | (s[1] & s[3]);
    let ge2 = ge2 | (s[2] & s[3]);
    let ge3 = (s[0] & s[1] & s[2]) | (s[0] & s[1] & s[3]) | (s[0] & s[2] & s[3]);
    let ge3 = ge3 | (s[1] & s[2] & s[3]);
    if ge3 != 0 {
        let t = highest(ge3);
        let pairs = ge2 & !(1 << t);
        if pairs != 0 {
            return (FULL_HOUSE << 20) | (t << 16) | (highest(pairs) << 12);
        }
    }
    if let Some(high) = straight_high(all) {
        return (STRAIGHT << 20) | (high << 16);
    }
    if ge3 != 0 {
        let t = highest(ge3);
        let kickers = top_ranks(all & !(1 << t), 2) >> 8;
        return (TRIPS << 20) | (t << 16) | kickers;
    }
    if ge2.count_ones() >= 2 {
        let p1 = highest(ge2);
        let p2 = highest(ge2 & !(1 << p1));
        let kicker = highest(all & !(1 << p1) & !(1 << p2));
        return (TWO_PAIR << 20) | (p1 << 16) | (p2 << 12) | (kicker << 8);
    }
    if ge2 != 0 {
        let p = highest(ge2);
        let kickers = top_ranks(all & !(1 << p), 3) >> 4;
        return (PAIR << 20) | (p << 16) | kickers;
    }
    (HIGH_CARD << 20) | top_ranks(all, 5)
}

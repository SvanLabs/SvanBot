//! v10.0.2's samplers, on `core`'s generator. v10 reproduced `rand` 0.10's algorithms, and its
//! decisions are only matched draw for draw if these are too: a float from the top bits of a
//! word, a 32-bit word from the top half of a 64-bit one, and a range by the widening multiply
//! with one correction draw.

use svanbot_core::rng::Rng;

/// A float in `[0, 1)` from the top 53 bits of a word.
pub fn f64(rng: &mut Rng) -> f64 {
    (rng.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

/// A float in `[0, 1)` from the top 24 bits of a 32-bit word.
pub fn f32(rng: &mut Rng) -> f32 {
    (u32(rng) >> 8) as f32 * (1.0 / (1u32 << 24) as f32)
}

/// A 32-bit word: the upper half of a 64-bit one.
pub fn u32(rng: &mut Rng) -> u32 {
    (rng.next_u64() >> 32) as u32
}

/// A 64-bit word.
pub fn u64(rng: &mut Rng) -> u64 {
    rng.next_u64()
}

/// A number from `low` to `high` inclusive, drawn on 32 bits.
pub fn inclusive_u32(low: u32, high: u32, rng: &mut Rng) -> u32 {
    assert!(low <= high, "cannot sample an empty range");
    let range = high.wrapping_sub(low).wrapping_add(1);
    if range == 0 {
        return u32(rng);
    }
    let wide = u64::from(u32(rng)) * u64::from(range);
    let (mut result, lo) = ((wide >> 32) as u32, wide as u32);
    if lo > range.wrapping_neg() {
        let hi = ((u64::from(u32(rng)) * u64::from(range)) >> 32) as u32;
        result += u32::from(lo.checked_add(hi).is_none());
    }
    low.wrapping_add(result)
}

/// A number below `n`, as v10's `random_range(0..n)` drew it on 32 bits: a card number, a
/// position in a list.
pub fn below(n: u32, rng: &mut Rng) -> u32 {
    assert!(n > 0, "cannot sample an empty range");
    inclusive_u32(0, n - 1, rng)
}

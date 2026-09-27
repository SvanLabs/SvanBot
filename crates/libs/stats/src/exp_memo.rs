//! An exact memo for `f32::exp` (0353).
//!
//! The range model's logistic kernels call `exp` millions of times per suite with arguments drawn
//! from a small set: `equity_vs_histogram` evaluates the same 1,600 arguments on every call, and
//! `continue_by_order` evaluates 1,180 on average of which only about 120 are distinct. A memo is
//! the one way to remove that libm traffic without changing a single result: a hit returns bits
//! that `f32::exp` itself produced for the same argument, so a memoized path and the plain
//! expression agree bit for bit — there is no tolerance to argue about. (An approximating kernel
//! would not do: these values feed decision EVs.)

#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

/// Slots in an [`ExpMemo`]. The two call sites see under 210 distinct arguments per call (measured
/// over a 120-hand suite: 202 for `equity_vs_histogram`, 120 for `continue_by_order`), so 512 slots
/// with a multiplicative hash hold a call's working set with little collision pressure, and the
/// 4 KiB key fill costs less than a handful of `exp` calls.
const SLOTS: usize = 512;

/// Marks an empty slot. `0xffff_ffff` is the bit pattern of one negative quiet NaN, which
/// [`ExpMemo::exp`] sends straight to `f32::exp` (together with every other NaN), so the marker
/// can never be mistaken for a key.
const EMPTY: u64 = u32::MAX as u64;

/// `f32::exp` calls a test observed, so its tests can tell a hit from a miss.
#[cfg(test)]
pub(crate) static EXP_CALLS: AtomicU64 = AtomicU64::new(0);

/// `f32::exp` with this call's arguments remembered, bit for bit.
///
/// [`ExpMemo::new`] is a 4 KiB key fill, so building one per call is cheap against the `exp` calls
/// it removes. A memo never answers with a value that was not produced by `f32::exp` for the very
/// same argument bits, so results are identical whether a lookup hits or misses.
pub struct ExpMemo {
    /// `argument bits | value bits << 32` per slot: one load per lookup.
    entries: [u64; SLOTS],
}

impl Default for ExpMemo {
    fn default() -> Self {
        Self::new()
    }
}

impl ExpMemo {
    /// An empty memo.
    pub fn new() -> Self {
        Self { entries: [EMPTY; SLOTS] }
    }

    /// Exactly `x.exp()`.
    #[inline]
    pub fn exp(&mut self, x: f32) -> f32 {
        let bits = x.to_bits();
        if bits == EMPTY as u32 {
            return compute(x);
        }
        let i = slot(bits);
        match self.hit(i, bits) {
            Some(v) => v,
            None => {
                let v = compute(x);
                self.entries[i] = bits as u64 | ((v.to_bits() as u64) << 32);
                v
            }
        }
    }

    /// The value stored for `bits` in slot `i`, if this exact key is there.
    #[inline]
    fn hit(&self, i: usize, bits: u32) -> Option<f32> {
        let e = self.entries[i];
        ((e as u32) == bits).then(|| f32::from_bits((e >> 32) as u32))
    }

    /// The value [`ExpMemo::exp`] would return for `x` without calling `f32::exp`: `Some` for a
    /// key the memo holds (so the next call hits), `None` for a miss.
    #[cfg(test)]
    pub(crate) fn probe(&self, x: f32) -> Option<f32> {
        let bits = x.to_bits();
        if bits == EMPTY as u32 { None } else { self.hit(slot(bits), bits) }
    }
}

/// `f32::exp`, counted in test builds.
#[inline]
fn compute(x: f32) -> f32 {
    #[cfg(test)]
    EXP_CALLS.fetch_add(1, Ordering::Relaxed);
    x.exp()
}

/// Multiplicative (Fibonacci) hash of the argument bits: the high bits of the 32-bit product index
/// the table, so the exponent and the mantissa both spread the key over the slots.
#[inline]
fn slot(bits: u32) -> usize {
    (bits.wrapping_mul(0x9E37_79B1) >> (32 - SLOTS.trailing_zeros())) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The tests below count `exp` calls, so they must not run at the same time as each other.
    static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

    /// Every exponent with a spread of mantissas, both signs — the whole range `exp` is defined
    /// over, including the denormal tail, the overflow tail and subnormal results — plus the
    /// special values.
    fn sweep() -> impl Iterator<Item = f32> {
        (0u32..=255)
            .flat_map(|e| (0u32..2048).map(move |m| (e << 23) | (m << 12)))
            .flat_map(|bits| [f32::from_bits(bits), f32::from_bits(bits | 0x8000_0000)])
            .chain([
                0.0,
                -0.0,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::MAX,
                f32::MIN,
                f32::MIN_POSITIVE,
                -f32::MIN_POSITIVE,
                f32::from_bits(1),
                f32::from_bits(0x8000_0001),
                f32::NAN,
                f32::from_bits(0xffff_ffff),
                f32::from_bits(0xffc0_0000),
                f32::from_bits(0x7f80_0001),
            ])
    }

    #[test]
    fn every_argument_returns_exactly_what_exp_returns() {
        let _guard = ONE_AT_A_TIME.lock().unwrap();
        // One memo across the whole sweep, so hits and evictions are both exercised.
        let mut memo = ExpMemo::new();
        let mut seen = 0u64;
        for x in sweep() {
            let got = memo.exp(x);
            let want = x.exp();
            assert_eq!(got.to_bits(), want.to_bits(), "exp({x:e}) = {got:e}, not {want:e}");
            seen += 1;
        }
        assert!(seen > 1_000_000, "sweep too small to mean anything: {seen}");
    }

    #[test]
    fn a_repeated_argument_hits_and_recomputes_nothing() {
        let _guard = ONE_AT_A_TIME.lock().unwrap();
        let mut memo = ExpMemo::new();
        let args: Vec<f32> = sweep().step_by(9973).take(SLOTS / 2).collect();
        for &x in &args {
            assert_eq!(memo.exp(x).to_bits(), x.exp().to_bits());
            assert!(memo.probe(x).is_some(), "the key must be stored for the next call to hit");
        }
        let before = EXP_CALLS.load(Ordering::Relaxed);
        for _ in 0..3 {
            for &x in &args {
                assert_eq!(memo.exp(x).to_bits(), x.exp().to_bits());
            }
        }
        assert_eq!(EXP_CALLS.load(Ordering::Relaxed), before, "a hit must not call exp");
        // A key the memo does not hold must not be answered from some other slot.
        let absent = f32::from_bits(0x3f00_0001);
        if !args.contains(&absent) {
            assert!(memo.probe(absent).is_none());
        }
    }

    #[test]
    fn evictions_and_collisions_never_corrupt_a_result() {
        let _guard = ONE_AT_A_TIME.lock().unwrap();
        // Far more distinct keys than slots: a lookup either finds its own key or recomputes.
        let mut memo = ExpMemo::new();
        let (mut state, mut calls) = (12345u32, 0u32);
        while calls < 40 * SLOTS as u32 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = (state >> 8) as f32 / (1u32 << 24) as f32 * 200.0 - 100.0;
            assert_eq!(memo.exp(x).to_bits(), x.exp().to_bits(), "exp({x:e})");
            calls += 1;
        }
    }
}

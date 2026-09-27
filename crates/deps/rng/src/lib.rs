//! Seedable random numbers for svanbot10, with no dependencies.
//!
//! [`rngs::SmallRng`] is xoshiro256++ seeded through SplitMix64, and every sampler reproduces the
//! algorithm `rand` 0.10 uses on 64-bit targets (its default features): floats from the top bits
//! (53 for `f64`, 24 for `f32`), `bool` from the top bit of a `u32`, integer ranges by the biased
//! Canon/Lemire widening multiply with one correction sample, `usize` ranges on 32 bits when they
//! fit, and `shuffle` through chunked "increasing uniform" indices. Seeded simulations, the golden
//! decision snapshot and the paired-sim guard therefore stay bit-identical to the `rand` era; the
//! reference vectors in `tests/reference.rs` were recorded from `rand` 0.10.2 itself.
//!
//! The names mirror `rand` (`Rng`, `RngExt`, `SeedableRng`, `rngs::SmallRng`, `seq::SliceRandom`)
//! so call sites read the same. Not for cryptography.

#![warn(missing_docs)]

use std::ops::{Range, RangeInclusive, RangeTo};

/// A source of random 64-bit words.
pub trait Rng {
    /// The next 64 random bits.
    fn next_u64(&mut self) -> u64;
    /// The next 32 random bits: the upper half of a 64-bit word (the low bits of xoshiro are weaker).
    fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }
}

impl<R: Rng + ?Sized> Rng for &mut R {
    fn next_u64(&mut self) -> u64 {
        (**self).next_u64()
    }
    fn next_u32(&mut self) -> u32 {
        (**self).next_u32()
    }
}

/// Generators built from a 64-bit seed.
pub trait SeedableRng: Sized {
    /// A generator whose whole output is determined by `seed`.
    fn seed_from_u64(seed: u64) -> Self;
}

/// Types with a standard uniform distribution (`[0, 1)` for floats, all values for integers).
pub trait Standard: Sized {
    /// Draw one value.
    fn sample<R: Rng + ?Sized>(rng: &mut R) -> Self;
}

impl Standard for u64 {
    fn sample<R: Rng + ?Sized>(rng: &mut R) -> u64 {
        rng.next_u64()
    }
}

impl Standard for u32 {
    fn sample<R: Rng + ?Sized>(rng: &mut R) -> u32 {
        rng.next_u32()
    }
}

impl Standard for u8 {
    fn sample<R: Rng + ?Sized>(rng: &mut R) -> u8 {
        rng.next_u32() as u8
    }
}

impl Standard for bool {
    fn sample<R: Rng + ?Sized>(rng: &mut R) -> bool {
        (rng.next_u32() as i32) < 0
    }
}

impl Standard for f64 {
    fn sample<R: Rng + ?Sized>(rng: &mut R) -> f64 {
        (rng.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }
}

impl Standard for f32 {
    fn sample<R: Rng + ?Sized>(rng: &mut R) -> f32 {
        (rng.next_u32() >> 8) as f32 * (1.0 / (1u32 << 24) as f32)
    }
}

impl<T: Standard, const N: usize> Standard for [T; N] {
    fn sample<R: Rng + ?Sized>(rng: &mut R) -> [T; N] {
        std::array::from_fn(|_| T::sample(rng))
    }
}

/// Ranges an integer can be drawn from uniformly.
pub trait SampleRange<T> {
    /// Draw one value; panics on an empty range.
    fn sample<R: Rng + ?Sized>(self, rng: &mut R) -> T;
}

/// Uniform `[low, high]` over a sample word of `$s` bits (`rand`'s `UniformInt::sample_single_inclusive`).
macro_rules! inclusive {
    ($name:ident, $ty:ty, $uty:ty, $s:ty, $wide:ty, $bits:expr, $draw:ident) => {
        fn $name<R: Rng + ?Sized>(low: $ty, high: $ty, rng: &mut R) -> $ty {
            assert!(low <= high, "cannot sample empty range");
            let range = high.wrapping_sub(low).wrapping_add(1) as $uty as $s;
            if range == 0 {
                return rng.$draw() as $ty;
            }
            let wide = rng.$draw() as $wide * range as $wide;
            let (mut result, lo) = ((wide >> $bits) as $s, wide as $s);
            if lo > range.wrapping_neg() {
                let hi = ((rng.$draw() as $wide * range as $wide) >> $bits) as $s;
                result += lo.checked_add(hi).is_none() as $s;
            }
            low.wrapping_add(result as $ty)
        }
    };
}

inclusive!(incl_u8, u8, u8, u32, u64, 32, next_u32);
inclusive!(incl_i32, i32, u32, u32, u64, 32, next_u32);
inclusive!(incl_u32, u32, u32, u32, u64, 32, next_u32);
inclusive!(incl_i64, i64, u64, u64, u128, 64, next_u64);
inclusive!(incl_u64, u64, u64, u64, u128, 64, next_u64);

fn incl_usize<R: Rng + ?Sized>(low: usize, high: usize, rng: &mut R) -> usize {
    if high > u32::MAX as usize { incl_u64(low as u64, high as u64, rng) as usize } else { incl_u32(low as u32, high as u32, rng) as usize }
}

macro_rules! ranges {
    ($ty:ty, $incl:ident) => {
        impl SampleRange<$ty> for Range<$ty> {
            fn sample<R: Rng + ?Sized>(self, rng: &mut R) -> $ty {
                assert!(self.start < self.end, "cannot sample empty range");
                $incl(self.start, self.end - 1, rng)
            }
        }
        impl SampleRange<$ty> for RangeInclusive<$ty> {
            fn sample<R: Rng + ?Sized>(self, rng: &mut R) -> $ty {
                $incl(*self.start(), *self.end(), rng)
            }
        }
    };
}

ranges!(u8, incl_u8);
ranges!(i32, incl_i32);
ranges!(u32, incl_u32);
ranges!(i64, incl_i64);
ranges!(u64, incl_u64);

impl SampleRange<usize> for Range<usize> {
    fn sample<R: Rng + ?Sized>(self, rng: &mut R) -> usize {
        assert!(self.start < self.end, "cannot sample empty range");
        // rand samples an exclusive usize range as u64 when `end` (not `end - 1`) exceeds u32.
        if self.end > u32::MAX as usize {
            incl_u64(self.start as u64, self.end as u64 - 1, rng) as usize
        } else {
            incl_u32(self.start as u32, self.end as u32 - 1, rng) as usize
        }
    }
}

impl SampleRange<usize> for RangeInclusive<usize> {
    fn sample<R: Rng + ?Sized>(self, rng: &mut R) -> usize {
        incl_usize(*self.start(), *self.end(), rng)
    }
}

impl SampleRange<u32> for RangeTo<u32> {
    fn sample<R: Rng + ?Sized>(self, rng: &mut R) -> u32 {
        (0..self.end).sample(rng)
    }
}

/// Sampling helpers on every [`Rng`].
pub trait RngExt: Rng {
    /// A value of `T`'s standard distribution.
    fn random<T: Standard>(&mut self) -> T {
        T::sample(self)
    }
    /// A uniform value in `range`; panics on an empty range.
    fn random_range<T, S: SampleRange<T>>(&mut self, range: S) -> T {
        range.sample(self)
    }
    /// `true` with probability `p` (panics outside `[0, 1]`).
    fn random_bool(&mut self, p: f64) -> bool {
        assert!((0.0..=1.0).contains(&p), "p={p:?} is outside range [0.0, 1.0]");
        if p >= 1.0 {
            return true;
        }
        let p_int = (p * (2.0 * (1u64 << 63) as f64)) as u64;
        self.next_u64() < p_int
    }
}

impl<R: Rng + ?Sized> RngExt for R {}

/// Generators.
pub mod rngs {
    use super::{Rng, SeedableRng};

    /// xoshiro256++: small, fast, statistically strong, not cryptographic.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct SmallRng {
        s: [u64; 4],
    }

    impl SeedableRng for SmallRng {
        fn seed_from_u64(mut state: u64) -> SmallRng {
            let mut s = [0u64; 4];
            for w in s.iter_mut() {
                state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut z = state;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                *w = z ^ (z >> 31);
            }
            SmallRng { s }
        }
    }

    impl SmallRng {
        /// A generator seeded from the operating system (`/dev/urandom`), falling back to the clock
        /// and process id where that is unavailable.
        pub fn from_os() -> SmallRng {
            SmallRng::seed_from_u64(super::os_u64())
        }
    }

    impl Rng for SmallRng {
        #[inline]
        fn next_u64(&mut self) -> u64 {
            let s = &mut self.s;
            let result = s[0].wrapping_add(s[3]).rotate_left(23).wrapping_add(s[0]);
            let t = s[1] << 17;
            s[2] ^= s[0];
            s[3] ^= s[1];
            s[1] ^= s[2];
            s[0] ^= s[3];
            s[2] ^= t;
            s[3] = s[3].rotate_left(45);
            result
        }
    }
}

/// Slice helpers.
pub mod seq {
    use super::{Rng, RngExt};

    /// Random permutations of slices.
    pub trait SliceRandom {
        /// Shuffle in place (Fisher–Yates, `rand`'s index order).
        fn shuffle<R: Rng + ?Sized>(&mut self, rng: &mut R);
    }

    impl<T> SliceRandom for [T] {
        fn shuffle<R: Rng + ?Sized>(&mut self, rng: &mut R) {
            if self.len() <= 1 {
                return;
            }
            if self.len() < u32::MAX as usize {
                let mut chooser = IncreasingUniform { n: 0, chunk: 0, chunk_remaining: 1 };
                for i in 0..self.len() {
                    let j = chooser.next_index(rng);
                    self.swap(i, j);
                }
            } else {
                for i in 0..self.len() {
                    let j = rng.random_range(0..=i);
                    self.swap(i, j);
                }
            }
        }
    }

    /// Indices in `[0, n]` for increasing `n`, many per random word (`rand`'s `IncreasingUniform`).
    struct IncreasingUniform {
        n: u32,
        chunk: u32,
        chunk_remaining: u8,
    }

    impl IncreasingUniform {
        fn next_index<R: Rng + ?Sized>(&mut self, rng: &mut R) -> usize {
            let next_n = self.n + 1;
            let next_remaining = match self.chunk_remaining.checked_sub(1) {
                Some(r) => r,
                None => {
                    let (bound, remaining) = bound_u32(next_n);
                    self.chunk = rng.random_range(..bound);
                    remaining - 1
                }
            };
            let result = if next_remaining == 0 {
                self.chunk as usize
            } else {
                let r = self.chunk % next_n;
                self.chunk /= next_n;
                r as usize
            };
            self.chunk_remaining = next_remaining;
            self.n = next_n;
            result
        }
    }

    /// `(m·(m+1)·…, count)`: the longest product of consecutive integers from `m` that fits a u32.
    fn bound_u32(m: u32) -> (u32, u8) {
        let (mut product, mut current) = (m, m + 1);
        loop {
            match product.checked_mul(current) {
                Some(p) => {
                    product = p;
                    current += 1;
                }
                None => return (product, (current - m) as u8),
            }
        }
    }
}

/// 64 bits from the operating system's entropy pool (`/dev/urandom`), or a clock/pid mix.
pub fn os_u64() -> u64 {
    use std::io::Read;
    let mut b = [0u8; 8];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_ok() {
        return u64::from_le_bytes(b);
    }
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    t ^ ((std::process::id() as u64) << 32)
}

/// Random bytes from the operating system (`/dev/urandom`); falls back to a seeded generator.
pub fn os_bytes<const N: usize>() -> [u8; N] {
    use std::io::Read;
    let mut b = [0u8; N];
    if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut b)).is_ok() {
        return b;
    }
    let mut rng = rngs::SmallRng::from_os();
    rng.random::<[u8; N]>()
}

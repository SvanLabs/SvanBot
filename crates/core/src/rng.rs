//! The seeded generator every pure layer draws from: xoshiro256++ (<https://prng.di.unimi.it/>),
//! with its state filled by SplitMix64, as its authors recommend. The same seed gives the same
//! draws on any machine. Fresh entropy for a seed comes from `runtime`.

/// A xoshiro256++ generator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rng {
    state: [u64; 4],
}

/// SplitMix64's increment: the golden ratio, as a 64-bit fraction.
const GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;

/// SplitMix64's output for its `n`th state after `seed`, counting from 1. SplitMix64 is a counter,
/// so any output can be read without reading the ones before it.
const fn splitmix64(seed: u64, n: u64) -> u64 {
    let mut z = seed.wrapping_add(GAMMA.wrapping_mul(n));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl Rng {
    /// The generator for `seed`.
    pub const fn seeded(seed: u64) -> Self {
        Self::stream(seed, 0)
    }

    /// The generator for block `index` of `seed`. Work cut into blocks draws each block from its
    /// own stream, so the result does not depend on how many threads run the blocks or in what
    /// order. Block 0 is [`Rng::seeded`]; block `i` takes SplitMix64's outputs `4i + 1` to `4i + 4`.
    pub const fn stream(seed: u64, index: u64) -> Self {
        let first = index.wrapping_mul(4);
        Self {
            state: [
                splitmix64(seed, first.wrapping_add(1)),
                splitmix64(seed, first.wrapping_add(2)),
                splitmix64(seed, first.wrapping_add(3)),
                splitmix64(seed, first.wrapping_add(4)),
            ],
        }
    }

    /// The generator with this exact state, for example one saved earlier. The state must not be
    /// all zeros: xoshiro never leaves that state.
    pub const fn from_state(state: [u64; 4]) -> Self {
        Self { state }
    }

    /// The next 64 random bits.
    pub const fn next_u64(&mut self) -> u64 {
        let s = &mut self.state;
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

    /// A number from `0` to `n - 1`, each equally likely. `n` must not be 0. Lemire's method:
    /// multiply, and draw again only in the rare case that would favour some values.
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0, "below(0) has no value to draw");
        let threshold = n.wrapping_neg() % n;
        loop {
            let product = u128::from(self.next_u64()) * u128::from(n);
            if (product as u64) >= threshold {
                return (product >> 64) as u64;
            }
        }
    }
}

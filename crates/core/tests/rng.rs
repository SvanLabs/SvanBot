//! The seeded generator: xoshiro256++ (<https://prng.di.unimi.it/>), seeded through SplitMix64.

use svanbot_core::rng::Rng;

#[test]
fn it_matches_the_reference_implementation() {
    // The outputs of the reference C code from the state [1, 2, 3, 4], as published with the
    // `rand_xoshiro` crate's tests. The first two are checked by hand: rotl(1 + 4, 23) + 1, and
    // so on.
    let mut rng = Rng::from_state([1, 2, 3, 4]);
    let expected: [u64; 10] = [
        41_943_041,
        58_720_359,
        3_588_806_011_781_223,
        3_591_011_842_654_386,
        9_228_616_714_210_784_205,
        9_973_669_472_204_895_162,
        14_011_001_112_246_962_877,
        12_406_186_145_184_390_807,
        15_849_039_046_786_891_736,
        10_450_023_813_501_588_000,
    ];
    for want in expected {
        assert_eq!(rng.next_u64(), want);
    }
}

#[test]
fn a_seed_fills_the_state_from_splitmix64() {
    // SplitMix64's first four outputs from 0, the published vector.
    let state = [
        0xe220_a839_7b1d_cdaf,
        0x6e78_9e6a_a1b9_65f4,
        0x06c4_5d18_8009_454f,
        0xf88b_b8a8_724c_81ec,
    ];
    let (mut seeded, mut reference) = (Rng::seeded(0), Rng::from_state(state));
    for _ in 0..4 {
        assert_eq!(seeded.next_u64(), reference.next_u64());
    }
}

#[test]
fn each_block_of_a_seed_has_its_own_stream() {
    let first = |mut rng: Rng| (0..4).map(|_| rng.next_u64()).collect::<Vec<_>>();
    assert_eq!(first(Rng::stream(7, 0)), first(Rng::seeded(7)));
    assert_eq!(first(Rng::stream(7, 3)), first(Rng::stream(7, 3)));
    assert_ne!(first(Rng::stream(7, 3)), first(Rng::stream(7, 4)));
    assert_ne!(first(Rng::stream(7, 3)), first(Rng::stream(8, 3)));
}

#[test]
fn below_draws_each_value_evenly() {
    let mut rng = Rng::seeded(2026);
    let mut counts = [0u32; 6];
    for _ in 0..60_000 {
        counts[rng.below(6) as usize] += 1;
    }
    // 10,000 expected for each, with a standard deviation of about 91.
    for count in counts {
        assert!((9_550..=10_450).contains(&count), "{counts:?}");
    }
    for n in [1, 2, 3, 52, 1 << 40, u64::MAX] {
        assert!((0..1000).all(|_| rng.below(n) < n));
    }
}

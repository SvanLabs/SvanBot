//! Every optimization of the shared deals reproduces the frozen reference bit for bit (0335): the
//! same deals, ranks, priors and random-stream position, and the same equity bits for every subset.

use super::SharedDeals;
use super::reference::Reference;
use sv10_cards::cards::Card;
use sv10_cards::range::Range;
use sv10_rng::rngs::SmallRng;
use sv10_rng::{Rng, RngExt, SeedableRng};

/// A range with a mix of zero, tiny and heavy weights; `density` of the combos are live.
fn range(rng: &mut SmallRng, density: f64) -> Range {
    let mut r = Range::empty();
    for w in r.w.iter_mut() {
        let u: f64 = rng.random();
        *w = if u > density {
            0.0
        } else if u < density * 0.3 {
            1e-4 * rng.random::<f32>()
        } else {
            rng.random::<f32>() * 3.0
        };
    }
    r
}

/// Hero and board cards distinct, from `rng`.
fn cards(rng: &mut SmallRng, n: usize) -> Vec<Card> {
    let mut used = 0u64;
    let mut out = Vec::new();
    while out.len() < n {
        let c = rng.random_range(0..52u8);
        if used & (1u64 << c) == 0 {
            used |= 1u64 << c;
            out.push(Card(c));
        }
    }
    out
}

fn same(a: &SharedDeals, b: &Reference, what: &str) {
    assert_eq!(a.k, b.k, "{what}: k");
    assert_eq!(a.combos, b.combos, "{what}: combos");
    assert_eq!(a.hero_rank, b.hero_rank, "{what}: hero ranks");
    assert_eq!(a.opp_rank, b.opp_rank, "{what}: opponent ranks");
    assert_eq!(a.full, b.full, "{what}: full ranges");
    assert_eq!(a.prior, b.prior, "{what}: priors");
}

#[test]
fn optimized_deals_reproduce_the_reference_bit_for_bit() {
    let mut meta = SmallRng::seed_from_u64(20260927);
    let mut exact_seen = 0;
    for case in 0..160 {
        let board_len = [0, 3, 4, 5][case % 4];
        let k = 1 + (case / 4) % 3;
        let dealt = cards(&mut meta, 2 + board_len);
        let (hero, board) = ([dealt[0], dealt[1]], &dealt[2..]);
        // Narrow ranges make heads-up enumeration fit; an empty range exercises the full fallback.
        let density = [0.02, 0.1, 0.5, 1.0][(case / 12) % 4];
        let ranges: Vec<Range> =
            (0..k).map(|j| if case % 17 == 5 && j == 0 { Range::empty() } else { range(&mut meta, density) }).collect();
        let refs: Vec<&Range> = ranges.iter().collect();
        let samples = [500, 3_000, 60_000][case % 3];
        let chunks = [1, 3][(case / 3) % 2];
        let seed = meta.next_u64();
        let (mut ra, mut rb) = (SmallRng::seed_from_u64(seed), SmallRng::seed_from_u64(seed));
        let a = SharedDeals::new_parallel(hero, board, &refs, samples, chunks, &mut ra);
        let b = Reference::new_parallel(hero, board, &refs, samples, chunks, &mut rb);
        let what = format!("case {case}: board {board_len}, {k} opponents, {samples} samples, {chunks} chunks");
        same(&a, &b, &what);
        assert_eq!(ra.next_u64(), rb.next_u64(), "{what}: random stream position");
        exact_seen += usize::from(a.is_exact());
        // Every subset of opponents, each either full or narrowed (sometimes to nothing).
        let narrowed: Vec<Range> = (0..k)
            .map(|_| {
                let density = [0.0, 0.05, 0.4, 0.9][meta.random_range(0..4usize)];
                range(&mut meta, density)
            })
            .collect();
        for mask in 1..(1u32 << k) {
            for narrow_mask in 0..(1u32 << k) {
                let subset: Vec<(usize, Option<&Range>)> =
                    (0..k).filter(|j| mask & (1 << j) != 0).map(|j| (j, (narrow_mask & (1 << j) != 0).then_some(&narrowed[j]))).collect();
                let (x, y) = (a.equity(&subset), b.equity(&subset));
                assert_eq!(x.map(f64::to_bits), y.map(f64::to_bits), "{what}: subset {mask:b} narrowed {narrow_mask:b}");
            }
        }
    }
    assert!(exact_seen >= 10, "the exact enumeration must be exercised ({exact_seen} cases)");
}

/// The Monte Carlo path rejects a deal when two opponents' combos share a card, and gives up after
/// 20 attempts per sample — so a table with enough opponents returns *fewer deals than it was asked
/// for*, silently, which is a quieter measurement rather than a wrong one (#383). Measured, the
/// boundary sits between 7 and 8 opponents: at k <= 7 every configuration tried returned every
/// sample, and at 8 a narrow range can return 1510 of 2500.
///
/// This project's table is six-max, so hero plus five opponents is the shape that has to hold, and
/// it holds with margin. Pinning it here is what makes a change that eats the margin visible: the
/// day the deals come back short at a table we actually play, this fails instead of the equity
/// quietly getting noisier.
#[test]
fn a_six_max_table_gets_every_sample_it_asked_for() {
    let samples = 2_500;
    for seed in 0..24u64 {
        let mut meta = SmallRng::seed_from_u64(seed * 7919 + 11);
        // A narrow range is the harder case: two opponents both concentrated on the same cards
        // collide more often than two full ranges, so the density sweep runs to the sparse end.
        for density in [0.02, 0.1, 0.5, 1.0] {
            for board_len in [0, 3, 5] {
                let dealt = cards(&mut meta, 2 + board_len);
                let (hero, board) = ([dealt[0], dealt[1]], &dealt[2..]);
                let ranges: Vec<Range> = (0..5).map(|_| range(&mut meta, density)).collect();
                let refs: Vec<&Range> = ranges.iter().collect();
                let d = SharedDeals::new(hero, board, &refs, samples, &mut SmallRng::seed_from_u64(seed));
                assert_eq!(
                    d.hero_rank.len(),
                    samples,
                    "seed {seed}, density {density}, board {board_len}: a six-max deal returned {} of {samples}",
                    d.hero_rank.len()
                );
                assert!(!d.is_exact(), "a five-opponent deal is never the exact enumeration");
            }
        }
    }
}

/// The attempt budget is what decides whether a full-ring table gets a full sample: sparse ranges
/// overlap, so the rejection loop spends far more attempts per accepted deal than a six-max table
/// does. Against the previous budget of 20, this nine-handed configuration returned 2053 of 2500
/// (#388 measured eight opponents at 1758 and ten at 299). Pinning it here is what stops the budget
/// from going quietly back down.
#[test]
fn a_full_ring_table_gets_every_sample_it_asked_for() {
    let samples = 2_500;
    let mut meta = SmallRng::seed_from_u64(12_644);
    let dealt = cards(&mut meta, 5);
    let (hero, board) = ([dealt[0], dealt[1]], &dealt[2..]);
    let ranges: Vec<Range> = (0..9).map(|_| range(&mut meta, 0.02)).collect();
    let refs: Vec<&Range> = ranges.iter().collect();
    let d = SharedDeals::new(hero, board, &refs, samples, &mut SmallRng::seed_from_u64(7));
    assert_eq!(d.hero_rank.len(), samples, "a nine-handed deal returned {} of {samples}", d.hero_rank.len());
    assert!(!d.is_exact(), "a nine-handed deal is never the exact enumeration");
}

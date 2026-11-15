//! The drift monitor: an anytime-valid test that the PITs of shown pairs stay uniform.

use svanbot_core::card::CardSet;
use svanbot_core::rng::Rng;
use svanbot_learn::drift::{BINS, Drift, pit};

fn uniform(rng: &mut Rng) -> f64 {
    rng.below(1 << 30) as f64 / f64::from(1u32 << 30)
}

#[test]
fn a_calibrated_stream_never_alarms_and_a_skewed_one_soon_does() {
    let mut rng = Rng::seeded(7);
    let mut calm = Drift::new(0.01);
    for _ in 0..5_000 {
        assert!(!calm.observe(uniform(&mut rng)), "{:?}", calm.histogram());
    }
    assert_eq!(calm.histogram().iter().sum::<u64>(), 5_000);
    assert!(calm.histogram().iter().all(|&n| n > 400));
    // A range too weak: the shown pairs sit at its top.
    let mut skewed = Drift::new(0.01);
    let alarmed_at = (1..=1_000).find(|_| skewed.observe(uniform(&mut rng).sqrt()));
    assert!(alarmed_at.is_some_and(|n| n < 300), "{alarmed_at:?}");
    assert!(skewed.e_value() >= 100.0);
}

#[test]
fn a_u_shaped_stream_with_mean_one_half_still_alarms() {
    let mut rng = Rng::seeded(8);
    let mut drift = Drift::new(0.01);
    let mut sum = 0.0;
    let mut alarmed = false;
    for i in 0..2_000 {
        let u = uniform(&mut rng) * 0.1;
        let x = if i % 2 == 0 { u } else { 1.0 - u };
        sum += x;
        alarmed |= drift.observe(x);
    }
    assert!((sum / 2_000.0 - 0.5).abs() < 0.01);
    assert!(alarmed);
    let histogram = drift.histogram();
    assert_eq!(histogram[0] + histogram[BINS - 1], 2_000);
}

#[test]
fn the_pit_is_the_share_of_the_range_weaker_than_the_shown_pair() {
    let board: CardSet = "AhKhQh7c2d".parse().unwrap();
    let range = |pairs: &[&str]| -> Vec<(CardSet, f64)> {
        pairs.iter().map(|p| (p.parse().unwrap(), 1.0)).collect()
    };
    let shown: CardSet = "AsAd".parse().unwrap();
    // Two pairs weaker, one (a royal flush) stronger.
    let mixed = range(&["9c8c", "5c4s", "JhTh"]);
    assert!((pit(&mixed, shown, board, 0.0) - 2.0 / 3.0).abs() < 1e-12);
    // A tie shares out by `u`.
    let tied = range(&["AsAd", "9c8c"]);
    assert!((pit(&tied, shown, board, 0.0) - 0.5).abs() < 1e-12);
    assert!((pit(&tied, shown, board, 1.0) - 1.0).abs() < 1e-12);
}

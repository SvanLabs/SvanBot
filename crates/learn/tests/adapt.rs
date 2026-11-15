//! The adaptivity class and the exploit gate.

use svanbot_core::rng::Rng;
use svanbot_learn::adapt::{CHANGE_THRESHOLD, MIN_SIDE, change_point, classify};
use svanbot_learn::exploit::gate;
use svanbot_strategy::read::Class;

fn bernoulli(rng: &mut Rng, p: f64) -> bool {
    (rng.below(1 << 30) as f64 / f64::from(1u32 << 30)) < p
}

fn series(rng: &mut Rng, rates: &[(f64, usize)]) -> Vec<bool> {
    rates
        .iter()
        .flat_map(|&(p, n)| (0..n).map(|_| p).collect::<Vec<_>>())
        .map(|p| bernoulli(rng, p))
        .collect()
}

#[test]
fn a_steady_player_is_fixed_and_a_switch_is_found_near_where_it_happened() {
    let mut rng = Rng::seeded(11);
    let steady = series(&mut rng, &[(0.3, 600)]);
    let others = series(&mut rng, &[(0.5, 300)]);
    let vs_us = series(&mut rng, &[(0.5, 300)]);
    assert_eq!(classify(&steady, &vs_us, &others), (Class::Fixed, None));
    let switched = series(&mut rng, &[(0.2, 300), (0.6, 300)]);
    let (class, change) = classify(&switched, &vs_us, &others);
    assert_eq!(class, Class::Switching);
    let at = change.unwrap().at;
    assert!((250..350).contains(&at), "{at}");
    assert!(change_point(&switched[..2 * MIN_SIDE - 1], CHANGE_THRESHOLD, MIN_SIDE).is_none());
}

#[test]
fn a_player_who_learns_to_fold_to_our_bets_is_learning() {
    let mut rng = Rng::seeded(12);
    let tallies = series(&mut rng, &[(0.3, 600)]);
    let others = series(&mut rng, &[(0.4, 400)]);
    // Against us they fold more and more.
    let vs_us = series(&mut rng, &[(0.5, 150), (0.7, 150), (0.85, 150)]);
    assert_eq!(classify(&tallies, &vs_us, &others).0, Class::Learning);
    // The same gap with no trend is not learning: it is just how they play us.
    let flat = series(&mut rng, &[(0.4, 450)]);
    assert_ne!(classify(&tallies, &flat, &others).0, Class::Learning);
}

#[test]
fn reads_deploy_only_when_their_loss_is_surely_under_the_cap() {
    let mut rng = Rng::seeded(13);
    let mut noisy = |mean: f64| -> Vec<f64> {
        (0..3_000)
            .map(|_| mean + (rng.below(2001) as f64 - 1000.0) / 10.0)
            .collect()
    };
    let gain = gate(&noisy(-5.0), 0.0);
    assert!(gain.passed && gain.upper < 0.0, "{gain:?}");
    let loss = gate(&noisy(5.0), 0.0);
    assert!(!loss.passed, "{loss:?}");
    assert!(!gate(&[1.0], 10.0).passed);
}

//! The population prior fitted by empirical Bayes, in place of v10's fixed pseudo-counts.

use svanbot_core::rng::Rng;
use svanbot_learn::prior::{BetaPrior, MAX_STRENGTH, fit_prior, ln_gamma};

/// `players` tallies of `trials` each, every player's true rate `mean - spread` or
/// `mean + spread` in equal shares.
fn tallies(players: u32, trials: u32, mean: f64, spread: f64, seed: u64) -> Vec<(u32, u32)> {
    let mut rng = Rng::seeded(seed);
    let mut draw = || rng.below(1 << 30) as f64 / f64::from(1u32 << 30);
    (0..players)
        .map(|player| {
            let rate = if player % 2 == 0 {
                mean - spread
            } else {
                mean + spread
            };
            let wins = (0..trials).filter(|_| draw() < rate).count() as u32;
            (wins, trials)
        })
        .collect()
}

#[test]
fn ln_gamma_matches_known_values() {
    assert!(ln_gamma(1.0).abs() < 1e-12);
    assert!((ln_gamma(5.0) - 24f64.ln()).abs() < 1e-12);
    assert!((ln_gamma(0.5) - std::f64::consts::PI.sqrt().ln()).abs() < 1e-12);
    assert!((ln_gamma(0.1) - 2.252_712_651_734_206).abs() < 1e-10);
}

#[test]
fn the_strength_follows_how_much_the_players_really_differ() {
    // A spread of 0.1 around 0.3 is the variance of a beta of strength 0.21 / 0.01 - 1 = 20.
    let spread = fit_prior(&tallies(600, 60, 0.3, 0.1, 1)).unwrap();
    assert!((spread.mean - 0.3).abs() < 0.02, "{spread:?}");
    assert!(
        spread.strength > 10.0 && spread.strength < 40.0,
        "{spread:?}"
    );
    // Players who do not differ at all: shrink as hard as the fit allows.
    let alike = fit_prior(&tallies(600, 60, 0.3, 0.0, 2)).unwrap();
    assert!(
        alike.strength > 1_000.0 && alike.strength <= MAX_STRENGTH,
        "{alike:?}"
    );
    assert_eq!(fit_prior(&[(0, 0), (0, 0)]), None);
}

#[test]
fn a_rate_is_shrunk_toward_the_mean_by_the_prior_strength() {
    let prior = BetaPrior {
        mean: 0.3,
        strength: 20.0,
    };
    assert!((prior.shrink(0, 0) - 0.3).abs() < 1e-12);
    assert!((prior.shrink(9, 10) - 15.0 / 30.0).abs() < 1e-12);
    assert!((prior.shrink(900, 1000) - 0.3).abs() > 0.5);
}

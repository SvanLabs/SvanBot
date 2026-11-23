//! The ship bar's canary stage (V11-22): before the fleet takes a release, the canary has
//! played it over 20 000 hands, and does not lose to the v10 fleet over the same window.

use svanbot_runtime::ship::{self, CANARY_HANDS};

/// `hands` results that average `mean` big blinds a hand, half of them half a big blind above it
/// and half below.
fn hands(hands: usize, mean: f64) -> Vec<f64> {
    (0..hands)
        .map(|hand| {
            if hand % 2 == 0 {
                mean + 0.5
            } else {
                mean - 0.5
            }
        })
        .collect()
}

#[test]
fn a_canary_short_of_its_hands_is_not_yet_measured() {
    assert_eq!(
        ship::canary(&hands(CANARY_HANDS - 1, 1.0), &hands(80_000, 0.0)),
        Err("the canary is not yet measured: 19999 of its 20000 hands".into())
    );
}

#[test]
fn a_window_with_too_few_fleet_hands_is_not_yet_measured() {
    assert_eq!(
        ship::canary(&hands(CANARY_HANDS, 0.0), &[]),
        Err("the fleet is not yet measured over the canary's window: 0 of 20000 hands".into())
    );
}

#[test]
fn a_canary_that_loses_to_the_fleet_beyond_the_noise_is_refused() {
    assert_eq!(
        ship::canary(&hands(CANARY_HANDS, -0.1), &hands(CANARY_HANDS, 0.1)),
        Err("the canary loses to the fleet: -20000 mbb/100 ±980 over 20000 hands".into())
    );
}

#[test]
fn a_canary_within_the_noise_of_the_fleet_or_ahead_of_it_lets_the_fleet_take_the_release() {
    let even = ship::canary(&hands(CANARY_HANDS, -0.001), &hands(CANARY_HANDS, 0.001));
    assert_eq!(
        even,
        Ok("the canary against the fleet: -200 mbb/100 ±980 over 20000 hands".into())
    );
    assert!(ship::canary(&hands(CANARY_HANDS, 0.1), &hands(CANARY_HANDS, 0.0)).is_ok());
}

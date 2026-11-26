//! The standing reference arm: one orbit in ten plays the frozen reference policy.

use svanbot_learn::probe::{Arm, Verdict};
use svanbot_learn::reference::{Estimate, SHARE, Side, Standing, assign, bot_seed};
use svanbot_strategy::params::Params;

fn standing(orbit: u64, side: Side, chance: f64) -> Standing {
    Standing {
        orbit,
        side,
        chance,
        reference: Params::DEFAULT.digest(),
    }
}

#[test]
fn a_bot_draws_the_same_arm_for_the_same_orbit_every_time() {
    let seed = bot_seed("SurSvan");
    let mut reference = 0;
    for orbit in 0..100_000 {
        let drawn = assign(seed, orbit);
        assert_eq!(drawn, assign(seed, orbit));
        match drawn {
            (Side::Reference, chance) => {
                assert_eq!(chance, SHARE);
                reference += 1;
            }
            (Side::Policy, chance) => assert_eq!(chance, 1.0 - SHARE),
        }
    }
    assert!((9_500..=10_500).contains(&reference), "{reference}");
}

#[test]
fn two_bots_with_different_names_draw_their_own_arms() {
    let (first, second) = (bot_seed("SurSvan"), bot_seed("MegaSvan"));
    assert_ne!(first, second);
    assert_eq!(first, bot_seed("SurSvan"));
    let different = (0..1_000)
        .filter(|&orbit| assign(first, orbit).0 != assign(second, orbit).0)
        .count();
    assert!(different > 0, "the two bots drew the same arms");
}

#[test]
fn the_chance_of_the_arm_played_is_the_chance_the_estimate_weights_by() {
    let reference = standing(0, Side::Reference, SHARE);
    let policy = standing(0, Side::Policy, 1.0 - SHARE);
    // The reference is the champion arm, and one in ten orbits drew it.
    assert_eq!(reference.challenger(), (Arm::Champion, 1.0 - SHARE));
    // The bot's policy is the challenger, and nine in ten orbits drew it.
    assert_eq!(policy.challenger(), (Arm::Challenger, 1.0 - SHARE));
}

#[test]
fn the_estimate_recovers_the_policy_s_edge_through_a_one_in_ten_arm() {
    let reference = Params::DEFAULT.digest();
    let policy = Params {
        exploit: 0.4,
        spread: 0.0,
    }
    .digest();
    let mut estimate = Estimate::new(reference, policy);
    let seed = bot_seed("SurSvan");
    // The policy wins a big blind a hand; the reference breaks even.
    for orbit in 0..20_000 {
        let (side, chance) = assign(seed, orbit);
        let won = match side {
            Side::Policy => 6.0,
            Side::Reference => 0.0,
        };
        estimate.observe(&standing(orbit, side, chance), won, 6);
    }
    let (mean, radius) = estimate.estimate();
    assert!((mean - 100.0).abs() < 10.0, "{mean} ± {radius}");
    assert!(radius < 10.0, "{radius}");
}

#[test]
fn the_guard_clears_a_level_arm_and_trips_a_losing_one() {
    let reference = Params::DEFAULT.digest();
    let policy = Params {
        exploit: 0.4,
        spread: 0.0,
    }
    .digest();
    let seed = bot_seed("SurSvan");
    let mut level = Estimate::guard(reference, policy, 3_000, 20.0);
    let mut losing = Estimate::guard(reference, policy, 3_000, 20.0);
    for orbit in 0..1_000 {
        let (side, chance) = assign(seed, orbit);
        let drawn = standing(orbit, side, chance);
        level.observe(&drawn, 0.0, 6);
        // The policy loses two big blinds a hand; the reference breaks even.
        losing.observe(
            &drawn,
            match side {
                Side::Policy => -12.0,
                Side::Reference => 0.0,
            },
            6,
        );
    }
    assert_eq!(level.verdict(), Verdict::Cleared);
    assert_eq!(losing.verdict(), Verdict::Harmful);
}

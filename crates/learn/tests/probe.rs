//! The Probe: a live experiment that compares the champion with one challenger.

use svanbot_core::rng::Rng;
use svanbot_learn::probe::{
    Arm, Decided, Hypothesis, Kind, Probe, Sequence, Verdict, arm, corrected,
};
use svanbot_strategy::params::Params;

fn hypothesis(kind: Kind) -> Hypothesis {
    Hypothesis {
        champion: Params::DEFAULT.digest(),
        challenger: Params {
            exploit: 0.25,
            spread: 0.0,
            ..Params::DEFAULT
        }
        .digest(),
        kind,
        harm_margin: 0.0,
        sim_estimate: None,
        max_orbits: 20_000,
    }
}

/// Roughly normal noise with mean 0 and spread 1: twelve uniforms, less six.
fn noise(rng: &mut Rng) -> f64 {
    (0..12)
        .map(|_| (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64)
        .sum::<f64>()
        - 6.0
}

/// Runs a probe where an orbit of the challenger wins `edge` bb/100 more than one of the
/// champion, with a spread of 300 bb/100 an orbit, and returns the verdict it ends on.
fn run(hypothesis: Hypothesis, edge: f64, seed: u64) -> (Verdict, u64) {
    let mut rng = Rng::seeded(seed);
    let mut probe = Probe::new(hypothesis);
    for orbit in 0..hypothesis.max_orbits {
        let (arm, chance) = arm(hypothesis.id(), 7, orbit);
        let rate = 300.0 * noise(&mut rng) + if arm == Arm::Challenger { edge } else { 0.0 };
        // Six hands an orbit: the rate in bb/100 is the orbit's big blinds times 100 / 6.
        probe.observe(arm, chance, rate * 6.0 / 100.0, 6);
        if probe.verdict() != Verdict::Running {
            return (probe.verdict(), orbit + 1);
        }
    }
    (probe.verdict(), hypothesis.max_orbits)
}

#[test]
fn a_hypothesis_is_named_by_its_pair_of_digests() {
    let probe = hypothesis(Kind::Probe);
    assert_eq!(probe.id(), hypothesis(Kind::Probe).id());
    let swapped = Hypothesis {
        champion: probe.challenger,
        challenger: probe.champion,
        ..probe
    };
    assert_ne!(probe.id(), swapped.id());
}

#[test]
fn an_orbit_s_arm_is_a_coin_fixed_by_the_probe_the_table_and_the_orbit() {
    let id = hypothesis(Kind::Probe).id();
    let challenger = (0..10_000)
        .filter(|&orbit| arm(id, 3, orbit).0 == Arm::Challenger)
        .count();
    assert!((4_800..=5_200).contains(&challenger), "{challenger}");
    for orbit in 0..100 {
        assert_eq!(arm(id, 3, orbit), arm(id, 3, orbit));
        assert_eq!(arm(id, 3, orbit).1, 0.5);
    }
    assert!((0..100).any(|orbit| arm(id, 3, orbit).0 != arm(id, 4, orbit).0));
    assert!((0..100).any(|orbit| arm(id, 3, orbit).0 != arm(id + 1, 3, orbit).0));
}

#[test]
fn the_luck_of_our_own_draw_is_taken_out_whatever_the_values_are_worth() {
    // Half the time each of two actions, valued 10 and 0: the draw is worth 5 either way.
    let decided = |taken| Decided {
        strategy: vec![0.5, 0.5],
        values: vec![10.0, 0.0],
        taken,
    };
    assert_eq!(corrected(3.0, &[decided(0)]), -2.0);
    assert_eq!(corrected(3.0, &[decided(1)]), 8.0);
    // So over the draw itself the correction averages nothing.
    assert_eq!(
        0.5 * corrected(3.0, &[decided(0)]) + 0.5 * corrected(3.0, &[decided(1)]),
        3.0
    );
    assert_eq!(corrected(3.0, &[]), 3.0);
}

#[test]
fn a_confidence_sequence_holds_at_every_look_at_once() {
    // 300 streams with a true mean of 0, looked at after every one of 2,000 observations. At
    // the 5% level a fixed-sample interval looked at this often would be wrong on most streams;
    // the sequence may be wrong on about 5% of them.
    let mut wrong = 0;
    for seed in 0..300 {
        let mut rng = Rng::seeded(seed);
        let mut sequence = Sequence::new(500.0);
        let mut left = false;
        for _ in 0..2_000 {
            sequence.observe(noise(&mut rng));
            left |= sequence.count() >= 10.0 && sequence.mean().abs() > sequence.radius(0.05);
        }
        wrong += u32::from(left);
    }
    assert!(wrong <= 24, "{wrong} of 300 streams left their interval");
    // And it closes in: the radius falls as the observations come.
    let mut rng = Rng::seeded(1);
    let mut sequence = Sequence::new(500.0);
    let mut radii = Vec::new();
    for n in 1..=4_000 {
        sequence.observe(noise(&mut rng));
        if [100, 1_000, 4_000].contains(&n) {
            radii.push(sequence.radius(0.05));
        }
    }
    assert!(radii[0] > radii[1] && radii[1] > radii[2], "{radii:?}");
    assert!(radii[2] < 0.1, "{radii:?}");
}

#[test]
fn a_better_challenger_is_supported_a_worse_one_is_harmful_and_an_equal_one_runs_out() {
    let (verdict, orbits) = run(hypothesis(Kind::Probe), 40.0, 5);
    assert_eq!(verdict, Verdict::Supported);
    assert!(orbits < 20_000);
    assert_eq!(run(hypothesis(Kind::Probe), -40.0, 5).0, Verdict::Harmful);
    assert_eq!(
        run(hypothesis(Kind::Probe), 0.0, 5),
        (Verdict::Inconclusive, 20_000)
    );
}

#[test]
fn a_probe_that_disagrees_with_its_sim_estimate_says_so() {
    let optimistic = Hypothesis {
        sim_estimate: Some(60.0),
        ..hypothesis(Kind::Probe)
    };
    assert_eq!(run(optimistic, 0.0, 5).0, Verdict::SimGap);
}

#[test]
fn a_guard_clears_after_its_hands_unless_the_challenger_is_clearly_losing() {
    // 3,000 hands are 500 orbits of six.
    let guard = hypothesis(Kind::Guard { hands: 3_000 });
    assert_eq!(run(guard, 0.0, 5), (Verdict::Cleared, 500));
    assert_eq!(run(guard, 40.0, 5), (Verdict::Cleared, 500));
    let (verdict, orbits) = run(guard, -150.0, 5);
    assert_eq!(verdict, Verdict::Harmful);
    assert!(orbits < 500);
}

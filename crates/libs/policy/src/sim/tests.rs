//! Tests for the paired simulator.
use super::*;

#[test]
fn combining_halves_keeps_the_mean_and_shrinks_the_error() {
    let a = PairedResult { hands: 1000, mean_bb: 0.02, se_bb: 0.01, differing: 400 };
    let b = PairedResult { hands: 1000, mean_bb: 0.04, se_bb: 0.01, differing: 500 };
    let c = a.combine(&b);
    assert_eq!((c.hands, c.differing), (2000, 900));
    assert!((c.mean_bb - 0.03).abs() < 1e-12);
    // Independent halves: SE of the pooled mean is se/sqrt(2).
    assert!((c.se_bb - 0.01 / 2f64.sqrt()).abs() < 1e-12, "{}", c.se_bb);
    // Unequal sizes weight by hands.
    let d = PairedResult { hands: 3000, mean_bb: 0.0, se_bb: 0.005, differing: 0 };
    assert!((a.combine(&d).mean_bb - 0.005).abs() < 1e-12);
}

#[test]
fn identical_policies_never_differ_and_runs_are_deterministic() {
    let p = crate::policy::Params { samples: 300, ..Default::default() };
    let archetypes: Vec<(crate::agents::Archetype, f64)> =
        ["station", "maniac"].iter().map(|k| (crate::agents::archetype(k), 1.0)).collect();
    let same = paired_eval(&p, &p, &archetypes, &sv10_model::model::ModelStore::default(), None, 2, 60, 100, 7);
    assert_eq!((same.hands, same.differing), (120, 0));
    assert_eq!(same.mean_bb, 0.0);
    // Profile clones: same guarantees.
    let clones = vec![(crate::agents::ProfileClone::exact("v", sv10_model::model::ModelStore::default().profile("v")), 1.0)];
    let q = crate::policy::Params { call_margin: 0.05, ..p.clone() };
    let r1 = paired_eval(&p, &q, &clones, &sv10_model::model::ModelStore::default(), None, 2, 60, 100, 11);
    let r2 = paired_eval(&p, &q, &clones, &sv10_model::model::ModelStore::default(), None, 2, 60, 100, 11);
    assert_eq!((r1.hands, r1.differing, r1.mean_bb), (r2.hands, r2.differing, r2.mean_bb));
    assert_eq!(paired_eval(&p, &p, &clones, &sv10_model::model::ModelStore::default(), None, 2, 60, 100, 11).differing, 0);
}

#[test]
fn a_shared_champion_gives_the_same_results_as_separate_evaluations() {
    let p = crate::policy::Params { samples: 300, ..Default::default() };
    let clones = vec![
        (crate::agents::ProfileClone::exact("v", sv10_model::model::ModelStore::default().profile("v")), 1.0),
        (crate::agents::ProfileClone::exact("w", sv10_model::model::ModelStore::default().profile("w")), 2.0),
    ];
    let challengers =
        vec![crate::policy::Params { call_margin: 0.05, ..p.clone() }, p.clone(), crate::policy::Params { open_bb: 3.0, ..p.clone() }];
    let models = sv10_model::model::ModelStore::default();
    let many = paired_eval_many(&p, &challengers, &clones, &models, None, 3, 50, 100, 21);
    assert_eq!(many.len(), 3);
    for (c, r) in challengers.iter().zip(&many) {
        let one = paired_eval(&p, c, &clones, &models, None, 3, 50, 100, 21);
        assert_eq!((one.hands, one.differing), (r.hands, r.differing));
        assert_eq!(one.mean_bb.to_bits(), r.mean_bb.to_bits());
        assert_eq!(one.se_bb.to_bits(), r.se_bb.to_bits());
    }
    assert_eq!(many[1].differing, 0);
    assert!(paired_eval_many(&p, &[], &clones, &models, None, 3, 50, 100, 21).is_empty());
}

#[test]
fn variance_split_separates_decision_noise_from_card_noise() {
    // Two cells with different card-driven means (±1) plus ±0.5 decision noise across salts.
    let d = vec![vec![1.5, -1.5], vec![0.5, -0.5]];
    let v = split_variance(&d);
    assert_eq!((v.cells, v.salts), (2, 2));
    assert!((v.within - 0.5).abs() < 1e-12, "{v:?}");
    assert!((v.total - 5.0 / 3.0).abs() < 1e-12, "{v:?}");
    assert!((v.between - (5.0 / 3.0 - 0.5)).abs() < 1e-12);
    // One salt measures no decision noise at all.
    assert_eq!(split_variance(&d[..1]).within, 0.0);
}

#[test]
fn salt_zero_is_the_unsalted_table_and_other_salts_keep_the_cards() {
    use crate::agents::{OpponentSpec, archetype};
    let make = || -> Vec<Box<dyn Agent>> { ["lag", "station", "maniac"].iter().enumerate().map(|(i, k)| archetype(k).agent(i)).collect() };
    let (mut a, mut b, mut c) = (make(), make(), make());
    let (mut la, mut lb, mut lc) = (Vec::new(), Vec::new(), Vec::new());
    run_table_logged(&mut a, 200, 100, 77, None, Some(&mut la));
    run_table_salted(&mut b, 200, 100, 77, 0, None, Some(&mut lb));
    run_table_salted(&mut c, 200, 100, 77, 5, None, Some(&mut lc));
    assert_eq!(la, lb, "salt 0 must reproduce today's streams exactly");
    assert_ne!(la, lc, "a new salt changes decisions");
}

#[test]
fn table_slices_pool_to_the_whole_evaluation() {
    let p = crate::policy::Params { samples: 300, ..Default::default() };
    let clones = vec![
        (crate::agents::ProfileClone::exact("v", sv10_model::model::ModelStore::default().profile("v")), 1.0),
        (crate::agents::ProfileClone::exact("w", sv10_model::model::ModelStore::default().profile("w")), 2.0),
    ];
    let models = sv10_model::model::ModelStore::default();
    let c = [crate::policy::Params { call_margin: 0.05, ..p.clone() }, crate::policy::Params { open_bb: 3.0, ..p.clone() }];
    let arms: Vec<Arm<'_>> = c.iter().map(|q| Arm { params: q, nn: None }).collect();
    let champ = Arm { params: &p, nn: None };
    let whole = paired_eval_arms(&champ, &arms, &clones, &models, 4, 40, 100, 31);
    let mut sums = vec![PairedSums::default(); 2];
    for range in [0..1, 1..3, 3..4] {
        for (s, part) in sums.iter_mut().zip(paired_sums_arms(&champ, &arms, &clones, &models, range, 40, 100, 31)) {
            s.add(&part);
        }
    }
    // One challenger per slice call gives the same totals: its tables do not depend on the others.
    let alone = paired_sums_arms(&champ, &arms[1..], &clones, &models, 0..4, 40, 100, 31);
    assert_eq!((alone[0].hands, alone[0].differing), (sums[1].hands, sums[1].differing));
    assert!((alone[0].sum - sums[1].sum).abs() < 1e-9 && (alone[0].sum_sq - sums[1].sum_sq).abs() < 1e-9);
    for (w, s) in whole.iter().zip(&sums) {
        let r = s.result();
        assert_eq!((r.hands, r.differing), (w.hands, w.differing));
        assert!((r.mean_bb - w.mean_bb).abs() < 1e-12 && (r.se_bb - w.se_bb).abs() < 1e-9, "{r:?} vs {w:?}");
    }
    assert!(whole[0].differing > 0, "the test needs a challenger that changes outcomes");
    assert_eq!(PairedSums::default().result().hands, 0);
}

#[test]
fn empty_paired_results_do_not_poison_independent_pooling() {
    let measured = PairedResult { hands: 1000, mean_bb: 0.03, se_bb: 0.005, differing: 100 };
    let empty = PairedResult::default();
    for pooled in [empty.combine(&measured), measured.combine(&empty)] {
        assert_eq!((pooled.hands, pooled.differing), (1000, 100));
        assert_eq!(pooled.mean_bb, measured.mean_bb);
        assert_eq!(pooled.se_bb, measured.se_bb);
        assert_eq!(pooled.lower_95(), measured.lower_95());
    }
    let pooled = empty.combine(&empty);
    assert_eq!(pooled.hands, 0);
    assert_eq!(pooled.mean_bb, 0.0);
    assert_eq!(pooled.se_bb, f64::INFINITY);
}

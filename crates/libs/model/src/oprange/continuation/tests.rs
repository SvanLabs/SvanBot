//! Bit-identity tests for the logistic kernels (0353): the table and the memo are speed only, so
//! every result must be the bits the pre-0353 expressions produced. Compared with `to_bits()`,
//! never a tolerance. Split out of `continuation.rs` (the 500-line rule).

use super::*;

/// All four streets, in [`Street::index`] order.
fn streets() -> [Street; STREETS] {
    [Street::Preflop, Street::Flop, Street::Turn, Street::River]
}

/// A deterministic spread in 0..1 per index, so any failure is reproducible.
fn hash_unit(i: usize) -> f32 {
    let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 29;
    (x >> 40) as f32 / (1u32 << 24) as f32
}

/// A 1326-combo score vector from a per-index function.
fn scores_of(f: impl Fn(usize) -> f32) -> Vec<f32> {
    (0..NUM_COMBOS).map(f).collect()
}

/// A range from a per-combo weight function.
fn range_of(f: impl Fn(usize) -> f32) -> Range {
    let mut r = Range::empty();
    for i in 0..NUM_COMBOS {
        r.w[i] = f(i);
    }
    r
}

/// The pre-0353 body of [`equity_vs_histogram`], kept as the scalar twin the table must
/// reproduce bit for bit.
fn equity_inline(hist: &[f32; STRENGTH_BINS], street: Street) -> [f32; STRENGTH_BINS] {
    let mut out = [0f32; STRENGTH_BINS];
    let tau = [0.30f32, 0.12, 0.09, 0.05][street.index()];
    for v in 0..STRENGTH_BINS {
        let sv = (v as f32 + 0.5) / STRENGTH_BINS as f32;
        let mut e = 0.0;
        for (b, &w) in hist.iter().enumerate() {
            if w > 0.0 {
                let sb = (b as f32 + 0.5) / STRENGTH_BINS as f32;
                e += w / (1.0 + ((sb - sv) / tau).exp());
            }
        }
        let floor = [0.25f32, 0.12, 0.07, 0.02][street.index()];
        out[v] = floor + (1.0 - floor) * e;
    }
    out
}

/// The pre-0353 body of `continue_by_order`, kept as the scalar twin the memo must reproduce
/// bit for bit — bar-rise sort included, which is deliberately still the old non-total one (0362):
/// it is the record of what the code did, not what it should do.
fn continue_inline(villain: &Range, scores: &[f32], order: Option<&[u16; NUM_COMBOS]>, need: f32, max_cont: f32) -> (f32, Range) {
    let total: f32 = villain.w.iter().sum();
    if total <= 0.0 {
        return (0.0, villain.clone());
    }
    let soft = |bar: f32, score: f32| {
        let c = 1.0 / (1.0 + ((bar - score) / 0.012).exp());
        if c < 0.03 { 0.0 } else { c }
    };
    let mut bar = need;
    let above: f32 = (0..NUM_COMBOS).filter(|&i| villain.w[i] > 0.0 && scores[i] >= need).map(|i| villain.w[i]).sum();
    if above / total > max_cont {
        let target = max_cont * total;
        let mut acc = 0.0;
        let mut cross = |i: usize| {
            acc += villain.w[i];
            (acc >= target).then(|| scores[i].max(need))
        };
        let found = match order {
            Some(order) => order.iter().map(|&i| i as usize).filter(|&i| villain.w[i] > 0.0).find_map(&mut cross),
            None => {
                let mut idx: Vec<usize> = (0..NUM_COMBOS).filter(|&i| villain.w[i] > 0.0).collect();
                idx.sort_unstable_by(|&a, &b| scores[b].partial_cmp(&scores[a]).unwrap_or(std::cmp::Ordering::Equal));
                idx.into_iter().find_map(&mut cross)
            }
        };
        if let Some(b) = found {
            bar = b;
        }
    }
    let mut cont = villain.clone();
    let mut kept = 0.0;
    for i in 0..NUM_COMBOS {
        let w = villain.w[i];
        if w <= 0.0 {
            continue;
        }
        let c = soft(bar, scores[i]);
        cont.w[i] = w * c;
        kept += w * c;
    }
    (kept / total, cont)
}

/// Every bin must carry the same bits as the twin.
fn same_equity(got: &[f32; STRENGTH_BINS], want: &[f32; STRENGTH_BINS], what: &str) {
    for v in 0..STRENGTH_BINS {
        assert_eq!(got[v].to_bits(), want[v].to_bits(), "{what}: bin {v}: {} vs {}", got[v], want[v]);
    }
}

/// The continuing fraction and every combo weight must carry the same bits as the twin.
fn same_continue(got: &(f32, Range), want: &(f32, Range), what: &str) {
    assert_eq!(got.0.to_bits(), want.0.to_bits(), "{what}: continuing fraction {} vs {}", got.0, want.0);
    for i in 0..NUM_COMBOS {
        assert_eq!(got.1.w[i].to_bits(), want.1.w[i].to_bits(), "{what}: combo {i}");
    }
}

/// Histograms that reach every branch of the sum: zero, one-hot, uniform, and weights that are
/// negative, subnormal, huge, infinite or NaN.
fn histograms() -> Vec<[f32; STRENGTH_BINS]> {
    let mut hists = vec![
        [0.0; STRENGTH_BINS],
        [-0.0; STRENGTH_BINS],
        [1.0; STRENGTH_BINS],
        [-1.0; STRENGTH_BINS],
        [f32::NAN; STRENGTH_BINS],
        [f32::INFINITY; STRENGTH_BINS],
        [f32::NEG_INFINITY; STRENGTH_BINS],
        [f32::MIN_POSITIVE; STRENGTH_BINS],
        [f32::from_bits(1); STRENGTH_BINS],
        [f32::MAX; STRENGTH_BINS],
        [100.0; STRENGTH_BINS],
    ];
    for b in 0..STRENGTH_BINS {
        let mut one = [0f32; STRENGTH_BINS];
        one[b] = 1.0;
        hists.push(one);
    }
    for seed in 0..64 {
        let mut h = [0f32; STRENGTH_BINS];
        for (b, x) in h.iter_mut().enumerate() {
            *x = hist_weight(hash_unit(seed * 41 + b));
        }
        hists.push(h);
    }
    hists
}

/// A weight that reaches the awkward branches: zero, negative zero, subnormal, huge, negative,
/// infinite, NaN, or an ordinary value.
fn hist_weight(u: f32) -> f32 {
    match (u * 8.0) as u32 {
        0 => 0.0,
        1 => -0.0,
        2 => f32::from_bits(1),
        3 => u * 1e38,
        4 => -u,
        5 => f32::INFINITY,
        6 => f32::NAN,
        _ => u,
    }
}

/// Ranges that reach every branch of the continuation loop: uniform, empty, one live combo,
/// sparse, extreme, subnormal, and mixtures with zeros, negatives and infinities.
fn ranges_under_test() -> Vec<Range> {
    vec![
        Range::full(),
        Range::empty(),
        range_of(|i| if i == 7 { 1.0 } else { 0.0 }),
        range_of(|i| if i % 3 == 0 { 0.0 } else { 1.0 }),
        range_of(|i| if i < 40 { hash_unit(i) * 1e38 } else { 0.0 }),
        range_of(|_| f32::from_bits(1)),
        range_of(|i| match i % 4 {
            0 => 0.0,
            1 => -1.0,
            2 => hash_unit(i) * 2.0,
            _ => f32::INFINITY,
        }),
    ]
}

/// Score sets that reach every branch of the soft threshold: equal to the bar, the 169-rung
/// preflop ladder, all-distinct, and NaN, ±infinity, ±0 and subnormal scores.
fn score_sets() -> Vec<Vec<f32>> {
    vec![
        vec![0.5f32; NUM_COMBOS],
        vec![0.0f32; NUM_COMBOS],
        vec![f32::from_bits(1); NUM_COMBOS],
        scores_of(|i| (i % 169) as f32 / 169.0),
        scores_of(|i| i as f32 / NUM_COMBOS as f32),
        scores_of(|i| if i % 7 == 0 { f32::NAN } else { hash_unit(i) }),
        scores_of(|i| if i % 5 == 0 { f32::INFINITY } else { hash_unit(i) }),
        scores_of(|i| match i % 11 {
            0 => 0.0,
            1 => -0.0,
            2 => f32::NEG_INFINITY,
            3 => -hash_unit(i),
            _ => hash_unit(i),
        }),
    ]
}

#[test]
fn every_table_entry_equals_the_expression_it_replaces() {
    for street in streets() {
        let den = logistic_den(street);
        let tau = TAU[street.index()];
        for v in 0..STRENGTH_BINS {
            for b in 0..STRENGTH_BINS {
                let want = 1.0 + ((bin_center(b) - bin_center(v)) / tau).exp();
                let got = den[v * STRENGTH_BINS + b];
                assert_eq!(got.to_bits(), want.to_bits(), "street {} v{v} b{b}: {got} vs {want}", street.index());
            }
        }
    }
}

#[test]
fn equity_vs_histogram_is_bit_identical_to_its_inline_sum() {
    for street in streets() {
        for hist in histograms() {
            let got = equity_vs_histogram(&hist, street);
            let want = equity_inline(&hist, street);
            same_equity(&got, &want, &format!("street {}", street.index()));
        }
    }
}

#[test]
fn continue_by_score_is_bit_identical_to_its_inline_soft() {
    let needs = [0.0f32, 0.25, 0.5, 0.75, 1.0, -1.0, f32::NAN, f32::INFINITY];
    let max_conts = [0.02f32, 0.3, 0.9, 1.0, 0.0, -1.0];
    // NaN scores are left to `nan_scores_are_bit_identical_on_the_order_path` and to the two 0362
    // tests below. This path sorts, and the twin it compares against is the pre-0353 body, whose
    // sort is not a total order once a score is NaN - the order check inside `sort_unstable_by`
    // faults on some such inputs and not others, so its output is no oracle for them. The sort this
    // file now ships is total.
    let scores: Vec<Vec<f32>> = score_sets().into_iter().filter(|s| !s.iter().any(|x| x.is_nan())).collect();
    let mut cases = 0;
    for villain in ranges_under_test() {
        for s in &scores {
            for need in needs {
                for max_cont in max_conts {
                    let got = continue_by_score(&villain, s, need, max_cont);
                    let want = continue_inline(&villain, s, None, need, max_cont);
                    same_continue(&got, &want, &format!("need {need} max_cont {max_cont}"));
                    cases += 1;
                }
            }
        }
    }
    assert!(cases >= 384, "the sweep shrank to {cases} cases");
}

/// 0362: the bar-rise sort is a total order with a NaN in it, and its order is pinned — strongest
/// real score first, NaN last. `unwrap_or(Equal)` compared a NaN `Equal` to everything, so the
/// relation was not transitive: release ordered arbitrarily, and a debug build panicked on the
/// inputs `sort_unstable_by`'s order check happened to catch.
#[test]
fn a_nan_score_sorts_last_in_the_bar_rise_order() {
    // Six live combos with distinct real scores, so the walk order is fully determined.
    let villain = range_of(|i| if i < 6 { 1.0 } else { 0.0 });
    let scores = scores_of(|i| match i {
        0 => 0.25,
        1 => f32::NAN,
        2 => 0.75,
        3 => -0.5,
        4 => f32::INFINITY,
        5 => f32::from_bits(1),
        _ => 0.0,
    });
    assert_eq!(by_descending_score(&villain, &scores), vec![4, 2, 0, 5, 3, 1], "strongest first, the NaN last");
}

/// 0362 at production size: the whole index, 1,326 live combos with NaNs among the scores, sorts to
/// completion, keeps every combo exactly once, leaves the real scores descending, and puts every NaN
/// behind all of them. The NaNs here include a negative one: `is_nan` decides, not a sign bit, so no
/// NaN pattern reaches the front of the walk (a `total_cmp` on the raw scores would make `+NaN` the
/// strongest of all).
#[test]
fn the_full_bar_rise_index_sorts_with_nans_in_it() {
    let scores = scores_of(|i| match i % 7 {
        0 => f32::NAN,
        1 => -f32::NAN,
        _ => hash_unit(i),
    });
    let order = by_descending_score(&Range::full(), &scores);
    let real = scores.iter().filter(|s| !s.is_nan()).count();
    assert_eq!(order.len(), NUM_COMBOS);
    let mut seen = vec![false; NUM_COMBOS];
    for &i in &order {
        assert!(!seen[i], "combo {i} appears twice");
        seen[i] = true;
    }
    assert_eq!(order.iter().position(|&i| scores[i].is_nan()), Some(real), "every real score sorts before every NaN");
    assert!(order[..real].windows(2).all(|w| scores[w[0]] >= scores[w[1]]), "the real scores stay strongest first");
}

/// NaN scores, compared where the bar rise walks the combo `order` instead of sorting. A NaN bar or
/// NaN score must still take the memo's bypass (which recomputes rather than caching the `0xffff_ffff`
/// marker) and land on the same bits as the inline `exp`.
#[test]
fn nan_scores_are_bit_identical_on_the_order_path() {
    let t = preflop::combo_scores_vs_top(0.3);
    let nan = scores_of(|i| if i % 7 == 0 { f32::NAN } else { hash_unit(i) });
    for villain in ranges_under_test() {
        for need in [0.0f32, 0.25, 0.5, 1.0, f32::NAN] {
            for max_cont in [0.02f32, 0.3, 1.0, -1.0] {
                let got = continue_by_order(&villain, &nan, Some(&t.order), need, max_cont);
                let want = continue_inline(&villain, &nan, Some(&t.order), need, max_cont);
                same_continue(&got, &want, &format!("nan scores, need {need} max_cont {max_cont}"));
            }
        }
    }
}

/// The live path: scores read out of a 40-bin equity table.
#[test]
fn villain_continue_is_bit_identical_to_its_inline_soft() {
    let hist: [f32; STRENGTH_BINS] = std::array::from_fn(hash_unit);
    let eq = equity_vs_histogram(&hist, Street::Flop);
    // Slightly wider than 0..1, so both clamp edges of the bin mapping are exercised.
    let strength = scores_of(|i| ((i as f32 + 0.5) / NUM_COMBOS as f32) * 1.02 - 0.01);
    // The pre-0353 score mapping, verbatim.
    let scores = scores_of(|i| eq[((strength[i] * STRENGTH_BINS as f32) as usize).min(STRENGTH_BINS - 1)]);
    for villain in ranges_under_test() {
        for need in [0.0f32, 0.3, 0.6, 1.0] {
            for max_cont in [0.1f32, 0.5, 0.9] {
                let got = villain_continue(&villain, &strength, &eq, need, max_cont);
                let want = continue_inline(&villain, &scores, None, need, max_cont);
                same_continue(&got, &want, &format!("need {need} max_cont {max_cont}"));
            }
        }
    }
}

/// The preflop path, including the precomputed strongest-first `order` the bar rise uses.
#[test]
fn the_preflop_ladder_is_bit_identical_to_its_inline_soft() {
    let t = preflop::combo_scores_vs_top(0.3);
    for villain in ranges_under_test() {
        for need in [0.0f32, 0.1, 0.4, 0.9, 1.0] {
            for max_cont in [0.05f32, 0.5, 1.0] {
                let got = villain_continue_preflop(&villain, 0.3, need, max_cont);
                let want = continue_inline(&villain, &t.score, Some(&t.order), need, max_cont);
                same_continue(&got, &want, &format!("need {need} max_cont {max_cont}"));
            }
        }
    }
}

/// The memo is a cache, not an approximation: 1,326 distinct arguments into its 512 slots, so
/// most lookups evict, and the bits must still be the inline ones.
#[test]
fn a_memo_oversubscribed_by_distinct_arguments_still_matches_the_inline_soft() {
    let villain = Range::full();
    let scores = scores_of(|i| (i as f32 + 0.5) / NUM_COMBOS as f32);
    for need in [0.0f32, 0.5, 0.9] {
        let got = continue_by_score(&villain, &scores, need, 1.0);
        let want = continue_inline(&villain, &scores, None, need, 1.0);
        same_continue(&got, &want, &format!("oversubscribed, need {need}"));
    }
}

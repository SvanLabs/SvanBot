//! Preflop hand-class strength ordering.

use crate::equity::equity_vs_ranges;
use std::sync::OnceLock;
use sv10_cards::cards::Card;
use sv10_cards::range::{NUM_COMBOS, Range, class_combos, combos, hand_class};
use sv10_rng::SeedableRng;
use sv10_rng::rngs::SmallRng;

/// Parallel map for `OnceLock` initializers. Deliberately not rayon: a rayon worker blocked on
/// the initializer can steal a task that re-enters the same `OnceLock` and waits on itself.
fn scoped_map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism().map(|t| t.get()).unwrap_or(1).min(n.max(1));
    let mut out: Vec<Option<T>> = (0..n).map(|_| None).collect();
    let chunk = n.div_ceil(threads).max(1);
    std::thread::scope(|s| {
        for (k, part) in out.chunks_mut(chunk).enumerate() {
            let f = &f;
            s.spawn(move || {
                for (j, slot) in part.iter_mut().enumerate() {
                    *slot = Some(f(k * chunk + j));
                }
            });
        }
    });
    out.into_iter().map(Option::unwrap).collect()
}

/// Smooth "combo is inside the top `frac` of the range" indicator.
#[inline]
pub fn soft_top(pct: f32, frac: f32) -> f32 {
    let frac = frac.clamp(0.0, 1.0);
    let t = (frac * 0.12).max(0.025);
    1.0 / (1.0 + ((pct - frac) / t).exp())
}

/// Preflop hand classes ordered by strength, with percentiles for building top-x ranges.
pub struct PreflopTable {
    /// Equity of each class against two random hands.
    pub strength: [f32; 169],
    /// Fraction of all combos at least as strong as the class (0 = best).
    pub percentile: [f32; 169],
    /// Classes from strongest to weakest.
    pub order: Vec<usize>,
    /// Per-combo percentile, for building ranges quickly.
    pub combo_percentile: Vec<f32>,
}

fn representative(class: usize) -> [Card; 2] {
    combos().cards.iter().find(|&&(a, b)| hand_class(a, b) == class).map(|&(a, b)| [a, b]).unwrap()
}

/// Equity of each class against two random hands: 30,000 seeded samples per class.
pub fn compute_class_strengths() -> Vec<f32> {
    let full = Range::full();
    scoped_map(169, |c| {
        let mut rng = SmallRng::seed_from_u64(1000 + c as u64);
        equity_vs_ranges(representative(c), &[], &[&full, &full], 30_000, &mut rng) as f32
    })
}

fn table_from_strengths(strengths: &[f32]) -> PreflopTable {
    let mut order: Vec<usize> = (0..169).collect();
    order.sort_by(|&a, &b| strengths[b].partial_cmp(&strengths[a]).unwrap());
    let mut percentile = [0f32; 169];
    let mut acc = 0.0;
    for &c in &order {
        // Midpoint of the class's combo block keeps the ordering strict.
        let w = class_combos(c) / NUM_COMBOS as f64;
        percentile[c] = (acc + w * 0.5) as f32;
        acc += w;
    }
    let combo_percentile = combos().cards.iter().map(|&(a, b)| percentile[hand_class(a, b)]).collect();
    let mut strength = [0f32; 169];
    strength.copy_from_slice(strengths);
    PreflopTable { strength, percentile, order, combo_percentile }
}

/// Built from the compiled-in strengths (`preflop_data.rs`, bit-identical to
/// `compute_class_strengths`; regenerate with `tables preflop-data`).
pub fn table() -> &'static PreflopTable {
    static T: OnceLock<PreflopTable> = OnceLock::new();
    T.get_or_init(|| table_from_strengths(&crate::preflop_data::CLASS_STRENGTH))
}

/// Strength percentile of a hole-card pair's class (0 = best).
pub fn percentile(a: Card, b: Card) -> f32 {
    table().percentile[hand_class(a, b)]
}

/// Heads-up equity of every hand class against the top-`frac` range, for a
/// geometric ladder of fractions; built lazily once.
pub struct TopRangeEquity {
    /// Top-range fractions of the rungs, ascending (1.2% .. 100%, geometric).
    pub fracs: Vec<f32>,
    /// Heads-up equity of each class against each rung's range.
    pub eq: Vec<[f32; 169]>,
}

fn top_range_fracs() -> Vec<f32> {
    (0..TOP_RANGE_RUNGS).map(|i| (0.012f32 * (1.0f32 / 0.012).powf(i as f32 / (TOP_RANGE_RUNGS - 1) as f32)).min(1.0)).collect()
}

/// Number of rungs in the top-range equity ladder.
pub const TOP_RANGE_RUNGS: usize = 28;

/// Class equities against each top-range rung (ranges from `strengths`' combo percentiles):
/// 1,500 seeded samples per class and rung.
pub fn compute_top_range_eq(strengths: &[f32]) -> Vec<[f32; 169]> {
    let fracs = top_range_fracs();
    let pct = &table_from_strengths(strengths).combo_percentile;
    scoped_map(fracs.len(), |i| {
        let f = fracs[i];
        let mut r = Range::empty();
        for i in 0..NUM_COMBOS {
            r.w[i] = soft_top(pct[i], f) + 1e-4;
        }
        let mut out = [0f32; 169];
        for c in 0..169 {
            let mut rng = SmallRng::seed_from_u64(77 + c as u64);
            out[c] = equity_vs_ranges(representative(c), &[], &[&r], 1500, &mut rng) as f32;
        }
        out
    })
}

/// The ladder, from the compiled-in data.
pub fn top_range_equity() -> &'static TopRangeEquity {
    static T: OnceLock<TopRangeEquity> = OnceLock::new();
    T.get_or_init(|| TopRangeEquity { fracs: top_range_fracs(), eq: crate::preflop_data::TOP_RANGE_EQ.to_vec() })
}

/// Rust source of `preflop_data.rs` from a fresh computation (exact f32 bit patterns).
pub fn render_data() -> String {
    let row = |v: &[f32]| v.iter().map(|x| format!("f(0x{:08x})", x.to_bits())).collect::<Vec<_>>().join(", ");
    let mut out = String::from(
        "//! Generated by `tables preflop-data` from `preflop::compute_class_strengths` and\n\
         //! `preflop::compute_top_range_eq` (exact f32 bits). Do not edit; a test recomputes it.\n\n\
         #![cfg_attr(rustfmt, rustfmt_skip)]\n\n\
         const fn f(bits: u32) -> f32 { f32::from_bits(bits) }\n\n",
    );
    let strengths = compute_class_strengths();
    out += &format!("pub static CLASS_STRENGTH: [f32; 169] = [{}];\n\n", row(&strengths));
    out += &format!("pub static TOP_RANGE_EQ: [[f32; 169]; {TOP_RANGE_RUNGS}] = [\n");
    for r in compute_top_range_eq(&strengths) {
        out += &format!("    [{}],\n", row(&r));
    }
    out += "];\n";
    out
}

/// Per-combo class equity against each ladder rung, with the combos ordered by that score
/// (strongest first). Built once per process from the ladder so hot paths neither rebuild the
/// 1,326-entry table nor sort it per call.
pub struct ComboScores {
    /// Class equity of each combo against the rung.
    pub score: [f32; NUM_COMBOS],
    /// All combo indices, highest `score` first.
    pub order: [u16; NUM_COMBOS],
}

/// Scores and order for the first rung at or above `frac` (the last rung beyond it).
pub fn combo_scores_vs_top(frac: f32) -> &'static ComboScores {
    static T: OnceLock<Vec<ComboScores>> = OnceLock::new();
    let rungs = T.get_or_init(|| {
        top_range_equity()
            .eq
            .iter()
            .map(|eq| {
                let mut score = [0f32; NUM_COMBOS];
                for (i, &(a, b)) in combos().cards.iter().enumerate() {
                    score[i] = eq[hand_class(a, b)];
                }
                let mut idx: Vec<u16> = (0..NUM_COMBOS as u16).collect();
                idx.sort_by(|&a, &b| score[b as usize].partial_cmp(&score[a as usize]).unwrap_or(std::cmp::Ordering::Equal));
                let mut order = [0u16; NUM_COMBOS];
                order.copy_from_slice(&idx);
                ComboScores { score, order }
            })
            .collect()
    });
    let t = top_range_equity();
    let i = t.fracs.iter().position(|&f| f >= frac).unwrap_or(t.fracs.len() - 1);
    &rungs[i]
}

/// Class equity against a top-`frac` range (nearest ladder rung).
pub fn class_equity_vs_top(frac: f32) -> &'static [f32; 169] {
    let t = top_range_equity();
    let i = t.fracs.iter().position(|&f| f >= frac).unwrap_or(t.fracs.len() - 1);
    &t.eq[i]
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv10_cards::range::class_name;

    /// The compiled-in tables must be exactly what the computation gives today; if equity sampling
    /// changes, regenerate with `tables preflop-data`.
    #[test]
    fn compiled_data_matches_computation() {
        let want = render_data();
        let have = include_str!("preflop_data.rs");
        assert!(want == have, "preflop_data.rs is stale: run `cargo run --release -p sv10-core --bin tables -- preflop-data`");
    }

    #[test]
    fn ordering_is_sane() {
        let t = table();
        assert_eq!(class_name(t.order[0]), "AA");
        let names: Vec<String> = t.order.iter().take(12).map(|&c| class_name(c)).collect();
        assert!(names.contains(&"KK".to_string()) && names.contains(&"AKs".to_string()), "{names:?}");
        let worst: Vec<String> = t.order.iter().rev().take(5).map(|&c| class_name(c)).collect();
        assert!(worst.iter().any(|n| n.starts_with("32") || n.starts_with("42")), "{worst:?}");
        let aa = t.percentile[12];
        assert!(aa < 0.01);
        let top10: f64 = t.order.iter().filter(|&&c| t.percentile[c] < 0.10).map(|&c| class_combos(c)).sum();
        assert!((top10 / 1326.0 - 0.10).abs() < 0.02);
    }
}

//! v10.0.2's preflop tables: each hand class's all-in equity against two random hands, ranked
//! into a percentile, and each class's equity against a top slice of hands at 28 widths.
//!
//! v10 computed both offline and shipped their exact f32 bits; `data/preflop.txt` is those bits,
//! the class strengths on its first line and one width a line after it.

use std::sync::OnceLock;

use super::cards::{NUM_COMBOS, class_combos, combos, hand_class};

/// The widths of the top slices, as fractions of all hands.
pub const TOP_RANGE_RUNGS: usize = 28;

/// The weight a combo at `pct` has in the top `frac` of hands: a soft step down past `frac`.
pub fn soft_top(pct: f32, frac: f32) -> f32 {
    let frac = frac.clamp(0.0, 1.0);
    let t = (frac * 0.12).max(0.025);
    1.0 / (1.0 + ((pct - frac) / t).exp())
}

/// The classes ranked by strength, and where each class and combo sits in that order.
pub struct PreflopTable {
    pub strength: [f32; 169],
    /// The share of hands above a class, plus half its own.
    pub percentile: [f32; 169],
    pub order: Vec<usize>,
    pub combo_percentile: Vec<f32>,
}

/// The shipped rows of `data/preflop.txt`, as floats.
fn rows() -> &'static [Vec<f32>] {
    static ROWS: OnceLock<Vec<Vec<f32>>> = OnceLock::new();
    ROWS.get_or_init(|| {
        include_str!("data/preflop.txt")
            .lines()
            .map(|line| {
                line.split(' ')
                    .map(|word| f32::from_bits(u32::from_str_radix(word, 16).expect("a hex f32")))
                    .collect()
            })
            .collect()
    })
}

fn table_from_strengths(strengths: &[f32]) -> PreflopTable {
    let mut order: Vec<usize> = (0..169).collect();
    order.sort_by(|&a, &b| strengths[b].partial_cmp(&strengths[a]).expect("no NaN"));
    let mut percentile = [0f32; 169];
    let mut acc = 0.0;
    for &c in &order {
        let w = class_combos(c) / NUM_COMBOS as f64;
        percentile[c] = (acc + w * 0.5) as f32;
        acc += w;
    }
    let combo_percentile = combos()
        .cards
        .iter()
        .map(|&(a, b)| percentile[hand_class(a, b)])
        .collect();
    let mut strength = [0f32; 169];
    strength.copy_from_slice(strengths);
    PreflopTable {
        strength,
        percentile,
        order,
        combo_percentile,
    }
}

/// The table from v10's class strengths.
pub fn table() -> &'static PreflopTable {
    static TABLE: OnceLock<PreflopTable> = OnceLock::new();
    TABLE.get_or_init(|| table_from_strengths(&rows()[0]))
}

fn top_range_fracs() -> Vec<f32> {
    (0..TOP_RANGE_RUNGS)
        .map(|i| {
            let step = i as f32 / (TOP_RANGE_RUNGS - 1) as f32;
            (0.012f32 * (1.0f32 / 0.012).powf(step)).min(1.0)
        })
        .collect()
}

/// Every combo's equity against the top slice of one width, and the combos best first.
pub struct ComboScores {
    pub score: [f32; NUM_COMBOS],
    pub order: [u16; NUM_COMBOS],
}

/// The rung at or past `frac`, or the widest.
fn rung(frac: f32) -> usize {
    static FRACS: OnceLock<Vec<f32>> = OnceLock::new();
    let fracs = FRACS.get_or_init(top_range_fracs);
    fracs
        .iter()
        .position(|&f| f >= frac)
        .unwrap_or(fracs.len() - 1)
}

/// The scores against the narrowest slice at least `frac` wide.
pub fn combo_scores_vs_top(frac: f32) -> &'static ComboScores {
    static RUNGS: OnceLock<Vec<ComboScores>> = OnceLock::new();
    let rungs = RUNGS.get_or_init(|| {
        rows()[1..]
            .iter()
            .map(|eq| {
                let mut score = [0f32; NUM_COMBOS];
                for (i, &(a, b)) in combos().cards.iter().enumerate() {
                    score[i] = eq[hand_class(a, b)];
                }
                let mut idx: Vec<u16> = (0..NUM_COMBOS as u16).collect();
                idx.sort_by(|&a, &b| {
                    score[usize::from(b)]
                        .partial_cmp(&score[usize::from(a)])
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                let mut order = [0u16; NUM_COMBOS];
                order.copy_from_slice(&idx);
                ComboScores { score, order }
            })
            .collect()
    });
    &rungs[rung(frac)]
}

/// Each class's equity against the narrowest slice at least `frac` wide.
pub fn class_equity_vs_top(frac: f32) -> &'static [f32] {
    &rows()[1 + rung(frac)]
}

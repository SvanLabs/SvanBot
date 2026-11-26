//! The margin check for a candidate policy over the Arena's standard rows.

use crate::arena::BOUT;
use crate::probe::Sequence;

/// The permitted loss against the reference, in mbb/100.
pub const MARGIN: f64 = 20_000.0;
pub const MIN_BOUTS: usize = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    Admitted,
    Refused,
    Unresolved,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    pub admission: Admission,
    pub mean: f64,
    pub radius: f64,
    pub bouts: usize,
}

/// Average the paired candidate-minus-reference differences over the rows, for each bout.
/// Both runs must have the same, nonzero number of rows and bouts. The caller pairs their rows.
pub fn pooled(base: &[&[f64]], next: &[&[f64]]) -> Option<Vec<f64>> {
    let bouts = base.first()?.len();
    if bouts == 0
        || base.len() != next.len()
        || base.iter().chain(next).any(|row| row.len() != bouts)
    {
        return None;
    }
    let unit = 100_000.0 / BOUT as f64;
    Some(
        (0..bouts)
            .map(|bout| {
                base.iter()
                    .zip(next)
                    .map(|(base, next)| (next[bout] - base[bout]) * unit)
                    .sum::<f64>()
                    / base.len() as f64
            })
            .collect(),
    )
}

/// Read the paired bout differences, in mbb/100, against the allowed loss.
pub fn read(differences: &[f64], margin: f64) -> Reading {
    let mut sequence = Sequence::new(200.0);
    for &difference in differences {
        sequence.observe(difference);
    }
    let bouts = differences.len();
    let mean = sequence.mean();
    let radius = sequence.radius(0.05);
    let admission = if bouts < MIN_BOUTS {
        Admission::Unresolved
    } else if mean + radius < -margin {
        Admission::Refused
    } else if radius > margin {
        Admission::Unresolved
    } else {
        Admission::Admitted
    };
    Reading {
        admission,
        mean,
        radius,
        bouts,
    }
}

/// The most a fitted model may lose against any Arena opponent, in mbb/100.
const MODEL_MAX_LOSS: f64 = 5_000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub mean: f64,
    pub radius: f64,
}

#[derive(Debug, PartialEq)]
pub struct ModelComparison {
    pub rows: Vec<Bounds>,
    pub overall: Bounds,
    pub bouts: usize,
}

impl ModelComparison {
    pub fn passes(&self) -> bool {
        self.bouts >= MIN_BOUTS
            && self.overall.mean - self.overall.radius > 0.0
            && self
                .rows
                .iter()
                .all(|row| row.mean - row.radius >= -MODEL_MAX_LOSS)
    }
}

/// Score the paired candidate-minus-incumbent sums in big blinds, one value per bout.
/// Pool rows within each bout and account for checking all rows and the pooled edge.
pub fn model_comparison(rows: &[&[f64]]) -> Option<ModelComparison> {
    let bouts = rows.first()?.len();
    if bouts == 0
        || rows
            .iter()
            .any(|row| row.len() != bouts || row.iter().any(|sum| !sum.is_finite()))
    {
        return None;
    }
    let alpha = 0.05 / (rows.len() + 1) as f64;
    let bounds = |sums: &mut dyn Iterator<Item = f64>| {
        let mut sequence = Sequence::new(200.0);
        for sum in sums {
            sequence.observe(sum * (100_000.0 / BOUT as f64));
        }
        Bounds {
            mean: sequence.mean(),
            radius: sequence.radius(alpha),
        }
    };
    let mut pooled =
        (0..bouts).map(|bout| rows.iter().map(|row| row[bout]).sum::<f64>() / rows.len() as f64);
    Some(ModelComparison {
        rows: rows
            .iter()
            .map(|row| bounds(&mut row.iter().copied()))
            .collect(),
        overall: bounds(&mut pooled),
        bouts,
    })
}

//! Means and interval half-widths.
//!
//! Two forms, both used by live code: from running sums with the population variance (the
//! dashboard's per-row and head-to-head tallies update hand by hand), and from a sample with the
//! unbiased variance (held-out gains, calibration bins).

/// Mean of `n` values that sum to `sum`; 0 when there are none.
pub fn mean(n: f64, sum: f64) -> f64 {
    if n > 0.0 { sum / n } else { 0.0 }
}

/// Half-width at `z` standard errors of the mean of `n` values with running `sum` and `sum_sq`,
/// using the population variance; infinite under two values.
pub fn half_width(n: f64, sum: f64, sum_sq: f64, z: f64) -> f64 {
    if n < 2.0 {
        return f64::INFINITY;
    }
    let var = (sum_sq / n - mean(n, sum).powi(2)).max(0.0);
    z * (var / n).sqrt()
}

/// Running count, sum and sum of squares.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Moments {
    /// Values added.
    pub n: f64,
    /// Their sum.
    pub sum: f64,
    /// The sum of their squares.
    pub sum_sq: f64,
}

impl Moments {
    /// Add one value.
    pub fn add(&mut self, x: f64) {
        self.n += 1.0;
        self.sum += x;
        self.sum_sq += x * x;
    }

    /// The mean ([`mean`]).
    pub fn mean(&self) -> f64 {
        mean(self.n, self.sum)
    }

    /// The half-width at `z` ([`half_width`]).
    pub fn half_width(&self, z: f64) -> f64 {
        half_width(self.n, self.sum, self.sum_sq, z)
    }
}

/// Mean of `xs` and its half-width at `z` standard errors with the unbiased (n − 1) variance; under
/// two values the half-width is infinite (and the mean is the one value, or 0).
pub fn mean_half_width(xs: &[f64], z: f64) -> (f64, f64) {
    if xs.len() < 2 {
        return (xs.first().copied().unwrap_or(0.0), f64::INFINITY);
    }
    let n = xs.len() as f64;
    let m = xs.iter().sum::<f64>() / n;
    let v = xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0);
    (m, z * (v / n).sqrt())
}

/// Two-sided 95% Student-t critical values the benchmark tools report intervals with (0335), by
/// degrees of freedom: tabulated at 1..=10, 14, 19 and 29, rounded upward. A degree of freedom
/// between rows takes the row below it (the larger value), so an interval is never narrower than the
/// textbook's. The table is the contract, not an approximation of [`crate::normal::normal_quantile`].
const T975: [(usize, f64); 13] = [
    (1, 12.71),
    (2, 4.31),
    (3, 3.19),
    (4, 2.78),
    (5, 2.58),
    (6, 2.45),
    (7, 2.37),
    (8, 2.31),
    (9, 2.27),
    (10, 2.23),
    (14, 2.15),
    (19, 2.10),
    (29, 2.05),
];

/// The critical value at `df` degrees of freedom: the largest tabulated row at or below it, and
/// infinite below the first row (no interval from fewer than two values).
pub fn t975(df: usize) -> f64 {
    T975.iter().rev().find(|&&(k, _)| df >= k).map_or(f64::INFINITY, |&(_, t)| t)
}

/// Mean of `xs` and the half-width of its 95% t-interval (sample standard deviation, n − 1), or
/// `None` under two values. The paired-ratio interval `ab` reports: a gain counts only when the
/// interval excludes 1.
pub fn t_interval(xs: &[f64]) -> Option<(f64, f64)> {
    if xs.len() < 2 {
        return None;
    }
    let n = xs.len() as f64;
    let m = xs.iter().sum::<f64>() / n;
    let sd = (xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
    Some((m, t975(xs.len() - 1) * sd / n.sqrt()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_table_never_understates_the_textbook() {
        // NIST/SEMATECH handbook, Student-t critical values, probability 0.975.
        let nist = [
            (1, 12.706),
            (2, 4.303),
            (3, 3.182),
            (5, 2.571),
            (7, 2.365),
            (9, 2.262),
            (10, 2.228),
            (11, 2.201),
            (14, 2.145),
            (19, 2.093),
            (29, 2.045),
            (30, 2.042),
        ];
        for (df, critical) in nist {
            assert!(t975(df) >= critical, "df {df}");
        }
        assert!(t975(1000) > 1.96, "a large sample keeps the last row, not the normal");
        assert!((1..100).map(t975).collect::<Vec<_>>().windows(2).all(|w| w[0] >= w[1]));
        assert_eq!(t975(0), f64::INFINITY);
        assert_eq!((t975(11), t975(13), t975(14), t975(15)), (2.23, 2.23, 2.15, 2.15));
    }

    #[test]
    fn t_interval_is_the_paired_ratio_interval() {
        assert_eq!(t_interval(&[1.0]), None);
        // Two values, df 1: mean 1.5, sd √0.5, half-width 12.71·√0.5/√2.
        let (mean, half) = t_interval(&[1.0, 2.0]).unwrap();
        assert_eq!(mean, 1.5);
        assert!((half - 12.71 * 0.5f64.sqrt() / 2f64.sqrt()).abs() < 1e-12);
        assert_eq!(t_interval(&[3.0, 3.0, 3.0]), Some((3.0, 0.0)));
    }

    #[test]
    fn running_and_sample_forms_agree_with_the_textbook() {
        let xs = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let mut m = Moments::default();
        xs.iter().for_each(|&x| m.add(x));
        assert_eq!(m.mean(), 5.0);
        // Population variance 4, n 8: z·√(4/8).
        assert!((m.half_width(1.96) - 1.96 * 0.5f64.sqrt()).abs() < 1e-12);
        let (mean, hw) = mean_half_width(&xs, 1.96);
        assert_eq!(mean, 5.0);
        // Unbiased variance 32/7.
        assert!((hw - 1.96 * (32.0 / 7.0 / 8.0f64).sqrt()).abs() < 1e-12);
        assert_eq!(Moments::default().mean(), 0.0);
        assert!(Moments { n: 1.0, sum: 3.0, sum_sq: 9.0 }.half_width(1.96).is_infinite());
        assert_eq!(mean_half_width(&[3.0], 1.96).0, 3.0);
        assert!(mean_half_width(&[], 1.96).1.is_infinite());
        // Equal values: rounding leaves the running variance near 0 (possibly below), never NaN.
        let hw = half_width(3.0, 0.3, 0.03, 1.96);
        assert!((0.0..1e-8).contains(&hw), "{hw}");
        assert_eq!(half_width(3.0, 0.3, 0.02, 1.96), 0.0, "a negative variance is floored at 0");
    }
}

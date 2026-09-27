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

#[cfg(test)]
mod tests {
    use super::*;

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

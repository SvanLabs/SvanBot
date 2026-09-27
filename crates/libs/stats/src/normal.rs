//! The standard normal distribution.

/// Inverse standard normal CDF (Acklam's rational approximation, relative error under 1.2e-9).
pub fn normal_quantile(p: f64) -> f64 {
    const A: [f64; 6] =
        [-3.969683028665376e1, 2.209460984245205e2, -2.759285104469687e2, 1.38357751867269e2, -3.066479806614716e1, 2.506628277459239];
    const B: [f64; 5] = [-5.447609879822406e1, 1.615858368580409e2, -1.556989798598866e2, 6.680131188771972e1, -1.328068155288572e1];
    const C: [f64; 6] =
        [-7.784894002430293e-3, -3.223964580411365e-1, -2.400758277161838, -2.549732539343734, 4.374664141464968, 2.938163982698783];
    const D: [f64; 4] = [7.784695709041462e-3, 3.224671290700398e-1, 2.445134137142996, 3.754408661907416];
    let tail = |q: f64| {
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5]) / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    let p = p.clamp(1e-300, 1.0 - 1e-16);
    if p < 0.02425 {
        tail((-2.0 * p.ln()).sqrt())
    } else if p > 1.0 - 0.02425 {
        -tail((-2.0 * (1.0 - p).ln()).sqrt())
    } else {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    }
}

/// Standard errors a one-sided verdict needs for a 2.5% chance of any false verdict among `tested`
/// comparisons (Bonferroni): 1.96 for one, about 3.55 for 130.
pub fn family_z(tested: usize) -> f64 {
    normal_quantile(1.0 - 0.025 / tested.max(1) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_match_known_values_and_the_family_widens() {
        assert!(normal_quantile(0.5).abs() < 1e-9);
        assert!((normal_quantile(0.975) - 1.959964).abs() < 1e-6);
        assert!((normal_quantile(0.025) + 1.959964).abs() < 1e-6);
        assert!((normal_quantile(0.999) - 3.090232).abs() < 1e-6);
        assert!((family_z(1) - 1.96).abs() < 1e-3);
        assert!((family_z(130) - 3.55).abs() < 0.01);
        assert_eq!(family_z(0), family_z(1));
    }
}

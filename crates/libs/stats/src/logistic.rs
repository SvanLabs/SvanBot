//! Logistic helpers: the fold-probability fits work in log-odds.

/// The logistic function `1 / (1 + e^−x)`.
pub fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// Log-odds of `p`, with `p` clamped to `[eps, 1 − eps]` first.
pub fn logit(p: f64, eps: f64) -> f64 {
    let p = p.clamp(eps, 1.0 - eps);
    (p / (1.0 - p)).ln()
}

/// Log-loss of predicting `p` for outcome `y`, with `p` clamped to `[eps, 1 − eps]` first.
pub fn log_loss(p: f64, y: bool, eps: f64) -> f64 {
    let p = p.clamp(eps, 1.0 - eps);
    if y { -p.ln() } else { -(1.0 - p).ln() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logit_and_sigmoid_invert_and_clamps_bound_the_loss() {
        for p in [0.01, 0.2, 0.5, 0.77, 0.99] {
            assert!((sigmoid(logit(p, 1e-4)) - p).abs() < 1e-12);
        }
        assert_eq!(logit(0.0, 1e-4), logit(1e-4, 1e-4));
        assert!((log_loss(0.5, true, 1e-6) - std::f64::consts::LN_2).abs() < 1e-15);
        assert!(log_loss(0.0, true, 1e-6).is_finite() && log_loss(1.0, false, 1e-6).is_finite());
        assert!((log_loss(0.0, true, 1e-6) - 1e6f64.ln()).abs() < 1e-6);
    }
}

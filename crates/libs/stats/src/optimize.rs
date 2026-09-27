//! One-dimensional minimisation.

/// Minimise a unimodal `f` on `[lo, hi]` by golden-section search over `iterations` steps; returns
/// the midpoint of the final bracket (60 steps shrink the bracket by 0.618^60 ≈ 3e-13).
pub fn golden_section_min(f: impl Fn(f64) -> f64, lo: f64, hi: f64, iterations: usize) -> f64 {
    let (mut a, mut b) = (lo, hi);
    let g = (5f64.sqrt() - 1.0) / 2.0;
    for _ in 0..iterations {
        let c = b - g * (b - a);
        let d = a + g * (b - a);
        if f(c) < f(d) { b = d } else { a = c }
    }
    (a + b) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_minimum_of_a_convex_function_and_respects_the_bracket() {
        assert!((golden_section_min(|x| (x - 0.3).powi(2), -1.0, 1.0, 60) - 0.3).abs() < 1e-9);
        // A minimum outside the bracket ends at the nearer edge.
        assert!((golden_section_min(|x| (x - 5.0).powi(2), -1.0, 1.0, 60) - 1.0).abs() < 1e-9);
    }
}

//! How the paired simulation's predicted edge carries over to a live probe.

/// A sim estimate, its live edge, and the live 95% half-width, all in bb/100.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub sim: f64,
    pub live: f64,
    pub radius: f64,
}

impl Point {
    pub fn is_valid(&self) -> bool {
        self.sim.is_finite()
            && self.live.is_finite()
            && self.radius.is_finite()
            && self.radius >= 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slope {
    pub value: f64,
    pub radius: f64,
    pub points: usize,
}

/// Weighted least squares through the origin, with weights inverse to live variance.
pub fn slope(points: &[Point]) -> Option<Slope> {
    if points.len() < 2 || points.iter().any(|point| !point.is_valid()) {
        return None;
    }
    // Scale the weights by the smallest informative radius squared to avoid dividing by a
    // tiny variance. A zero-width point with a nonzero sim dominates in the limit.
    let scale = points
        .iter()
        .filter(|point| point.sim != 0.0)
        .map(|point| point.radius)
        .reduce(f64::min)?;
    let (mut cross, mut square) = (0.0, 0.0);
    for point in points.iter().filter(|point| point.sim != 0.0) {
        let weight = if point.radius == 0.0 {
            1.0
        } else {
            (scale / point.radius).powi(2)
        };
        cross += weight * point.sim * point.live;
        square += weight * point.sim * point.sim;
    }
    let value = cross / square;
    let radius = scale / square.sqrt();
    (square.is_finite() && value.is_finite() && radius.is_finite()).then_some(Slope {
        value,
        radius,
        points: points.len(),
    })
}

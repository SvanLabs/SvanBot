//! Pooling and confidence intervals for independent paired evaluations.

/// Challenger-minus-champion result of a paired evaluation.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PairedResult {
    /// Hands compared.
    pub hands: u64,
    /// Challenger minus champion, big blinds per hand.
    pub mean_bb: f64,
    /// Standard error of `mean_bb`.
    pub se_bb: f64,
    /// Hands whose outcome differed between the two policies (0 = the change never mattered).
    pub differing: u64,
}

impl Default for PairedResult {
    /// No hands compared: no estimate, and no interval either (an empty result is not a precise one).
    fn default() -> PairedResult {
        PairedResult { hands: 0, mean_bb: 0.0, se_bb: f64::INFINITY, differing: 0 }
    }
}

impl PairedResult {
    /// Pool two independent evaluations (different deals) of the same pair.
    pub fn combine(&self, other: &PairedResult) -> PairedResult {
        if self.hands == 0 {
            return other.clone();
        }
        if other.hands == 0 {
            return self.clone();
        }
        let (n1, n2) = (self.hands as f64, other.hands as f64);
        let n = (n1 + n2).max(1.0);
        let mean = (self.mean_bb * n1 + other.mean_bb * n2) / n;
        let var_mean = (n1 * n1 * self.se_bb * self.se_bb + n2 * n2 * other.se_bb * other.se_bb) / (n * n);
        PairedResult { hands: self.hands + other.hands, mean_bb: mean, se_bb: var_mean.sqrt(), differing: self.differing + other.differing }
    }

    /// Lower end of the 95% interval (the promotion gate).
    pub fn lower_95(&self) -> f64 {
        self.mean_bb - 1.96 * self.se_bb
    }
    /// Upper end of the 95% interval.
    pub fn upper_95(&self) -> f64 {
        self.mean_bb + 1.96 * self.se_bb
    }
}

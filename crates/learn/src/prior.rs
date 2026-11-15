//! The population prior for a rate, fitted rather than fixed.
//!
//! v10 shrank every opponent's rate toward the population with fixed pseudo-counts (150, 15, 12
//! and 40). Empirical Bayes fits them instead: the prior is a beta distribution whose mean and
//! strength maximise the beta-binomial marginal likelihood of every player's tally, so a rate is
//! shrunk exactly as much as the players' real spread justifies (`docs/research/2026-11/
//! 02-range-calibration.md`, option 1).

/// A beta prior on a rate: its mean, and its strength as a pseudo-count of trials.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BetaPrior {
    pub mean: f64,
    pub strength: f64,
}

impl BetaPrior {
    /// A player's rate after `successes` in `trials`, shrunk toward the population mean.
    pub fn shrink(&self, successes: u32, trials: u32) -> f64 {
        (f64::from(successes) + self.mean * self.strength) / (f64::from(trials) + self.strength)
    }
}

/// The strongest prior a fit returns: players that do not differ get this, not infinity.
pub const MAX_STRENGTH: f64 = 100_000.0;

/// Fits a prior to every player's `(successes, trials)` for one rate, or `None` without any trials.
pub fn fit_prior(tallies: &[(u32, u32)]) -> Option<BetaPrior> {
    let (wins, trials) = tallies.iter().fold((0u64, 0u64), |(w, n), &(s, t)| {
        (w + u64::from(s), n + u64::from(t))
    });
    if trials == 0 {
        return None;
    }
    let pooled = (wins as f64 / trials as f64).clamp(1e-4, 1.0 - 1e-4);
    let mut prior = BetaPrior {
        mean: pooled,
        strength: 10.0,
    };
    for _ in 0..4 {
        let log_strength = golden(0.5f64.ln(), MAX_STRENGTH.ln(), |log_k| {
            marginal(tallies, prior.mean, log_k.exp())
        });
        prior.strength = log_strength.exp();
        prior.mean = golden(1e-4, 1.0 - 1e-4, |mean| {
            marginal(tallies, mean, prior.strength)
        });
    }
    Some(prior)
}

/// The beta-binomial log marginal likelihood, without the binomial terms, which do not depend on
/// the prior.
fn marginal(tallies: &[(u32, u32)], mean: f64, strength: f64) -> f64 {
    let (a, b) = (mean * strength, (1.0 - mean) * strength);
    let base = ln_beta(a, b);
    tallies
        .iter()
        .filter(|&&(_, n)| n > 0)
        .map(|&(s, n)| ln_beta(a + f64::from(s), b + f64::from(n - s)) - base)
        .sum()
}

/// The maximum of a unimodal `f` on `[low, high]`, by golden-section search.
fn golden(mut low: f64, mut high: f64, f: impl Fn(f64) -> f64) -> f64 {
    let ratio = (5f64.sqrt() - 1.0) / 2.0;
    let mut x1 = high - ratio * (high - low);
    let mut x2 = low + ratio * (high - low);
    let (mut f1, mut f2) = (f(x1), f(x2));
    for _ in 0..80 {
        if f1 < f2 {
            low = x1;
            x1 = x2;
            f1 = f2;
            x2 = low + ratio * (high - low);
            f2 = f(x2);
        } else {
            high = x2;
            x2 = x1;
            f2 = f1;
            x1 = high - ratio * (high - low);
            f1 = f(x1);
        }
    }
    (low + high) / 2.0
}

fn ln_beta(a: f64, b: f64) -> f64 {
    ln_gamma(a) + ln_gamma(b) - ln_gamma(a + b)
}

/// The log of the gamma function for `x > 0`, by Lanczos' approximation (g = 7, nine terms),
/// good to about 15 digits.
pub fn ln_gamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        // The reflection formula.
        return std::f64::consts::PI.ln()
            - (std::f64::consts::PI * x).sin().abs().ln()
            - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut sum = C[0];
    for (i, &c) in C.iter().enumerate().skip(1) {
        sum += c / (x + i as f64);
    }
    let t = x + G + 0.5;
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + sum.ln()
}

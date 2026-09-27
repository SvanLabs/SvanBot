//! Per-opponent river sizing tells (0223): how each opponent's river bet size tracks the strength
//! of the hand they bet, against the pool curve the range model already applies.
//!
//! The 0223 study found the pool barely sizes by strength below a pot-sized bet, while individual
//! opponents differ well beyond sampling noise: some bet bigger with stronger hands, some the
//! reverse. A tell `k` scales the value and bluff shares of a player's river betting range by
//! `(size / SIZE_TELL_REF)^-k` ([`crate::oprange::size_tell_factor`]). Each player's `k` maximises
//! the likelihood of the hands they showed after betting the river, shrunk toward 0 by sample
//! size, and the whole set is installed only while it raises the held-out likelihood of newer
//! showdowns at 95%.

use crate::calibrate::ShowdownSample;
use crate::oprange::RangeParams;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Candidate tells searched per player.
pub const GRID: [f32; 17] = [-2.0, -1.75, -1.5, -1.25, -1.0, -0.75, -0.5, -0.25, 0.0, 0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
/// Shrinkage: pseudo-samples at tell 0, so a player with few river showdowns barely moves.
pub const PRIOR: f64 = 300.0;
/// Fewest river-bet showdowns before a player gets a tell.
pub const MIN_SAMPLES: usize = 8;
/// Fewest held-out samples the gate judges on.
pub const MIN_HELD_OUT: usize = 300;

/// The fitted tells and the held-out evidence that decides whether they are installed.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SizeTellFit {
    /// Mean held-out log-likelihood gain per river-bet showdown (nats).
    pub gain: f64,
    /// 95% half-width of `gain`.
    pub half_width: f64,
    /// Held-out river-bet showdowns.
    pub n: usize,
    /// River-bet showdowns the installed tells were fitted on (every sample).
    pub fitted_on: usize,
    /// Whether the tells are installed.
    pub active: bool,
    /// Tell per opponent (players without one play the pool curve).
    pub tells: HashMap<String, f64>,
}

/// Each player's tell from `samples`: the grid value with the highest summed likelihood, shrunk by
/// `n / (n + PRIOR)`.
pub fn player_tells(samples: &[&ShowdownSample], rp: &RangeParams) -> HashMap<String, f64> {
    player_tells_with(samples, rp, PRIOR)
}

/// [`player_tells`] with an explicit shrinkage prior.
pub fn player_tells_with(samples: &[&ShowdownSample], rp: &RangeParams, prior: f64) -> HashMap<String, f64> {
    let mut by: HashMap<&str, Vec<&ShowdownSample>> = HashMap::new();
    for s in samples {
        by.entry(s.name()).or_default().push(s);
    }
    by.into_par_iter()
        .filter(|(_, v)| v.len() >= MIN_SAMPLES)
        .filter_map(|(name, v)| {
            let ll = |k: f32| v.iter().map(|s| s.log_likelihood_with_tell(rp, k)).sum::<f64>();
            let (best, _) = GRID.iter().map(|&k| (k, ll(k))).fold((0.0f32, f64::NEG_INFINITY), |a, b| if b.1 > a.1 { b } else { a });
            let n = v.len() as f64;
            let tell = f64::from(best) * n / (n + prior);
            (tell.abs() > 1e-3).then(|| (name.to_string(), tell))
        })
        .collect()
}

/// Fit on `samples` (oldest first): tells learned on the older 70% of river-bet showdowns are
/// scored on the newest 30%; the installed tells are then refitted on all of them.
pub fn fit(samples: &[ShowdownSample], rp: &RangeParams) -> SizeTellFit {
    fit_with(samples, rp, PRIOR)
}

/// [`fit`] with an explicit shrinkage prior (for `review sizing-fit`).
pub fn fit_with(samples: &[ShowdownSample], rp: &RangeParams, prior: f64) -> SizeTellFit {
    let river: Vec<&ShowdownSample> = samples.iter().filter(|s| s.bet_the_river()).collect();
    let split = river.len() * 7 / 10;
    let train_tells = player_tells_with(&river[..split], rp, prior);
    let gains: Vec<f64> = river[split..]
        .par_iter()
        .map(|s| match train_tells.get(s.name()) {
            Some(&k) => s.log_likelihood_with_tell(rp, k as f32) - s.log_likelihood_with_tell(rp, 0.0),
            None => 0.0,
        })
        .collect();
    let n = gains.len();
    let gain = gains.iter().sum::<f64>() / n.max(1) as f64;
    let var = if n > 1 { gains.iter().map(|g| (g - gain).powi(2)).sum::<f64>() / (n - 1) as f64 } else { f64::INFINITY };
    let half_width = 1.96 * (var / n.max(1) as f64).sqrt();
    SizeTellFit {
        gain,
        half_width,
        n,
        fitted_on: river.len(),
        active: n >= MIN_HELD_OUT && gain - half_width > 0.0,
        tells: player_tells_with(&river, rp, prior),
    }
}

/// The tells to play with from a fit (none unless active).
pub fn installed(fit: Option<SizeTellFit>) -> std::sync::Arc<HashMap<String, f32>> {
    let fit = fit.filter(|f| f.active);
    std::sync::Arc::new(fit.map(|f| f.tells.into_iter().map(|(k, v)| (k, v as f32)).collect()).unwrap_or_default())
}

/// Least-squares slope of strength on ln(size) over `(size, strength)` points, with its standard
/// error; `None` for fewer than 3 points or a single size. Sizes are floored at 5% of the pot.
pub fn log_size_slope(points: &[(f64, f64)]) -> Option<(f64, f64)> {
    let n = points.len() as f64;
    if n < 3.0 {
        return None;
    }
    let xs: Vec<f64> = points.iter().map(|p| p.0.max(0.05).ln()).collect();
    let mx = xs.iter().sum::<f64>() / n;
    let my = points.iter().map(|p| p.1).sum::<f64>() / n;
    let sxx: f64 = xs.iter().map(|x| (x - mx).powi(2)).sum();
    if sxx < 1e-9 {
        return None;
    }
    let sxy: f64 = xs.iter().zip(points).map(|(x, p)| (x - mx) * (p.1 - my)).sum();
    let slope = sxy / sxx;
    let resid: f64 = xs.iter().zip(points).map(|(x, p)| (p.1 - my - slope * (x - mx)).powi(2)).sum();
    Some((slope, (resid / (n - 2.0) / sxx).sqrt()))
}

/// Cochran's heterogeneity test of per-group slopes against a pooled one: `z_scores` are each
/// group's `(slope - pooled) / se`. Returns `(Q, degrees of freedom, z)`, with Q mapped to an
/// approximate standard normal by the Wilson-Hilferty transform; z above 1.64 means the groups
/// differ beyond sampling noise at 95% (one-sided).
pub fn heterogeneity(z_scores: &[f64]) -> (f64, f64, f64) {
    let q: f64 = z_scores.iter().map(|z| z * z).sum();
    let df = z_scores.len().saturating_sub(1) as f64;
    let z = if df > 0.0 { ((q / df).cbrt() - (1.0 - 2.0 / (9.0 * df))) / (2.0 / (9.0 * df)).sqrt() } else { 0.0 };
    (q, df, z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{HandSummary, ModelStore};
    use sv10_cards::cards::{Card, parse_cards};
    use sv10_engine::engine::{ActionKind, ActionRecord, Street};

    fn card(s: &str) -> Card {
        Card::parse(s).unwrap()
    }

    /// Heads-up hand: `name` checks the flop and turn, then bets `size` of the pot on the river
    /// holding `hole`, and shows it.
    fn river_bet(name: &str, hole: [&str; 2], size: f64) -> HandSummary {
        let pot = 400;
        let bet = (size * pot as f64) as i64;
        let rec = |seat, street, kind, to, pot_before, to_call_before| ActionRecord {
            seat,
            street,
            kind,
            to,
            pot_before,
            to_call_before,
            bet_before: 0,
            full_raise: kind == ActionKind::Raise,
            think_ms: None,
            street_open: false,
        };
        let mut history =
            vec![rec(0, Street::Preflop, ActionKind::Call, 20, 30, 10), rec(1, Street::Preflop, ActionKind::Check, 20, 40, 0)];
        for street in [Street::Flop, Street::Turn] {
            history.push(rec(1, street, ActionKind::Check, 0, 40, 0));
            history.push(rec(0, street, ActionKind::Check, 0, 40, 0));
        }
        history.push(rec(1, Street::River, ActionKind::Check, 0, pot, 0));
        history.push(rec(0, Street::River, ActionKind::Raise, bet, pot, 0));
        history.push(rec(1, Street::River, ActionKind::Call, bet, pot + bet, bet));
        HandSummary {
            players: vec![(0, name.to_string()), (1, "hero".to_string())],
            button: 0,
            bb: 20,
            history,
            board: parse_cards(&["2c", "7d", "9h", "Js", "3c"]).unwrap(),
            shown: vec![(0, [card(hole[0]), card(hole[1])])],
            stacks: vec![(0, 10_000), (1, 10_000)],
        }
    }

    #[test]
    fn log_size_slope_recovers_a_known_line_and_rejects_degenerate_input() {
        // strength = 0.8 + 0.05 ln(size), no noise: slope 0.05, zero standard error.
        let pts: Vec<(f64, f64)> = [0.25, 0.5, 1.0, 2.0, 4.0].iter().map(|&s| (s, 0.8 + 0.05 * f64::ln(s))).collect();
        let (b, se) = log_size_slope(&pts).unwrap();
        assert!((b - 0.05).abs() < 1e-12 && se < 1e-9, "{b} {se}");
        assert_eq!(log_size_slope(&pts[..2]), None, "too few points");
        assert_eq!(log_size_slope(&[(1.0, 0.1), (1.0, 0.9), (1.0, 0.5)]), None, "one size only");
    }

    #[test]
    fn heterogeneity_is_quiet_for_noise_and_loud_for_real_differences() {
        // z-scores of one: Q equals its expectation (df + 1 here), z near zero.
        let (q, df, z) = heterogeneity(&[1.0; 101]);
        assert_eq!((q, df), (101.0, 100.0));
        assert!(z.abs() < 0.3, "{z}");
        // A real spread: Q 256 on 102 df is far beyond noise.
        let spread: Vec<f64> = (0..103).map(|i| if i % 2 == 0 { 1.58 } else { -1.58 }).collect();
        let (q, df, z) = heterogeneity(&spread);
        assert!((q - 257.1).abs() < 0.5 && df == 102.0 && z > 7.0, "{q} {df} {z}");
        assert_eq!(heterogeneity(&[2.0]).2, 0.0, "one group: no test");
    }

    #[test]
    fn an_untold_player_replays_exactly_as_before() {
        assert_eq!(crate::oprange::size_tell_factor(0.0, 3.0, Street::River), 1.0);
        assert_eq!(crate::oprange::size_tell_factor(1.0, 3.0, Street::Turn), 1.0);
        let f = crate::oprange::size_tell_factor(1.0, 2.0 * crate::oprange::SIZE_TELL_REF, Street::River);
        assert!((f - 0.5).abs() < 1e-6, "a positive tell makes a bigger bet stronger: {f}");
        assert_eq!(crate::oprange::size_tell_factor(2.0, 1e6, Street::River), 0.25, "bounded");
    }

    #[test]
    fn a_player_who_bets_big_with_nuts_and_small_with_air_gets_a_positive_tell() {
        let models = ModelStore::default();
        let mut hands = Vec::new();
        for i in 0..40 {
            hands.push(if i % 2 == 0 { river_bet("teller", ["9s", "9d"], 2.0) } else { river_bet("teller", ["5h", "4h"], 0.25) });
        }
        let samples: Vec<ShowdownSample> =
            hands.iter().flat_map(|h| crate::calibrate::samples_from_hand(h, &models, &["hero".to_string()])).collect();
        assert_eq!(samples.len(), 40);
        assert!(samples.iter().all(|s| s.bet_the_river()));
        let refs: Vec<&ShowdownSample> = samples.iter().collect();
        let raw = player_tells_with(&refs, &RangeParams::DEFAULT, 0.0);
        assert!(raw["teller"] > 0.2, "unshrunk: {raw:?}");
        let shrunk = player_tells(&refs, &RangeParams::DEFAULT);
        assert!(shrunk["teller"] > 0.0 && shrunk["teller"] < raw["teller"] * 0.2, "40 samples barely move a tell: {shrunk:?}");
        let fit = fit(&samples, &RangeParams::DEFAULT);
        assert!(fit.gain > 0.0, "{fit:?}");
        assert!(!fit.active, "12 held-out samples are too few to install: {fit:?}");
        let active = SizeTellFit { active: true, ..fit.clone() };
        assert!((installed(Some(active))["teller"] as f64 - fit.tells["teller"]).abs() < 1e-6);
        assert!(installed(Some(fit)).is_empty() && installed(None).is_empty());
    }
}

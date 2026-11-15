//! The adaptivity class (`GLOSSARY.md`): does an opponent play one fixed strategy, switch
//! between strategies, or learn from how we play?
//!
//! - **Switching** is a change point in their per-hand tallies: the split of the series into two
//!   rates that most raises the binomial likelihood, kept when its log-ratio reaches a threshold.
//! - **Learning** is "responds to us": their fold rate against our bets differs from their fold
//!   rate against everyone else's, and the gap grows from the first half of the series to the
//!   second (`docs/research/2026-11/03-opponent-response.md`, option 2).
//!
//! Learning wins over switching: a player who learns from us also changes over time.

use svanbot_strategy::read::Class;

/// The log-likelihood ratio a change point must reach. Splits are searched at every point, so
/// this is set well above a single test's: false alarms only cost exploitation.
pub const CHANGE_THRESHOLD: f64 = 10.0;

/// Observations a side of a change point needs.
pub const MIN_SIDE: usize = 30;

/// The two-sided z a "responds to us" gap must reach.
pub const Z: f64 = 3.0;

/// A change in a rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Change {
    /// The first observation of the new rate.
    pub at: usize,
    /// The log-likelihood ratio of two rates against one.
    pub log_ratio: f64,
}

/// The strongest change point in `series`, if its log-ratio reaches `threshold` with at least
/// `min_side` observations on each side.
pub fn change_point(series: &[bool], threshold: f64, min_side: usize) -> Option<Change> {
    let n = series.len();
    if n < 2 * min_side.max(1) {
        return None;
    }
    let total = series.iter().filter(|&&x| x).count();
    let whole = binomial(total, n);
    let mut left = series[..min_side].iter().filter(|&&x| x).count();
    let mut best: Option<Change> = None;
    for at in min_side..=n - min_side {
        if at > min_side {
            left += usize::from(series[at - 1]);
        }
        let log_ratio = binomial(left, at) + binomial(total - left, n - at) - whole;
        if best.is_none_or(|b| log_ratio > b.log_ratio) {
            best = Some(Change { at, log_ratio });
        }
    }
    best.filter(|change| change.log_ratio >= threshold)
}

/// The maximised binomial log-likelihood of `k` in `n`.
fn binomial(k: usize, n: usize) -> f64 {
    let term = |count: usize| {
        if count == 0 {
            0.0
        } else {
            count as f64 * (count as f64 / n as f64).ln()
        }
    };
    term(k) + term(n - k)
}

/// Whether the opponent answers our bets differently from everyone else's, and more so over
/// time. Each series holds, in order, whether they folded to a bet.
pub fn responds_to_us(vs_us: &[bool], vs_others: &[bool]) -> bool {
    let rate = |xs: &[bool]| xs.iter().filter(|&&x| x).count() as f64 / xs.len().max(1) as f64;
    if vs_us.len() < 2 * MIN_SIDE || vs_others.len() < MIN_SIDE {
        return false;
    }
    let (ours, theirs) = (rate(vs_us), rate(vs_others));
    let pooled = (ours * vs_us.len() as f64 + theirs * vs_others.len() as f64)
        / (vs_us.len() + vs_others.len()) as f64;
    let se = (pooled * (1.0 - pooled) * (1.0 / vs_us.len() as f64 + 1.0 / vs_others.len() as f64))
        .sqrt();
    if se == 0.0 || ((ours - theirs) / se).abs() < Z {
        return false;
    }
    let (early, late) = vs_us.split_at(vs_us.len() / 2);
    let (gap_early, gap_late) = (rate(early) - theirs, rate(late) - theirs);
    gap_late.abs() > gap_early.abs() && gap_late.signum() == (ours - theirs).signum()
}

/// The opponent's class from their per-hand tallies and their folds to our bets and to others'.
/// A switch also gives where the new strategy began, so their reads can restart there.
pub fn classify(tallies: &[bool], vs_us: &[bool], vs_others: &[bool]) -> (Class, Option<Change>) {
    if responds_to_us(vs_us, vs_others) {
        return (Class::Learning, None);
    }
    match change_point(tallies, CHANGE_THRESHOLD, MIN_SIDE) {
        Some(change) => (Class::Switching, Some(change)),
        None => (Class::Fixed, None),
    }
}

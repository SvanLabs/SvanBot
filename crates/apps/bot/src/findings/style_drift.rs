//! The style-drift instrument (0332), split out of `findings.rs` (the 500-line rule).
//!
//! It reads our own preflop mix — the share of hands whose first preflop decision was each action, the
//! last day against the six days before — and flags a shift too large and too consistent to be cards.
//! On 2026-09-26 a self-calibration rule change took first-in raising from 40% of hands to 3% within an
//! hour and nothing noticed; this is what notices.
//!
//! #321: it reports **one finding per decision point**, not one per action. The actions are the
//! outcomes of a single choice, so a rise in one and a fall in another are the same event — reported
//! apart they took the panel's two highest-severity slots and read as two findings.

use super::Finding;

/// Smallest change in a first-preflop-action share, in percentage points, that is a style drift.
pub const DRIFT_POINTS: f64 = 5.0;
/// Standard errors a drift must clear: both windows hold thousands of hands, so only a real shift passes.
pub const DRIFT_Z: f64 = 5.0;

/// What one action's share did, `(action, now %, before %, points)`.
type Move<'a> = (&'a str, f64, f64, f64);

/// The style-drift finding: the share of hands whose first preflop decision was each action, the last
/// day against the days before. `P1`: a risk to look at, never a ticket on its own.
///
/// One finding per decision point (#321): the largest move names it, every other action that moved is
/// named in its evidence, and the finding's id is the decision point (`style-drift:preflop`) rather
/// than one action within it — so a shift that moves two actions files, ages and clears as one thing.
pub fn style_drift(recent: &[(String, i64)], baseline: &[(String, i64)]) -> Vec<Finding> {
    let total = |w: &[(String, i64)]| w.iter().map(|r| r.1).sum::<i64>().max(0) as f64;
    let (nr, nb) = (total(recent), total(baseline));
    if nr < 1.0 || nb < 1.0 {
        return Vec::new();
    }
    let share = |w: &[(String, i64)], a: &str| w.iter().filter(|r| r.0 == a).map(|r| r.1).sum::<i64>() as f64;
    let mut actions: Vec<&str> = recent.iter().chain(baseline).map(|r| r.0.as_str()).collect();
    actions.sort_unstable();
    actions.dedup();
    let mut moved: Vec<Move> = actions
        .into_iter()
        .filter_map(|a| {
            let (pr, pb) = (share(recent, a) / nr, share(baseline, a) / nb);
            let pooled = (share(recent, a) + share(baseline, a)) / (nr + nb);
            let se = (pooled * (1.0 - pooled) * (1.0 / nr + 1.0 / nb)).sqrt();
            let points = (pr - pb) * 100.0;
            (points.abs() >= DRIFT_POINTS && se > 0.0 && ((pr - pb) / se).abs() >= DRIFT_Z).then_some((a, pr * 100.0, pb * 100.0, points))
        })
        .collect();
    // The largest move names the finding; the rest are the same shift seen from its other side.
    moved.sort_by(|a, b| b.3.abs().total_cmp(&a.3.abs()));
    let Some(&(top, now_pct, was_pct, points)) = moved.first() else {
        return Vec::new();
    };
    let also: Vec<String> = moved[1..].iter().map(|(a, pr, pb, _)| format!("{a} {} from {pb:.1}% to {pr:.1}%", verb(pr - pb))).collect();
    let evidence = format!(
        "{nr:.0} hands in the last day against {nb:.0} in the six days before{}",
        if also.is_empty() { String::new() } else { format!("; also moved: {}", also.join(", ")) }
    );
    vec![Finding::new(
        "style-drift:preflop",
        "P1",
        &format!("first preflop action shifted: {top} {} from {was_pct:.1}% of hands to {now_pct:.1}% in the last day", verb(points)),
        evidence,
        points,
        0.0,
    )]
}

/// Which way a share moved, so a row reads as a sentence rather than as a signed number.
fn verb(points: f64) -> &'static str {
    if points >= 0.0 { "rose" } else { "fell" }
}

/// The explanation the drift findings share (#321), stated once above them: what a shift this size is
/// evidence of, and what a reader should do about it.
pub fn style_drift_legend() -> String {
    "A shift this size comes from a rule, parameter or calibration change, not the cards — check the \
     releases, the champion and `review margins` (0332). The actions are one decision's outcomes, so a \
     rise in one and a fall in another are one event: the largest move names the finding and the rest \
     are in its evidence."
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0332: the 2026-09-26 shift (first-in raising 40% of hands to 3% within an hour) is a finding; the
    /// day-to-day wobble of a stable style is not, and neither is a thin window.
    ///
    /// #321: it is **one** finding, naming every action that moved — the case that produced two `P1`
    /// rows for one event.
    #[test]
    fn a_preflop_style_shift_is_one_finding_and_ordinary_wobble_is_not() {
        let w =
            |call: i64, raise: i64, fold: i64| vec![("call".to_string(), call), ("raise".to_string(), raise), ("fold".to_string(), fold)];
        let shifted = style_drift(&w(11_900, 350, 250), &w(40_000, 26_000, 800));
        assert_eq!(shifted.len(), 1, "call up and raise down are one event: {shifted:?}");
        assert_eq!(shifted[0].id, "style-drift:preflop", "the finding is the decision point, not one action in it");
        assert_eq!(shifted[0].severity, "P1", "a risk to look at, not a filed ticket");
        assert!(shifted[0].value < -30.0, "the largest move names it: {shifted:?}");
        // The title names the biggest move, which is the fall in raising here; the evidence names the
        // rise in calling that goes with it.
        assert!(shifted[0].title.contains("raise fell") && shifted[0].title.contains("2.8%"), "{:?}", shifted[0]);
        assert!(
            shifted[0].evidence.contains("also moved") && shifted[0].evidence.contains("call rose from 59.9% to 95.2%"),
            "{:?}",
            shifted[0]
        );
        // A stable style: 61/37/2 against 60/38/2 over thousands of hands.
        assert!(style_drift(&w(7_320, 4_440, 240), &w(40_000, 25_300, 1_330)).is_empty());
        // A large but thin shift is not evidence either way.
        assert!(style_drift(&w(8, 1, 1), &w(40_000, 26_000, 800)).is_empty());
        assert!(style_drift(&[], &w(40_000, 26_000, 800)).is_empty(), "nothing recent: nothing to compare");
    }

    /// #321: the legend is the paragraph the rows used to repeat, and it is where a reader is told what
    /// a shift means — so it must keep naming the tools and the issue that established it.
    #[test]
    fn the_legend_says_what_a_shift_means() {
        let legend = style_drift_legend();
        for want in ["rule, parameter or calibration change", "`review margins`", "0332"] {
            assert!(legend.contains(want), "no {want:?} in {legend}");
        }
    }
}

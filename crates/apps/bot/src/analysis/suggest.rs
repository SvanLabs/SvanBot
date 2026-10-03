//! Rule-based study notes for the leak finder: each names the evidence, the likely cause,
//! and what the bot does (or can be tuned to do) about it.

use super::{Row, Tally, Tripwire, street_name};
use serde_json::{Value, json};

/// A bb rate for an evidence string: signed with `decimals`, or a dash with no priced hand.
fn rate_or_dash(v: Option<f64>, decimals: usize) -> String {
    match v {
        Some(x) => format!("{:+.*}", decimals, x),
        None => "—".to_string(),
    }
}

/// Rule-based study notes from the findings: each names the evidence, the likely cause, and
/// what the bot does (or can be tuned to do) about it.
#[allow(clippy::too_many_arguments)]
pub(super) fn suggest(
    lines: &[Row],
    outcomes: &[Row],
    positions: &[Row],
    trips: &[Tripwire],
    opp: &[Value],
    calibration: Option<&Value>,
    overall_bb: Tally,
    aggressive_hands: [usize; 4],
) -> Vec<Value> {
    let mut out = Vec::new();
    let mut push = |severity: &str, title: String, evidence: String, action: String| {
        out.push(json!({"severity": severity, "title": title, "evidence": evidence, "action": action}));
    };
    // A line that ends in a fold always loses what was put in, so fold-ending results are judged
    // by how often aggression gets abandoned, not by their (necessarily negative) chips.
    for r in outcomes.iter().filter(|r| r.key.starts_with("bet_fold_")) {
        let street = ["preflop", "flop", "turn", "river"].iter().position(|s| r.key.ends_with(s)).unwrap_or(0);
        let base = aggressive_hands[street].max(1);
        let rate = r.hands as f64 / base as f64;
        // Opening then folding to a 3-bet is routine up to roughly a third of the time.
        let limit = if street == 0 { 0.15 } else { 0.25 };
        if rate > limit && r.hands >= 20 {
            push(
                "medium",
                format!("{:.0}% of our {} bets/raises end in a fold", rate * 100.0, street_name(street)),
                format!("{} of {} hands, {:+} chips ({} bb/100 of the total)", r.hands, base, r.total_chips, rate_or_dash(r.share_bb100, 1)),
                "Aggression is abandoned too often: raising hands that cannot continue when played back at. The EV model prices raise-backs (raise_risk, tuned by the learner); if this stays high, bet fewer medium-strength hands or size smaller on this street.".into(),
            );
        }
    }
    if let (Some(won), Some(lost)) = (outcomes.iter().find(|r| r.key == "showdown_won"), outcomes.iter().find(|r| r.key == "showdown_lost"))
    {
        let n = (won.hands + lost.hands).max(1);
        let win_rate = won.hands as f64 / n as f64;
        push(
            if win_rate < 0.45 && n >= 100 { "medium" } else { "info" },
            format!(
                "Showdowns: win {:.0}% of {} for {} bb/100 net",
                win_rate * 100.0,
                n,
                match (won.share_bb100, lost.share_bb100) {
                    (Some(a), Some(b)) => format!("{:+.0}", a + b),
                    _ => "—".to_string(),
                }
            ),
            format!("Won {} ({:+} chips), lost {} ({:+} chips)", won.hands, won.total_chips, lost.hands, lost.total_chips),
            if win_rate < 0.45 {
                "Reaching showdown behind too often: calling down too light. Tighten river calls against players who rarely bluff.".into()
            } else {
                "Healthy: we reach showdown mostly with the best hand.".into()
            },
        );
    }
    // Losing lines that did not end in a fold are genuine leaks (calling down, betting into better hands).
    for r in lines.iter().filter(|r| r.high_bb.is_some_and(|h| h < 0.0) && r.hands >= 40 && !r.key.ends_with('F')).take(4) {
        push(
            "high",
            format!("Losing line: {}", r.label),
            format!("{} hands, {} bb per hand (95% {}..{}), {} bb/100 of the total", r.hands, rate_or_dash(r.bb_per_hand, 2), rate_or_dash(r.low_bb, 2), rate_or_dash(r.high_bb, 2), rate_or_dash(r.share_bb100, 0)),
            "Significantly negative without folding, so chips go in behind. Replay the biggest losses of this line; if the chosen action showed a high EV each time, the opponent response model is too optimistic here.".into(),
        );
    }
    if let Some(worst) = positions.iter().filter(|r| r.hands >= 100).min_by(|a, b| {
        a.bb_per_hand.unwrap_or(f64::INFINITY).partial_cmp(&b.bb_per_hand.unwrap_or(f64::INFINITY)).unwrap_or(std::cmp::Ordering::Equal)
    }) {
        let blinds = worst.key == "SB" || worst.key == "BB";
        push(
            if worst.high_bb.is_some_and(|h| h < 0.0) && !blinds { "medium" } else { "info" },
            format!("Weakest position: {} ({} bb/100)", worst.key, rate_or_dash(worst.bb_per_hand.map(|m| m * 100.0), 1)),
            format!(
                "{} hands (95% {}..{} bb/100)",
                worst.hands,
                rate_or_dash(worst.low_bb.map(|l| l * 100.0), 1),
                rate_or_dash(worst.high_bb.map(|h| h * 100.0), 1)
            ),
            if blinds {
                "Blinds lose money for every player (forced bets); judge them against each other, not against zero.".into()
            } else {
                "A late or middle position losing money usually means opening too wide or c-betting too often from it; compare the Starting hand library for this seat.".into()
            },
        );
    }
    for t in trips {
        if t.flagged {
            push(
                "high",
                format!("Opponents are targeting our {} folds", t.street),
                format!(
                    "They bet into us {:.0}% vs {:.0}% elsewhere (z={:+.1}); we fold {:.0}% vs MDF {:.0}%",
                    t.bet_into_us * 100.0,
                    t.bet_elsewhere * 100.0,
                    t.z,
                    t.our_fold * 100.0,
                    t.mdf_fold * 100.0
                ),
                "Call wider with bluff-catchers on this street (lower call_margin), since their betting range now contains more bluffs."
                    .into(),
            );
        } else if t.our_fold > t.mdf_fold + 0.15 && t.faced >= 30 {
            push(
                "info",
                format!("We over-fold the {} by game theory, and that is correct here", t.street),
                format!(
                    "Fold {:.0}% vs MDF {:.0}%, but opponents bet into us no more than elsewhere (z={:+.1})",
                    t.our_fold * 100.0,
                    t.mdf_fold * 100.0,
                    t.z
                ),
                "Their bets are value-heavy; keep exploiting. The tripwire flags this if it changes.".into(),
            );
        }
    }
    for o in opp.iter().filter(|o| o["beats_us"].as_bool() == Some(true)) {
        push(
            "medium",
            format!("{} beats us", o["name"].as_str().unwrap_or("?")),
            format!("{:+.0} bb/100 over {} hands", o["bb100"].as_f64().unwrap_or(0.0), o["hands"]),
            "Table selection already treats this player as tough and never seeks their table.".into(),
        );
    }
    if let Some(cal) = calibration.and_then(|c| c.as_object()) {
        let mut worst: Vec<(&String, f64, i64)> = cal
            .iter()
            .filter_map(|(k, v)| Some((k, v["bias_bb"].as_f64()?, v["n"].as_i64().unwrap_or(0))))
            .filter(|(_, b, _)| *b < -0.5)
            .collect();
        worst.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        for (k, b, n) in worst.into_iter().take(3) {
            push(
                "info",
                format!("EV model over-optimistic in {k}"),
                format!("Self-calibration subtracts {:.1} bb from this spot ({n} decisions)", -b),
                "Already corrected automatically; persistent large corrections point at the opponent-response model for this spot.".into(),
            );
        }
    }
    if overall_bb.n >= 500.0 && overall_bb.mean() - overall_bb.half_width(1.96) > 0.0 {
        push(
            "info",
            format!("Winning significantly: {:+.0} bb/100", overall_bb.mean() * 100.0),
            format!(
                "{} hands, 95% lower bound {:+.0} bb/100",
                overall_bb.n as i64,
                (overall_bb.mean() - overall_bb.half_width(1.96)) * 100.0
            ),
            "Volume and uptime are now the biggest levers on the leaderboard.".into(),
        );
    }
    let rank = |s: &Value| match s["severity"].as_str() {
        Some("high") => 0,
        Some("medium") => 1,
        _ => 2,
    };
    out.sort_by_key(rank);
    out
}

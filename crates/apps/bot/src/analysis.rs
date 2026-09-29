//! Leak finder: which lines, positions and opponents cost the fleet chips, whether opponents
//! exploit our tendencies, how results trend over time, and what to change — computed from
//! svanbot10's own recorded hands (downloaded past hands were played by older bots, so they
//! are not used here).

use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use sv10_core::engine::{ActionKind, ActionRecord, Street};
use sv10_core::model::{HandSummary, aggressive};
use sv10_core::situation::position_of;
use sv10_store::store::Store;

/// Running mean and 95% interval of chips per hand (`sv10_stats`, 0231).
type Tally = sv10_stats::moments::Moments;

#[derive(Serialize)]
pub struct Row {
    pub key: String,
    pub label: String,
    pub hands: usize,
    pub total_chips: i64,
    /// Mean result per hand in big blinds, and its 95% interval.
    pub bb_per_hand: f64,
    pub low_bb: f64,
    pub high_bb: f64,
    /// Contribution to the fleet's overall bb/100.
    pub share_bb100: f64,
}

fn rows(map: HashMap<String, (String, Tally)>, total_hands: f64, min_hands: f64, bb: f64) -> Vec<Row> {
    let mut out: Vec<Row> = map
        .into_iter()
        .filter(|(_, (_, t))| t.n >= min_hands)
        .map(|(key, (label, t))| {
            let hw = t.half_width(1.96);
            Row {
                key,
                label,
                hands: t.n as usize,
                total_chips: t.sum as i64,
                bb_per_hand: t.mean() / bb,
                low_bb: (t.mean() - hw) / bb,
                high_bb: (t.mean() + hw) / bb,
                share_bb100: t.sum / bb / total_hands.max(1.0) * 100.0,
            }
        })
        .collect();
    out.sort_by_key(|r| r.total_chips);
    out
}

fn code(rec: &ActionRecord) -> char {
    match rec.kind {
        ActionKind::Fold => 'F',
        ActionKind::Check => 'X',
        ActionKind::Call => 'C',
        _ if aggressive(rec) => {
            if rec.to_call_before == 0 {
                'B'
            } else {
                'R'
            }
        }
        ActionKind::AllIn => 'C',
        ActionKind::Raise => 'R',
    }
}

/// Our action codes per street, e.g. `R/B/X/F` (preflop raise, flop bet, turn check, river
/// fold); streets we never acted on are omitted from the end.
fn hero_line(h: &HandSummary, hero: usize) -> Vec<String> {
    let mut streets = vec![String::new(); 4];
    for rec in h.history.iter().filter(|r| r.seat == hero) {
        streets[rec.street.index()].push(code(rec));
    }
    while streets.len() > 1 && streets.last().map(|s| s.is_empty()).unwrap_or(false) {
        streets.pop();
    }
    streets.into_iter().map(|s| if s.is_empty() { "-".into() } else { s }).collect()
}

fn street_name(i: usize) -> &'static str {
    ["preflop", "flop", "turn", "river"][i.min(3)]
}

fn describe_line(line: &[String]) -> String {
    let word = |c: char| match c {
        'F' => "fold",
        'X' => "check",
        'C' => "call",
        'B' => "bet",
        'R' => "raise",
        _ => "?",
    };
    line.iter()
        .enumerate()
        .filter(|(_, s)| s.as_str() != "-")
        .map(|(i, s)| format!("{} {}", street_name(i), s.chars().map(word).collect::<Vec<_>>().join("-")))
        .collect::<Vec<_>>()
        .join(", ")
}

/// How the hand ended for us: folded on a street (after investing voluntarily or not), or
/// showdown won/lost, or won without showdown.
fn outcome(h: &HandSummary, hero: usize, net: i64) -> (String, String) {
    let fold = h.history.iter().find(|r| r.seat == hero && r.kind == ActionKind::Fold);
    let invested_voluntarily = |upto: Street| {
        h.history
            .iter()
            .any(|r| r.seat == hero && r.street <= upto && matches!(r.kind, ActionKind::Call | ActionKind::Raise | ActionKind::AllIn))
    };
    let bet_same_street = |s: Street| h.history.iter().any(|r| r.seat == hero && r.street == s && aggressive(r));
    if let Some(f) = fold {
        let s = f.street;
        if bet_same_street(s) {
            return (format!("bet_fold_{}", street_name(s.index())), format!("Bet or raised, then folded ({})", street_name(s.index())));
        }
        if s == Street::Preflop && !invested_voluntarily(s) {
            return ("fold_pre_blind".into(), "Folded preflop without investing (blinds only)".into());
        }
        return (format!("fold_{}", street_name(s.index())), format!("Folded on the {} after investing", street_name(s.index())));
    }
    let showdown = h.shown.iter().any(|(s, _)| *s == hero);
    match (showdown, net >= 0) {
        (true, true) => ("showdown_won".into(), "Won at showdown".into()),
        (true, false) => ("showdown_lost".into(), "Lost at showdown".into()),
        (false, true) => ("won_no_showdown".into(), "Won without showdown".into()),
        (false, false) => ("lost_no_showdown".into(), "Lost without showdown (all-in runout or split)".into()),
    }
}

#[derive(Serialize)]
pub struct Tripwire {
    pub street: String,
    pub bet_into_us: f64,
    pub bet_into_us_n: usize,
    pub bet_elsewhere: f64,
    pub bet_elsewhere_n: usize,
    pub z: f64,
    pub our_fold: f64,
    pub faced: usize,
    pub mdf_fold: f64,
    pub flagged: bool,
}

/// Do opponents bet into us more often than into others while we fold above MDF?
pub fn tripwires(hands: &[(String, HandSummary)]) -> Vec<Tripwire> {
    let mut out = Vec::new();
    for street in [Street::Flop, Street::Turn, Street::River] {
        let (mut in_opp, mut in_bet, mut out_opp, mut out_bet, mut face, mut fold, mut mdf) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        for (bot, h) in hands {
            let Some(hero) = h.players.iter().find(|p| p.1 == *bot).map(|p| p.0) else { continue };
            let mut folded: Vec<usize> = Vec::new();
            for rec in &h.history {
                if rec.street == street {
                    if rec.seat != hero && rec.to_call_before == 0 && !folded.contains(&rec.seat) {
                        let a = aggressive(rec) as i32 as f64;
                        if folded.contains(&hero) {
                            out_opp += 1.0;
                            out_bet += a;
                        } else {
                            in_opp += 1.0;
                            in_bet += a;
                        }
                    }
                    if rec.seat == hero && rec.to_call_before > 0 {
                        face += 1.0;
                        fold += (rec.kind == ActionKind::Fold) as i32 as f64;
                        let pot = (rec.pot_before - rec.to_call_before).max(1) as f64;
                        mdf += pot / (pot + rec.to_call_before as f64);
                    }
                }
                if rec.kind == ActionKind::Fold {
                    folded.push(rec.seat);
                }
            }
        }
        let rate = |a: f64, n: f64| if n > 0.0 { a / n } else { 0.0 };
        let (pin, pout) = (rate(in_bet, in_opp), rate(out_bet, out_opp));
        let pooled = rate(in_bet + out_bet, in_opp + out_opp);
        let z = if in_opp > 0.0 && out_opp > 0.0 {
            (pin - pout) / (pooled * (1.0 - pooled) * (1.0 / in_opp + 1.0 / out_opp)).sqrt().max(1e-9)
        } else {
            0.0
        };
        let our_fold = rate(fold, face);
        let mdf_fold = 1.0 - rate(mdf, face);
        out.push(Tripwire {
            street: street.name().to_string(),
            bet_into_us: pin,
            bet_into_us_n: in_opp as usize,
            bet_elsewhere: pout,
            bet_elsewhere_n: out_opp as usize,
            z,
            our_fold,
            faced: face as usize,
            mdf_fold,
            flagged: z > 2.0 && our_fold > mdf_fold,
        });
    }
    out
}

/// Load every recorded hand of the fleet in time order: (bot, ended_at, net, summary).
fn load(store: &Store, fleet: &[String]) -> anyhow::Result<Vec<(String, String, i64, HandSummary)>> {
    let mut all = Vec::new();
    for bot in fleet {
        for row in store.recent_hands(bot, 10_000_000)? {
            let (Some(net), Ok(h)) = (row.net, serde_json::from_str::<HandSummary>(&row.summary)) else { continue };
            all.push((bot.clone(), row.ended_at, net, h));
        }
    }
    all.sort_by(|a, b| a.1.cmp(&b.1));
    Ok(all)
}

/// The full leak-finder report.
///
/// The leaks themselves are accumulated learning about our own play — the strategy carries across
/// seasons, so the lines, positions, outcomes, trend, tripwires and advice read every stored hand.
/// Only the headline result is a season standing, so `season` reports this season's alone.
pub fn report(
    store: &Store,
    fleet: &[String],
    h2h: &HashMap<String, crate::headtohead::HeadToHead>,
    calibration: Option<&Value>,
    bb: f64,
    season: Option<&crate::season::CurrentSeason>,
) -> anyhow::Result<Value> {
    let hands = load(store, fleet)?;
    let total = hands.len() as f64;
    let mut lines: HashMap<String, (String, Tally)> = HashMap::new();
    let mut outcomes: HashMap<String, (String, Tally)> = HashMap::new();
    let mut positions: HashMap<String, (String, Tally)> = HashMap::new();
    let mut overall = Tally::default();
    let mut this_season = Tally::default();
    let mut aggressive_hands = [0usize; 4];
    for (bot, ended_at, net, h) in &hands {
        let Some(hero) = h.players.iter().find(|p| p.1 == *bot).map(|p| p.0) else { continue };
        let x = *net as f64;
        overall.add(x);
        if season.map(|s| s.contains(crate::season::ended_at_secs(ended_at))).unwrap_or(true) {
            this_season.add(x);
        }
        for (i, n) in aggressive_hands.iter_mut().enumerate() {
            if h.history.iter().any(|r| r.seat == hero && r.street.index() == i && aggressive(r)) {
                *n += 1;
            }
        }
        let line = hero_line(h, hero);
        let key = line.join("/");
        lines.entry(key).or_insert_with(|| (describe_line(&line), Tally::default())).1.add(x);
        let (ok, olabel) = outcome(h, hero, *net);
        outcomes.entry(ok).or_insert_with(|| (olabel, Tally::default())).1.add(x);
        let seats: Vec<usize> = h.players.iter().map(|p| p.0).collect();
        let pos = position_of(&seats, h.button, hero).name().to_string();
        positions.entry(pos.clone()).or_insert_with(|| (pos, Tally::default())).1.add(x);
    }
    let line_rows = rows(lines, total, 20.0, bb);
    let outcome_rows = rows(outcomes, total, 10.0, bb);
    let mut position_rows = rows(positions, total, 1.0, bb);
    let order = ["EP", "MP", "CO", "BTN", "SB", "BB"];
    position_rows.sort_by_key(|r| order.iter().position(|p| *p == r.key).unwrap_or(9));

    // Trend: bb/100 per block of 250 hands plus the cumulative result.
    let block = 250usize;
    let mut trend = Vec::new();
    let mut cumulative = 0i64;
    for (i, chunk) in hands.chunks(block).enumerate() {
        let sum: i64 = chunk.iter().map(|c| c.2).sum();
        cumulative += sum;
        trend.push(json!({"block": i, "hands_end": (i * block + chunk.len()), "bb100": sum as f64 / bb / chunk.len() as f64 * 100.0, "cumulative_bb": cumulative as f64 / bb, "until": chunk.last().map(|c| c.1.clone())}));
    }
    let recent: Vec<(String, HandSummary)> = hands.iter().map(|(b, _, _, h)| (b.clone(), h.clone())).collect();
    let trips = tripwires(&recent);

    // Opponents: worst head-to-head results with enough hands.
    let tested = crate::headtohead::tested(h2h, 300.0);
    let mut opp: Vec<Value> = h2h
        .iter()
        .filter(|(_, v)| v.hands >= 100.0)
        .map(|(n, v)| json!({"name": n, "hands": v.hands as i64, "bb100": v.mean() / bb * 100.0, "upper_bb100": v.upper_95() / bb * 100.0, "beats_us": v.beats_us(300.0, tested)}))
        .collect();
    opp.sort_by(|a, b| a["bb100"].as_f64().partial_cmp(&b["bb100"].as_f64()).unwrap_or(std::cmp::Ordering::Equal));
    opp.truncate(15);

    let suggestions = suggest(&line_rows, &outcome_rows, &position_rows, &trips, &opp, calibration, overall, aggressive_hands, bb);
    let summary = |t: &Tally| {
        json!({"hands": t.n as i64, "bb100": t.mean() / bb * 100.0, "low_bb100": (t.mean() - t.half_width(1.96)) / bb * 100.0,
               "high_bb100": (t.mean() + t.half_width(1.96)) / bb * 100.0, "chips": t.sum as i64})
    };
    let mut season_summary = summary(&this_season);
    season_summary["scoped"] = json!(season.is_some());
    season_summary["number"] = json!(season.and_then(|s| s.number));
    season_summary["started_at"] = json!(season.map(|s| s.started_at));
    Ok(json!({
        "hands": total as i64,
        "season": season_summary,
        "overall": {"bb100": overall.mean() / bb * 100.0, "low_bb100": (overall.mean() - overall.half_width(1.96)) / bb * 100.0, "high_bb100": (overall.mean() + overall.half_width(1.96)) / bb * 100.0, "chips": overall.sum as i64},
        "costly_lines": line_rows.iter().take(15).collect::<Vec<_>>(),
        "best_lines": line_rows.iter().rev().take(8).collect::<Vec<_>>(),
        "outcomes": outcome_rows,
        "positions": position_rows,
        "trend": trend,
        "tripwires": trips,
        "opponents": opp,
        "suggestions": suggestions,
        "updated": chrono::Utc::now().timestamp(),
    }))
}

/// Rule-based study notes from the findings: each names the evidence, the likely cause, and
/// what the bot does (or can be tuned to do) about it.
#[allow(clippy::too_many_arguments)]
fn suggest(
    lines: &[Row],
    outcomes: &[Row],
    positions: &[Row],
    trips: &[Tripwire],
    opp: &[Value],
    calibration: Option<&Value>,
    overall: Tally,
    aggressive_hands: [usize; 4],
    bb: f64,
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
                format!("{} of {} hands, {:+} chips ({:.1} bb/100 of the total)", r.hands, base, r.total_chips, r.share_bb100),
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
            format!("Showdowns: win {:.0}% of {} for {:+.0} bb/100 net", win_rate * 100.0, n, won.share_bb100 + lost.share_bb100),
            format!("Won {} ({:+} chips), lost {} ({:+} chips)", won.hands, won.total_chips, lost.hands, lost.total_chips),
            if win_rate < 0.45 {
                "Reaching showdown behind too often: calling down too light. Tighten river calls against players who rarely bluff.".into()
            } else {
                "Healthy: we reach showdown mostly with the best hand.".into()
            },
        );
    }
    // Losing lines that did not end in a fold are genuine leaks (calling down, betting into better hands).
    for r in lines.iter().filter(|r| r.high_bb < 0.0 && r.hands >= 40 && !r.key.ends_with('F')).take(4) {
        push(
            "high",
            format!("Losing line: {}", r.label),
            format!("{} hands, {:+.2} bb per hand (95% {:+.2}..{:+.2}), {:.0} bb/100 of the total", r.hands, r.bb_per_hand, r.low_bb, r.high_bb, r.share_bb100),
            "Significantly negative without folding, so chips go in behind. Replay the biggest losses of this line; if the chosen action showed a high EV each time, the opponent response model is too optimistic here.".into(),
        );
    }
    if let Some(worst) = positions
        .iter()
        .filter(|r| r.hands >= 100)
        .min_by(|a, b| a.bb_per_hand.partial_cmp(&b.bb_per_hand).unwrap_or(std::cmp::Ordering::Equal))
    {
        let blinds = worst.key == "SB" || worst.key == "BB";
        push(
            if worst.high_bb < 0.0 && !blinds { "medium" } else { "info" },
            format!("Weakest position: {} ({:+.1} bb/100)", worst.key, worst.bb_per_hand * 100.0),
            format!("{} hands (95% {:+.1}..{:+.1} bb/100)", worst.hands, worst.low_bb * 100.0, worst.high_bb * 100.0),
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
    if overall.n >= 500.0 && overall.mean() - overall.half_width(1.96) > 0.0 {
        push(
            "info",
            format!("Winning significantly: {:+.0} bb/100", overall.mean() / bb * 100.0),
            format!("{} hands, 95% lower bound {:+.0} bb/100", overall.n as i64, (overall.mean() - overall.half_width(1.96)) / bb * 100.0),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn tally(sum: f64, n: f64) -> Tally {
        let mut t = Tally::default();
        for _ in 0..n as usize {
            t.add(sum / n);
        }
        t
    }

    #[test]
    fn rows_uses_passed_big_blind() {
        let mut lines: HashMap<String, (String, Tally)> = HashMap::new();
        lines.insert("test".into(), ("Test".into(), tally(1000.0, 100.0)));

        let bb20 = rows(lines.clone(), 100.0, 1.0, 20.0);
        let bb10 = rows(lines.clone(), 100.0, 1.0, 10.0);
        let bb50 = rows(lines.clone(), 100.0, 1.0, 50.0);

        // bb_per_hand scales inversely with bb
        assert_eq!(bb10[0].bb_per_hand, 2.0 * bb20[0].bb_per_hand);
        assert_eq!(bb50[0].bb_per_hand, 0.4 * bb20[0].bb_per_hand);
        // share_bb100 scales inversely with bb
        assert_eq!(bb10[0].share_bb100, 2.0 * bb20[0].share_bb100);
        assert_eq!(bb50[0].share_bb100, 0.4 * bb20[0].share_bb100);
    }
}

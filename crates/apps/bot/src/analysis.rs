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

mod suggest;

/// Running mean and 95% interval of chips per hand (`sv10_stats`, 0231).
type Tally = sv10_stats::moments::Moments;

/// One leak group: raw chips over every settled hand, plus the per-hand-normalized
/// big-blind tally over only the hands with a positive recorded blind.
struct Group {
    label: String,
    chips: Tally,
    priced: Tally,
}

/// This hand's net in big blinds, or `None` when the recorded blind is missing or
/// nonpositive: an unpriced hand still counts in chip totals, never in a rate.
fn normalize(net: i64, bb: i64) -> Option<f64> {
    (bb > 0).then_some(net as f64 / bb as f64)
}

#[derive(Serialize)]
pub struct Row {
    pub key: String,
    pub label: String,
    pub hands: usize,
    /// Hands with a positive recorded blind behind the bb rates; rates are absent without one.
    pub priced_hands: usize,
    pub total_chips: i64,
    /// Mean result per priced hand in big blinds, and its 95% interval.
    pub bb_per_hand: Option<f64>,
    pub low_bb: Option<f64>,
    pub high_bb: Option<f64>,
    /// Contribution to the fleet's overall bb/100 over priced hands.
    pub share_bb100: Option<f64>,
}

fn rows(map: HashMap<String, Group>, priced_total: f64, min_hands: f64) -> Vec<Row> {
    let mut out: Vec<Row> = map
        .into_iter()
        .filter(|(_, g)| g.chips.n >= min_hands)
        .map(|(key, g)| {
            let some = g.priced.n > 0.0;
            let hw = g.priced.half_width(1.96);
            Row {
                key,
                label: g.label,
                hands: g.chips.n as usize,
                priced_hands: g.priced.n as usize,
                total_chips: g.chips.sum as i64,
                bb_per_hand: some.then_some(g.priced.mean()),
                low_bb: some.then_some(g.priced.mean() - hw),
                high_bb: some.then_some(g.priced.mean() + hw),
                share_bb100: (some && priced_total > 0.0).then_some(g.priced.sum / priced_total * 100.0),
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
    let mut lines: HashMap<String, Group> = HashMap::new();
    let mut outcomes: HashMap<String, Group> = HashMap::new();
    let mut positions: HashMap<String, Group> = HashMap::new();
    let mut overall = Tally::default();
    let mut this_season = Tally::default();
    let mut overall_bb = Tally::default();
    let mut season_bb = Tally::default();
    let mut aggressive_hands = [0usize; 4];
    let group = |map: &mut HashMap<String, Group>, key: String, label: String, net: f64, rate: Option<f64>| {
        let g = map.entry(key).or_insert_with(|| Group { label, chips: Tally::default(), priced: Tally::default() });
        g.chips.add(net);
        if let Some(q) = rate {
            g.priced.add(q);
        }
    };
    for (bot, ended_at, net, h) in &hands {
        let Some(hero) = h.players.iter().find(|p| p.1 == *bot).map(|p| p.0) else { continue };
        let x = *net as f64;
        let rate = normalize(*net, h.bb);
        overall.add(x);
        if season.map(|s| s.contains(crate::season::ended_at_secs(ended_at))).unwrap_or(true) {
            this_season.add(x);
        }
        if let Some(q) = rate {
            overall_bb.add(q);
            if season.map(|s| s.contains(crate::season::ended_at_secs(ended_at))).unwrap_or(true) {
                season_bb.add(q);
            }
        }
        for (i, n) in aggressive_hands.iter_mut().enumerate() {
            if h.history.iter().any(|r| r.seat == hero && r.street.index() == i && aggressive(r)) {
                *n += 1;
            }
        }
        let line = hero_line(h, hero);
        let key = line.join("/");
        group(&mut lines, key.clone(), describe_line(&line), x, rate);
        let (ok, olabel) = outcome(h, hero, *net);
        group(&mut outcomes, ok, olabel, x, rate);
        let seats: Vec<usize> = h.players.iter().map(|p| p.0).collect();
        let pos = position_of(&seats, h.button, hero).name().to_string();
        group(&mut positions, pos.clone(), pos, x, rate);
    }
    let priced_total = overall_bb.n;
    let line_rows = rows(lines, priced_total, 20.0);
    let outcome_rows = rows(outcomes, priced_total, 10.0);
    let mut position_rows = rows(positions, priced_total, 1.0);
    let order = ["EP", "MP", "CO", "BTN", "SB", "BB"];
    position_rows.sort_by_key(|r| order.iter().position(|p| *p == r.key).unwrap_or(9));

    // Trend: bb/100 per block of 250 hands plus the cumulative result. Block rates use
    // each hand's own recorded blind; unpriced hands count in chips, never in a rate.
    let block = 250usize;
    let mut trend = Vec::new();
    let mut bb_cumulative = 0.0;
    for (i, chunk) in hands.chunks(block).enumerate() {
        let mut priced = 0usize;
        let mut bb_sum = 0.0;
        for c in chunk {
            if let Some(q) = normalize(c.2, c.3.bb) {
                priced += 1;
                bb_sum += q;
            }
        }
        bb_cumulative += bb_sum;
        trend.push(json!({"block": i, "hands_end": (i * block + chunk.len()), "priced_hands": priced,
            "bb100": if priced > 0 { Some(bb_sum / priced as f64 * 100.0) } else { None::<f64> },
            "cumulative_bb": bb_cumulative, "until": chunk.last().map(|c| c.1.clone())}));
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

    let suggestions = suggest::suggest(&line_rows, &outcome_rows, &position_rows, &trips, &opp, calibration, overall_bb, aggressive_hands);
    let summary = |t: &Tally, priced: &Tally| {
        let some = priced.n > 0.0;
        let hw = priced.half_width(1.96);
        json!({"hands": t.n as i64, "priced_hands": priced.n as i64,
               "bb100": some.then_some(priced.mean() * 100.0),
               "low_bb100": some.then_some((priced.mean() - hw) * 100.0),
               "high_bb100": some.then_some((priced.mean() + hw) * 100.0),
               "chips": t.sum as i64})
    };
    let mut season_summary = summary(&this_season, &season_bb);
    season_summary["scoped"] = json!(season.is_some());
    season_summary["number"] = json!(season.and_then(|s| s.number));
    season_summary["started_at"] = json!(season.map(|s| s.started_at));
    Ok(json!({
        "hands": hands.len() as i64,
        "priced_hands": overall_bb.n as i64,
        "season": season_summary,
        "overall": summary(&overall, &overall_bb),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn mixed_blinds_normalize_per_hand_and_ignore_the_current_blind() {
        assert_eq!(normalize(10, 10), Some(1.0));
        assert_eq!(normalize(20, 20), Some(1.0));
        assert_eq!(normalize(10, 0), None);
        assert_eq!(normalize(10, -20), None);

        // +10 chips at BB 10 and +20 chips at BB 20 are both +1 bb/hand.
        let mut chips = Tally::default();
        chips.add(10.0);
        chips.add(20.0);
        let mut priced = Tally::default();
        priced.add(1.0);
        priced.add(1.0);
        let mut map: HashMap<String, Group> = HashMap::new();
        map.insert("mix".into(), Group { label: "Mix".into(), chips, priced });
        // A hand with no positive recorded blind still counts in chips, never in rates.
        let mut raw_chips = Tally::default();
        raw_chips.add(50.0);
        map.insert("raw".into(), Group { label: "Raw".into(), chips: raw_chips, priced: Tally::default() });

        let out = rows(map, 2.0, 1.0);
        let mix = out.iter().find(|r| r.key == "mix").unwrap();
        assert_eq!((mix.hands, mix.priced_hands), (2, 2));
        assert_eq!(mix.total_chips, 30);
        assert_eq!(mix.bb_per_hand, Some(1.0));
        assert_eq!((mix.low_bb, mix.high_bb), (Some(1.0), Some(1.0)));
        assert_eq!(mix.share_bb100, Some(100.0));
        let raw = out.iter().find(|r| r.key == "raw").unwrap();
        assert_eq!((raw.hands, raw.priced_hands), (1, 0));
        assert_eq!(raw.total_chips, 50);
        assert_eq!(raw.bb_per_hand, None);
        assert_eq!(raw.share_bb100, None);
    }
}

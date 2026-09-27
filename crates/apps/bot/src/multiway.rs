//! Multiway all-in calls (0219): our equity estimate against the exact share when every other
//! player live at the decision showed their cards.
//!
//! The heads-up all-in fits ([`crate::raisewar`]) only use spots with one opponent. Here a spot
//! counts when our postflop call, shove or raise put the chips in with two or more opponents live,
//! nobody folded or raised after it on that street, and every player live at the decision showed
//! down. The exact share is our pot share against the shown hands over every runout (ties split),
//! the same notion as the decision's equity against ranges, so their gap is the model's bias.

use crate::raisewar::{Commit, STREETS, is_raise};
use sv10_core::cards::{Card, CardMask, mask_of, parse_cards};
use sv10_core::eval::eval;

/// Our expected share of a pot contested by `hero` and `villains` on `board` (3–5 cards), over
/// every runout of the missing cards; ties split.
pub fn exact_share(hero: [Card; 2], villains: &[[Card; 2]], board: &[Card]) -> f64 {
    let mut dead = mask_of(&hero) | mask_of(board);
    for v in villains {
        dead |= mask_of(v);
    }
    let rest: Vec<CardMask> = (0..52u8).map(|c| Card(c).bit()).filter(|b| dead & b == 0).collect();
    let (h, b) = (mask_of(&hero), mask_of(board));
    let vs: Vec<CardMask> = villains.iter().map(|v| mask_of(v)).collect();
    let (mut score, mut n) = (0.0, 0.0);
    let mut settle = |extra: CardMask| {
        let ours = eval(h | b | extra);
        let best = vs.iter().map(|&v| eval(v | b | extra)).max().unwrap_or(0);
        score += if ours > best {
            1.0
        } else if ours == best {
            1.0 / (1 + vs.iter().filter(|&&v| eval(v | b | extra) == ours).count()) as f64
        } else {
            0.0
        };
        n += 1.0;
    };
    match 5 - board.len() {
        0 => settle(0),
        1 => rest.iter().for_each(|&x| settle(x)),
        _ => {
            for i in 0..rest.len() {
                for j in i + 1..rest.len() {
                    settle(rest[i] | rest[j]);
                }
            }
        }
    }
    score / n
}

/// The multiway spot of our `ordinal`-th action on `street` in a stored hand, with the number of
/// opponents live at it, when the chips went in and every live opponent showed.
pub fn multiway_commit_from_summary(
    summary: &serde_json::Value,
    hero: &str,
    street: usize,
    ordinal: usize,
    action: &str,
    estimate: f64,
    (pot, to_call): (i64, i64),
) -> Option<(usize, Commit)> {
    let players = summary["players"].as_array()?;
    let seat = players.iter().find(|p| p[1].as_str() == Some(hero))?[0].as_i64()?;
    let history = summary["history"].as_array()?;
    let name = STREETS[street];
    let at =
        history.iter().enumerate().filter(|(_, a)| a["seat"].as_i64() == Some(seat) && a["street"].as_str() == Some(name)).nth(ordinal)?.0;
    let ours = &history[at];
    let call = match (action, ours["kind"].as_str()?) {
        ("call", "Call") => true,
        ("call", "AllIn") if !is_raise(ours) => true,
        ("all_in", "AllIn") => !is_raise(ours),
        ("raise", "Raise") => false,
        _ => return None,
    };
    if !history[at + 1..]
        .iter()
        .all(|a| a["street"].as_str() == Some(name) && matches!(a["kind"].as_str(), Some("Call" | "AllIn")) && !is_raise(a))
    {
        return None;
    }
    let folded_before = |s: i64| history[..at].iter().any(|a| a["seat"].as_i64() == Some(s) && a["kind"].as_str() == Some("Fold"));
    let live: Vec<i64> = players.iter().filter_map(|p| p[0].as_i64()).filter(|&s| s != seat && !folded_before(s)).collect();
    if live.len() < 2 {
        return None;
    }
    let shown = summary["shown"].as_array()?;
    let cards = |s: i64| -> Option<[Card; 2]> {
        let e = shown.iter().find(|e| e[0].as_i64() == Some(s))?;
        let v: Vec<&str> = e[1].as_array()?.iter().filter_map(|c| c.as_str()).collect();
        let c = parse_cards(&v)?;
        (c.len() == 2).then(|| [c[0], c[1]])
    };
    let hero_cards = cards(seat)?;
    let villains: Vec<[Card; 2]> = live.iter().map(|&s| cards(s)).collect::<Option<_>>()?;
    let board_all: Vec<&str> = summary["board"].as_array()?.iter().filter_map(|c| c.as_str()).collect();
    let board = parse_cards(&board_all[..(3 + street).min(board_all.len())])?;
    if board.len() != 3 + street {
        return None;
    }
    let raises_before = history[..at].iter().filter(|a| a["street"].as_str() == Some(name) && is_raise(a)).count();
    let bet_to_pot = history[..at]
        .iter()
        .rfind(|a| a["street"].as_str() == Some(name) && a["seat"].as_i64() != Some(seat) && is_raise(a))
        .and_then(|a| {
            let added = (a["to"].as_i64()? - a["bet_before"].as_i64().unwrap_or(0)) as f64;
            let before = a["pot_before"].as_i64()? as f64;
            (before > 0.0).then(|| added / before)
        })
        .unwrap_or(0.0);
    let exact = exact_share(hero_cards, &villains, &board);
    Some((live.len(), Commit { street, raises_before, call, estimate, exact, pot, to_call, bet_to_pot }))
}

/// Every multiway spot in the store (oldest first) with its opponent count.
pub fn multiway_commits_from_store(store: &sv10_store::store::Store) -> anyhow::Result<Vec<(usize, Commit)>> {
    use std::collections::HashMap;
    let mut ordinal: HashMap<(String, String, String), usize> = HashMap::new();
    let mut summaries: HashMap<(String, String), Option<serde_json::Value>> = HashMap::new();
    let mut out = Vec::new();
    for d in &store.postflop_decisions()? {
        let key = (d.bot.clone(), d.hand_id.clone(), d.street.clone());
        let k = *ordinal.entry(key).and_modify(|n| *n += 1).or_insert(0);
        if d.opponents.unwrap_or(0) < 2 || d.equity < 0.0 || !matches!(d.action.as_str(), "call" | "all_in" | "raise") {
            continue;
        }
        let Some(street) = ["flop", "turn", "river"].iter().position(|s| *s == d.street) else { continue };
        let summary = summaries
            .entry((d.bot.clone(), d.hand_id.clone()))
            .or_insert_with(|| store.hand(&d.bot, &d.hand_id).ok().flatten().and_then(|h| serde_json::from_str(&h.summary).ok()));
        let spot = (d.pot.unwrap_or(0), d.to_call.unwrap_or(0));
        if let Some(c) = summary.as_ref().and_then(|s| multiway_commit_from_summary(s, &d.bot, street, k, &d.action, d.equity, spot)) {
            out.push(c);
        }
    }
    Ok(out)
}

/// Estimate against exact share for a group of spots.
#[derive(Clone, Debug, PartialEq)]
pub struct BiasBin {
    /// Spots.
    pub n: usize,
    /// Mean estimate and mean exact share.
    pub estimate: f64,
    /// Mean exact share.
    pub exact: f64,
    /// Mean over-estimate and its 95% half-width.
    pub gap: f64,
    /// 95% half-width of `gap`.
    pub half_width: f64,
}

/// The bias of a set of spots.
pub fn bias(commits: &[&Commit]) -> BiasBin {
    let n = commits.len();
    let nf = n.max(1) as f64;
    let estimate = commits.iter().map(|c| c.estimate).sum::<f64>() / nf;
    let exact = commits.iter().map(|c| c.exact).sum::<f64>() / nf;
    let gap = estimate - exact;
    let var =
        if n > 1 { commits.iter().map(|c| (c.estimate - c.exact - gap).powi(2)).sum::<f64>() / (n - 1) as f64 } else { f64::INFINITY };
    BiasBin { n, estimate, exact, gap, half_width: 1.96 * (var / nf).sqrt() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn c(s: &[&str]) -> Vec<Card> {
        parse_cards(s).unwrap()
    }

    #[test]
    fn exact_share_splits_ties_and_matches_heads_up() {
        let hero = [Card::parse("As").unwrap(), Card::parse("Ah").unwrap()];
        let villain = [Card::parse("Kd").unwrap(), Card::parse("Kc").unwrap()];
        let board = c(&["2c", "7d", "9h"]);
        let hu = exact_share(hero, &[villain], &board);
        assert!((hu - crate::raisewar::exact_equity(hero, villain, &board)).abs() < 1e-12);
        // Three identical broadway straights on the river board: everyone splits three ways.
        let b = c(&["Ts", "Jd", "Qh", "Kc", "As"]);
        let (x, y, z) = (
            [Card::parse("2c").unwrap(), Card::parse("3d").unwrap()],
            [Card::parse("4c").unwrap(), Card::parse("5d").unwrap()],
            [Card::parse("6c").unwrap(), Card::parse("7d").unwrap()],
        );
        assert!((exact_share(x, &[y, z], &b) - 1.0 / 3.0).abs() < 1e-12);
        // A third player can only lower our share.
        let third = [Card::parse("9s").unwrap(), Card::parse("9d").unwrap()];
        assert!(exact_share(hero, &[villain, third], &board) < hu);
    }

    fn hand(shown: serde_json::Value, after: serde_json::Value) -> serde_json::Value {
        let mut history = vec![
            json!({"seat": 3, "street": "Preflop", "kind": "Fold", "to": 0}),
            json!({"seat": 1, "street": "Flop", "kind": "AllIn", "to": 900, "bet_before": 0, "pot_before": 300, "full_raise": true}),
            json!({"seat": 2, "street": "Flop", "kind": "Call", "to": 900, "bet_before": 0, "pot_before": 1200, "full_raise": false}),
            json!({"seat": 0, "street": "Flop", "kind": "Call", "to": 900, "bet_before": 0, "pot_before": 2100, "full_raise": false}),
        ];
        history.extend(after.as_array().unwrap().iter().cloned());
        json!({"players": [[0, "Hero"], [1, "A"], [2, "B"], [3, "C"]], "history": history,
            "board": ["2c", "7d", "9h", "Js", "3d"], "shown": shown})
    }

    #[test]
    fn a_multiway_call_counts_only_when_every_live_opponent_showed() {
        let all = json!([[0, ["As", "Ah"]], [1, ["Kd", "Kc"]], [2, ["9s", "9d"]]]);
        let (opp, commit) = multiway_commit_from_summary(&hand(all.clone(), json!([])), "Hero", 0, 0, "call", 0.6, (2100, 900)).unwrap();
        assert_eq!(opp, 2, "the preflop folder is not live");
        assert!(commit.call && commit.bet_to_pot > 2.9);
        let hero = [Card::parse("As").unwrap(), Card::parse("Ah").unwrap()];
        let vs = [[Card::parse("Kd").unwrap(), Card::parse("Kc").unwrap()], [Card::parse("9s").unwrap(), Card::parse("9d").unwrap()]];
        assert!((commit.exact - exact_share(hero, &vs, &c(&["2c", "7d", "9h"]))).abs() < 1e-12);
        // One live opponent's cards missing, or a heads-up spot: not counted here.
        let partial = json!([[0, ["As", "Ah"]], [1, ["Kd", "Kc"]]]);
        assert!(multiway_commit_from_summary(&hand(partial, json!([])), "Hero", 0, 0, "call", 0.6, (2100, 900)).is_none());
        // A raise after ours means the chips were not in at our decision.
        let raise =
            json!([{"seat": 2, "street": "Flop", "kind": "Raise", "to": 3000, "bet_before": 900, "pot_before": 3000, "full_raise": true}]);
        assert!(multiway_commit_from_summary(&hand(all, raise), "Hero", 0, 0, "call", 0.6, (2100, 900)).is_none());
    }

    #[test]
    fn bias_reports_the_mean_over_estimate() {
        let spot = |e: f64, x: f64| Commit {
            street: 0,
            raises_before: 1,
            call: true,
            estimate: e,
            exact: x,
            pot: 100,
            to_call: 50,
            bet_to_pot: 1.0,
        };
        let spots = [spot(0.5, 0.4), spot(0.6, 0.5), spot(0.4, 0.3)];
        let b = bias(&spots.iter().collect::<Vec<_>>());
        assert_eq!(b.n, 3);
        assert!((b.gap - 0.1).abs() < 1e-12 && b.half_width < 1e-9);
    }
}

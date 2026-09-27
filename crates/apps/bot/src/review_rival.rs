//! `review rival NAME...` and `review allin-luck` (0213, 0317): what we win and lose against named
//! opponents, and where.
//!
//! The head-to-head ledger (0221) says *how much* moved between us and an opponent; with 10,000 shared
//! hands and a ±190 bb/100 interval it cannot say *why*. [`rival`] splits the same chip flow by how
//! each confrontation ended, the street, the pot type, position and who raised first preflop, removes
//! all-in luck where the split is exact (a heads-up all-in between the two), and prints our whole net at
//! their tables beside it. The two are different questions (0280): the flow is the rivalry, the table
//! net is what the leaderboard counts.
//!
//! The flow half is the ledger's population, so it is champion play — experiment-mode treatment hands
//! are left out of it, as they are out of the ledger (0361) — while the whole net is every chip we won
//! or lost at their tables, experiment hands included, because there the chips are the answer.

use anyhow::Result;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use sv10_core::allin::AllInLuck;
use sv10_core::engine::{ActionKind, Street};
use sv10_core::model::HandSummary;
use sv10_store::store::{HandRow, Store};

/// One hand in which chips moved between us and the rival, classified.
#[derive(Clone, Debug, PartialEq)]
pub struct Confrontation {
    /// `they fold`, `we fold` or `showdown`.
    pub ending: &'static str,
    /// Street the confrontation ended on: the fold's street, or where betting closed at a showdown.
    pub street: Street,
    /// `limped`, `single-raised`, `3-bet` or `4-bet+`, by the preflop raises.
    pub pot_type: &'static str,
    /// Whether we act after the rival postflop.
    pub in_position: bool,
    /// Who made the last preflop raise: `we`, `they`, `another` or `nobody`.
    pub opener: &'static str,
    /// Chips the rival lost to us (negative: we lost to them), in big blinds.
    pub flow_bb: f64,
    /// All-in luck in the flow, to add for its all-in EV (0 unless the split is exact).
    pub luck_bb: f64,
}

/// Whether record `i` of `h` raised (an `AllIn` record carries no amount: its chips come from the pot).
fn aggressive(h: &HandSummary, i: usize) -> bool {
    let r = &h.history[i];
    match r.kind {
        ActionKind::Raise => true,
        ActionKind::AllIn => h.history.get(i + 1).map_or(r.to - r.bet_before, |n| n.pot_before - r.pot_before) > r.to_call_before,
        _ => false,
    }
}

/// Classify one hand between `hero` and `rival` (seats), given the flow between them and our all-in
/// luck adjustment, both in chips. `None` when no chips moved between the two.
///
/// Luck is split exactly only for a heads-up all-in between the two: the flow's share of our
/// adjustment is `2 · min(contribution) / pot` (dead money from folders is the rest). A multiway
/// all-in stays unadjusted.
pub fn classify(h: &HandSummary, hero: usize, rival: usize, flow: f64, hero_luck: f64, pot: i64) -> Option<Confrontation> {
    if flow == 0.0 {
        return None;
    }
    let folded_at = |seat: usize| h.history.iter().find(|r| r.seat == seat && r.kind == ActionKind::Fold).map(|r| r.street);
    let closed = h.history.last().map_or(Street::Preflop, |r| r.street);
    let (ending, street) = match (folded_at(rival), folded_at(hero)) {
        (Some(s), _) => ("they fold", s),
        (None, Some(s)) => ("we fold", s),
        (None, None) => ("showdown", closed),
    };
    let raises: Vec<usize> =
        (0..h.history.len()).filter(|&i| h.history[i].street == Street::Preflop && aggressive(h, i)).map(|i| h.history[i].seat).collect();
    let pot_type = match raises.len() {
        0 => "limped",
        1 => "single-raised",
        2 => "3-bet",
        _ => "4-bet+",
    };
    let opener = match raises.last() {
        None => "nobody",
        Some(&s) if s == hero => "we",
        Some(&s) if s == rival => "they",
        Some(_) => "another",
    };
    // Postflop order: the seats after the button, the button last.
    let mut seats: Vec<usize> = h.players.iter().map(|p| p.0).collect();
    seats.sort_unstable();
    let order: Vec<usize> =
        seats.iter().copied().filter(|&s| s > h.button).chain(seats.iter().copied().filter(|&s| s <= h.button)).collect();
    let place = |s: usize| order.iter().position(|&x| x == s);
    let in_position = place(hero) > place(rival);
    let live: Vec<usize> = seats.iter().copied().filter(|&s| folded_at(s).is_none()).collect();
    let luck = match (hero_luck != 0.0 && live.len() == 2 && live.contains(&hero) && live.contains(&rival), pot > 0) {
        (true, true) => sv10_core::flow::contributions(h, pot)
            .and_then(|c| Some(hero_luck * 2.0 * (*c.get(&hero)?).min(*c.get(&rival)?) as f64 / pot as f64))
            .unwrap_or(0.0),
        _ => 0.0,
    };
    let bb = h.bb.max(1) as f64;
    Some(Confrontation { ending, street, pot_type, in_position, opener, flow_bb: flow / bb, luck_bb: luck / bb })
}

/// How `hero` first answered a flop continuation bet by the preflop raiser: `Some((raiser, answer))`
/// with `fold`, `call` or `raise`, when the last preflop raise was not ours, that raiser made the
/// first flop bet, and we acted on it. The adaptation check of 0317: facing a player who c-bets 81%
/// of flops, our folds should be rarer than facing the pool.
pub fn cbet_answer(h: &HandSummary, hero: usize) -> Option<(usize, &'static str)> {
    let raiser = (0..h.history.len())
        .filter(|&i| h.history[i].street == Street::Preflop && aggressive(h, i))
        .map(|i| h.history[i].seat)
        .next_back()?;
    let flop: Vec<usize> = (0..h.history.len()).filter(|&i| h.history[i].street == Street::Flop).collect();
    let bet = *flop.iter().find(|&&i| aggressive(h, i))?;
    if raiser == hero || h.history[bet].seat != raiser || h.history[bet].to_call_before != 0 {
        return None;
    }
    let answer = flop.iter().copied().find(|&i| i > bet && h.history[i].seat == hero)?;
    Some((
        raiser,
        match h.history[answer].kind {
            ActionKind::Fold => "fold",
            _ if aggressive(h, answer) => "raise",
            _ => "call",
        },
    ))
}

/// What our hole cards made on the flop, over what the board alone shows: `no pair of ours`, `one
/// pair` or `two pair+`. The c-bet comparison is read within each, so a wider preflop defence against
/// one raiser (weaker flops on average) is not mistaken for folding too much to them.
pub fn flop_strength(hole: [sv10_core::cards::Card; 2], flop: &[sv10_core::cards::Card]) -> &'static str {
    let mask = flop.iter().chain(&hole).fold(0u64, |m, c| m | c.bit());
    let ours = sv10_core::eval::category(sv10_core::eval::eval(mask));
    let most = flop.iter().map(|c| flop.iter().filter(|d| d.rank() == c.rank()).count()).max().unwrap_or(1);
    let board = match most {
        3 => 3,
        2 => 1,
        _ => 0,
    };
    match ours.saturating_sub(board) {
        0 => "no pair of ours",
        1 => "one pair",
        _ => "two pair+",
    }
}

/// `fold / call / raise` shares of a tally, with its size.
fn shares(t: &BTreeMap<&'static str, usize>) -> String {
    let n = t.values().sum::<usize>().max(1) as f64;
    let pct = |k: &str| t.get(k).copied().unwrap_or(0) as f64 * 100.0 / n;
    format!("n {:>5}  fold {:>4.1}%  call {:>4.1}%  raise {:>4.1}%", t.values().sum::<usize>(), pct("fold"), pct("call"), pct("raise"))
}

/// Mean and 95% half-width of per-hand values, in bb/100.
fn rate(values: &[f64]) -> (f64, f64) {
    let n = values.len().max(1) as f64;
    let mean = values.iter().sum::<f64>() / n;
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    (mean * 100.0, 1.96 * (var / n).sqrt() * 100.0)
}

/// How many hands a 95% lower bound above zero needs at the measured rate and spread.
fn hands_to_prove(values: &[f64]) -> String {
    let (mean, half) = rate(values);
    if mean <= 0.0 {
        return "not ahead at this rate: no sample proves a win".into();
    }
    let needed = (values.len() as f64 * (half / mean).powi(2)).ceil();
    if needed <= values.len() as f64 {
        "proven: the 95% lower bound is above zero".into()
    } else {
        format!("about {needed:.0} hands at this rate and spread ({:.0} more)", needed - values.len() as f64)
    }
}

/// A way of grouping confrontations for one table of the report.
type Key = fn(&Confrontation) -> String;

/// Every hand of every fleet bot, loaded once for several reports.
fn fleet_rows(store: &Store) -> Result<Vec<HandRow>> {
    let mut rows = Vec::new();
    for b in store.bot_names()? {
        rows.extend(store.recent_hands(&b, 1_000_000)?);
    }
    Ok(rows)
}

/// The rival report for each of `args` naming an opponent (0317); a `since=DATE` argument (RFC 3339
/// or `YYYY-MM-DD`) keeps only hands that ended then or later. A window matters: the five largest
/// pots against the heaviest rival were all played before banking (0204) capped table stacks.
pub fn rival(store: &Store, args: &[String]) -> Result<String> {
    let since = args.iter().find_map(|a| a.strip_prefix("since=")).unwrap_or("");
    let names: Vec<&String> = args.iter().filter(|a| !a.starts_with("since=")).collect();
    let rows: Vec<HandRow> = fleet_rows(store)?.into_iter().filter(|r| r.ended_at.as_str() >= since).collect();
    // The flow half of this report splits the ledger's number, so it shares the ledger's population:
    // champion play, the experiment arms' treatment hands left out (0361). The whole net keeps them.
    let treated = store.treatment_hands()?;
    let ordinary = |r: &HandRow| !treated.contains(&(r.bot.clone(), r.hand_id.clone()));
    let mut out = if since.is_empty() { String::new() } else { format!("hands ended since {since}\n") };
    for name in names {
        let needle = format!("{}]", serde_json::Value::String(name.clone()));
        let (mut dealt, mut table_net, mut table_ev, mut flow, mut flow_ev) = (0usize, vec![], vec![], vec![], vec![]);
        let mut found: Vec<(Confrontation, String, String)> = Vec::new();
        let mut unreconciled = 0usize;
        // Our answers to flop c-bets: this rival's, and every other raiser's in the same window.
        type Tally = BTreeMap<&'static str, usize>;
        let (mut theirs, mut others): (BTreeMap<&'static str, Tally>, BTreeMap<&'static str, Tally>) = Default::default();
        for r in rows.iter().filter(|r| ordinary(r)) {
            let (Some(hero), Ok(h)) = (r.hero_seat, serde_json::from_str::<HandSummary>(&r.summary)) else { continue };
            let (Some((raiser, answer)), Some(hole)) = (cbet_answer(&h, hero as usize), crate::luck::hole(&r.hole)) else { continue };
            if h.board.len() < 3 {
                continue;
            }
            let own = h.players.iter().any(|p| p.0 == raiser && p.1 == **name);
            let side = if own { &mut theirs } else { &mut others };
            for key in ["all", flop_strength(hole, &h.board[..3])] {
                *side.entry(key).or_default().entry(answer).or_default() += 1;
            }
        }
        for r in rows.iter().filter(|r| r.summary.contains(&needle)) {
            let (Some(net), Some(hero), Ok(h)) = (r.net, r.hero_seat, serde_json::from_str::<HandSummary>(&r.summary)) else { continue };
            let Some(seat) = h.players.iter().find(|p| p.1 == **name).map(|p| p.0) else { continue };
            let bb = h.bb.max(1) as f64;
            let luck = crate::luck::hand_luck(r).adjustment();
            dealt += 1; // the table result counts every hand
            table_net.push(net as f64 / bb);
            table_ev.push((net as f64 + luck) / bb);
            if !ordinary(r) {
                continue; // the flow half is champion play (0361)
            }
            let winners: Vec<&str> = r.winners.split(',').filter(|w| !w.is_empty()).collect();
            let Some(f) =
                sv10_core::flow::flow_to_hero(&h, r.pot, &winners, hero as usize).and_then(|f| f.into_iter().find(|x| x.0 == seat))
            else {
                unreconciled += 1;
                continue;
            };
            let c = classify(&h, hero as usize, seat, f.1, luck, r.pot);
            flow.push(f.1 / bb);
            flow_ev.push(c.as_ref().map_or(0.0, |c| c.flow_bb + c.luck_bb));
            if let Some(c) = c {
                found.push((c, r.bot.clone(), r.hand_id.clone()));
            }
        }
        if dealt == 0 {
            let _ = writeln!(out, "{name}: never dealt in with our bots\n");
            continue;
        }
        let line = |v: &[f64]| {
            let (m, h) = rate(v);
            format!("{m:+8.1} ± {h:5.1} bb/100 ({:+.1}..{:+.1})", m - h, m + h)
        };
        let _ = writeln!(
            out,
            "{name}: {dealt} hands dealt in with our bots (every hand), {} champion confrontations (chips moved between us), {unreconciled} not reconciled",
            found.len()
        );
        let _ = writeln!(out, "  flow between us                {}", line(&flow));
        let _ = writeln!(out, "  flow, all-in luck removed      {}", line(&flow_ev));
        let _ = writeln!(out, "  our whole net at their tables  {}", line(&table_net));
        let _ = writeln!(out, "  whole net, all-in luck removed {}", line(&table_ev));
        let _ = writeln!(out, "  to prove we beat them (flow, luck removed): {}", hands_to_prove(&flow_ev));
        let _ = writeln!(out, "  our answer to a flop c-bet, by what we hold (theirs | every other raiser's):");
        for key in ["all", "no pair of ours", "one pair", "two pair+"] {
            let (t, o) = (theirs.get(key).cloned().unwrap_or_default(), others.get(key).cloned().unwrap_or_default());
            let _ = writeln!(out, "    {key:<16} {} | {}", shares(&t), shares(&o));
        }
        let groups: [(&str, Key); 6] = [
            ("how it ended", |c| c.ending.to_string()),
            ("ended on", |c| format!("{:?} {}", c.street, c.ending)),
            ("pot type", |c| c.pot_type.to_string()),
            ("position", |c| if c.in_position { "we act last".into() } else { "they act last".into() }),
            ("last preflop raise", |c| c.opener.to_string()),
            ("pot type × opener", |c| format!("{} by {}", c.pot_type, c.opener)),
        ];
        for (title, key) in groups {
            let mut by: BTreeMap<String, (usize, f64, f64)> = BTreeMap::new();
            for (c, _, _) in &found {
                let e = by.entry(key(c)).or_default();
                e.0 += 1;
                e.1 += c.flow_bb;
                e.2 += c.flow_bb + c.luck_bb;
            }
            let mut rows: Vec<_> = by.into_iter().collect();
            rows.sort_by(|a, b| a.1.2.total_cmp(&b.1.2));
            let _ = writeln!(out, "  {title:<24} {:>7} {:>11} {:>11} {:>9}", "n", "bb/100", "ev bb/100", "bb/conf");
            for (k, (n, sum, ev)) in rows {
                let per100 = |x: f64| x / dealt as f64 * 100.0;
                let _ = writeln!(out, "    {k:<22} {n:>7} {:>+11.1} {:>+11.1} {:>+9.2}", per100(sum), per100(ev), ev / n.max(1) as f64);
            }
        }
        found.sort_by(|a, b| a.0.flow_bb.abs().total_cmp(&b.0.flow_bb.abs()).reverse());
        let _ = writeln!(out, "  biggest confrontations (bb, luck removed):");
        for (c, bot, id) in found.iter().take(8) {
            let _ = writeln!(
                out,
                "    {:+8.1} ({:+8.1}) {bot:<11} {id}  {} {:?}, {}, {} raised last",
                c.flow_bb,
                c.flow_bb + c.luck_bb,
                c.ending,
                c.street,
                c.pot_type,
                c.opener
            );
        }
        out.push('\n');
    }
    out.push_str(
        "'bb/100': each class's share of the flow per 100 hands dealt in together (the rows add up to the flow).\n\
         'flow' counts only chips that moved between us and them (0221) in champion hands — experiment-arm hands are excluded, as in the ledger (0361); the whole net is every chip we won at their tables.\n",
    );
    Ok(out)
}

/// `review allin-luck [NAME...]` (0213): per bot, actual against all-in EV results; against named
/// opponents, our whole net in the hands they were dealt into.
pub fn allin_luck(store: &Store, names: &[String]) -> Result<String> {
    let bb = store.latest_big_blind()?.unwrap_or(crate::live::DEFAULT_BIG_BLIND) as f64;
    let rate = |n: f64, sum: f64, sq: f64| {
        let m = sum / n.max(1.0);
        let sd = (sq / n.max(1.0) - m * m).max(0.0).sqrt();
        format!("{:+.1} ± {:.1}", m / bb * 100.0, 1.96 * sd / n.max(1.0).sqrt() / bb * 100.0)
    };
    let mut out = format!(
        "{:>12} {:>7} {:>8} {:>6} {:>12} {:>12} {:>10} {:>18} {:>18}\n",
        "bot", "hands", "all-ins", "unver", "net", "ev net", "luck", "bb/100 ±95%", "ev bb/100 ±95%"
    );
    for b in &store.bot_names()? {
        let (mut n, mut adj, mut unver) = (0usize, 0usize, 0usize);
        let (mut net, mut ev, mut sq, mut evsq) = (0f64, 0f64, 0f64, 0f64);
        for r in &store.recent_hands(b, 1_000_000)? {
            let Some(x) = r.net else { continue };
            let luck = crate::luck::hand_luck(r);
            match luck {
                AllInLuck::Adjusted { .. } => adj += 1,
                AllInLuck::Unverifiable => unver += 1,
                AllInLuck::NotAllIn => {}
            }
            let e = x as f64 + luck.adjustment();
            (n, net, ev, sq, evsq) = (n + 1, net + x as f64, ev + e, sq + (x * x) as f64, evsq + e * e);
        }
        let _ = writeln!(
            out,
            "{b:>12} {n:>7} {adj:>8} {unver:>6} {net:>12.0} {ev:>12.0} {:>+10.0} {:>18} {:>18}",
            net - ev,
            rate(n as f64, net, sq),
            rate(n as f64, ev, evsq)
        );
    }
    if names.is_empty() {
        return Ok(out);
    }
    let _ = writeln!(
        out,
        "\n{:>18} {:>7} {:>8} {:>12} {:>12} {:>18} {:>18}",
        "opponent", "hands", "all-ins", "net", "ev net", "bb/100 ±95%", "ev bb/100 ±95%"
    );
    let rows = fleet_rows(store)?;
    for name in names {
        let (mut n, mut adj, mut net, mut ev, mut sq, mut evsq) = (0f64, 0usize, 0f64, 0f64, 0f64, 0f64);
        for r in &rows {
            let Some(x) = r.net else { continue };
            let Ok(h) = serde_json::from_str::<HandSummary>(&r.summary) else { continue };
            if !h.players.iter().any(|(_, p)| p == name) {
                continue;
            }
            let luck = crate::luck::hand_luck(r);
            adj += usize::from(matches!(luck, AllInLuck::Adjusted { .. }));
            let e = x as f64 + luck.adjustment();
            (n, net, ev, sq, evsq) = (n + 1.0, net + x as f64, ev + e, sq + (x * x) as f64, evsq + e * e);
        }
        let _ = writeln!(out, "{name:>18} {n:>7} {adj:>8} {net:>12.0} {ev:>12.0} {:>18} {:>18}", rate(n, net, sq), rate(n, ev, evsq));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The all-in hand of `sv10_core::flow`'s tests: SurSvan (seat 4, big blind) shoves the flop,
    /// villain5 (seat 5, the preflop raiser) calls 2,000, villain2 folds.
    const ALL_IN: &str = r#"{"players":[[0,"villain0"],[1,"villain1"],[2,"villain2"],[3,"villain3"],[4,"SurSvan"],[5,"villain5"]],"button":2,"bb":20,"history":[{"seat":5,"street":"Preflop","kind":"Raise","to":50,"pot_before":30,"to_call_before":20,"bet_before":0,"full_raise":true},{"seat":0,"street":"Preflop","kind":"Fold","to":0,"pot_before":80,"to_call_before":50,"bet_before":0,"full_raise":false},{"seat":1,"street":"Preflop","kind":"Fold","to":0,"pot_before":80,"to_call_before":50,"bet_before":0,"full_raise":false},{"seat":2,"street":"Preflop","kind":"Call","to":50,"pot_before":80,"to_call_before":50,"bet_before":0,"full_raise":false},{"seat":3,"street":"Preflop","kind":"Fold","to":0,"pot_before":130,"to_call_before":40,"bet_before":10,"full_raise":false},{"seat":4,"street":"Preflop","kind":"Call","to":50,"pot_before":130,"to_call_before":30,"bet_before":20,"full_raise":false},{"seat":4,"street":"Flop","kind":"AllIn","to":0,"pot_before":160,"to_call_before":0,"bet_before":0,"full_raise":false},{"seat":5,"street":"Flop","kind":"Raise","to":4629,"pot_before":2110,"to_call_before":1950,"bet_before":0,"full_raise":true},{"seat":2,"street":"Flop","kind":"Fold","to":0,"pot_before":6739,"to_call_before":4629,"bet_before":0,"full_raise":false}],"board":["5d","As","3s","6s","Th"],"shown":[[4,["Ts","Ac"]],[5,["Ah","5h"]]]}"#;

    /// 0317: a confrontation is classified by how it ended, the pot type, position and the preflop
    /// raiser, and the rival's share of an all-in's luck is exactly 2·min(contribution)/pot of ours.
    #[test]
    fn a_heads_up_all_in_is_classified_and_its_luck_split_exactly() {
        let h: HandSummary = serde_json::from_str(ALL_IN).unwrap();
        // We lost 2,000 to seat 5; our all-in EV was 600 chips better than the result.
        let c = classify(&h, 4, 5, -2_000.0, 600.0, 4_060).unwrap();
        assert_eq!((c.ending, c.street, c.pot_type, c.opener), ("showdown", Street::Flop, "single-raised", "they"));
        assert!(!c.in_position, "seat 5 acts after the big blind postflop");
        assert_eq!(c.flow_bb, -100.0);
        // 2 · min(2,000, 2,000) / 4,060 of the 600-chip adjustment, in big blinds.
        assert!((c.luck_bb - 600.0 * 4_000.0 / 4_060.0 / 20.0).abs() < 1e-9, "{}", c.luck_bb);
        // Against the folder: they folded on the flop, and no luck is theirs (they were not all in).
        let f = classify(&h, 4, 2, 50.0, 600.0, 4_060).unwrap();
        assert_eq!((f.ending, f.street, f.luck_bb), ("they fold", Street::Flop, 0.0));
        assert_eq!(classify(&h, 4, 0, 0.0, 600.0, 4_060), None, "no chips moved: not a confrontation");
    }

    /// 0317: the c-bet answer is found only when the rival raised last preflop, bet the flop first,
    /// and we acted after the bet — here villain5 raised, but we led the flop, so it is no c-bet.
    #[test]
    fn a_flop_c_bet_answer_needs_the_raiser_to_bet_first() {
        let h: HandSummary = serde_json::from_str(ALL_IN).unwrap();
        assert_eq!(cbet_answer(&h, 4), None, "we shoved first: not a c-bet");
        let mut bet = h.clone();
        // Seat 5 bets the flop first; we (seat 4) fold, then villain2 calls.
        bet.history.truncate(6);
        let r = |seat, kind, to, pot_before, to_call_before| sv10_core::engine::ActionRecord {
            seat,
            street: Street::Flop,
            kind,
            to,
            pot_before,
            to_call_before,
            bet_before: 0,
            full_raise: kind == ActionKind::Raise,
            think_ms: None,
            ..h.history[0].clone()
        };
        bet.history.push(r(4, ActionKind::Check, 0, 160, 0));
        bet.history.push(r(5, ActionKind::Raise, 100, 160, 0));
        bet.history.push(r(2, ActionKind::Call, 100, 260, 100));
        bet.history.push(r(4, ActionKind::Fold, 0, 360, 100));
        assert_eq!(cbet_answer(&bet, 4), Some((5, "fold")));
        assert_eq!(cbet_answer(&bet, 5), None, "the raiser does not answer its own c-bet");
    }

    /// The honest reading of "always win": a lower bound above zero, or how far away it is.
    #[test]
    fn hands_to_prove_a_win_follow_the_rate_and_the_spread() {
        let losing = vec![-1.0, 1.0, -2.0, 0.5];
        assert!(hands_to_prove(&losing).starts_with("not ahead"));
        // A mean of 0.1 bb with a 1 bb spread needs about (1.96 · 1 / 0.1)² ≈ 384 hands.
        let small: Vec<f64> = (0..100).map(|i| if i % 2 == 0 { 1.1 } else { -0.9 }).collect();
        let text = hands_to_prove(&small);
        assert!(text.starts_with("about 38"), "{text}");
        let clear: Vec<f64> = (0..400).map(|i| if i % 2 == 0 { 1.5 } else { -0.5 }).collect();
        assert!(hands_to_prove(&clear).starts_with("proven"));
    }
}

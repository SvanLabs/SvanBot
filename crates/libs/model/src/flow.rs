//! Chip flow between two players in a stored hand (0221): which of our chips an opponent actually
//! won, and which of theirs we won. Head-to-head results used to charge an opponent our whole net
//! in every hand they were dealt into, so a player who folded preflop while someone else stacked
//! us was blamed for it; flow counts only chips that moved between the two.
//!
//! Contributions are rebuilt from the action history: blinds from the button, then each action's
//! chips as the pot growth to the next record (server `AllIn` records carry no amount), the last
//! action from its own amounts, and the largest contribution capped at the second largest (an
//! uncalled bet is returned). A hand is used only when that rebuild reproduces the stored pot, so
//! a summary that misses a contribution is reported as `None`, never guessed.

use crate::model::HandSummary;
use std::collections::HashMap;
use sv10_engine::engine::{ActionKind, Street};

/// Chips each dealt-in seat put into the pot, uncalled chips returned, or `None` when the rebuild
/// does not reproduce the stored `pot`.
pub fn contributions(hand: &HandSummary, pot: i64) -> Option<HashMap<usize, i64>> {
    let out = reconstructed_contributions(hand)?;
    (out.values().sum::<i64>() == pot).then_some(out)
}

/// Rebuild eligible contribution levels from a complete summary, including uncalled refunds.
pub(crate) fn reconstructed_contributions(hand: &HandSummary) -> Option<HashMap<usize, i64>> {
    let mut seats: Vec<usize> = hand.players.iter().map(|p| p.0).collect();
    seats.sort_unstable();
    if seats.len() < 2 {
        return None;
    }
    let stacks: HashMap<usize, i64> = hand.stacks.iter().copied().collect();
    let stack = |s: usize| stacks.get(&s).copied().unwrap_or(i64::MAX);
    // Blinds sit clockwise from the button; heads-up the button posts the small blind.
    let after: Vec<usize> =
        seats.iter().copied().filter(|&s| s > hand.button).chain(seats.iter().copied().filter(|&s| s <= hand.button)).collect();
    let (sb, bb) = if seats.len() == 2 { (hand.button, after[0]) } else { (after[0], after[1]) };
    let mut out: HashMap<usize, i64> = seats.iter().map(|&s| (s, 0)).collect();
    *out.entry(sb).or_default() += (hand.bb / 2).min(stack(sb));
    *out.entry(bb).or_default() += hand.bb.min(stack(bb));
    let blinds: i64 = out.values().sum();
    if hand.history.first().is_some_and(|r| r.street == Street::Preflop && r.pot_before != blinds) {
        return None; // a dead blind or straddle this rebuild does not model
    }
    for (i, r) in hand.history.iter().enumerate() {
        let chips = match (hand.history.get(i + 1), r.kind) {
            (Some(next), _) => next.pot_before - r.pot_before,
            (None, ActionKind::Fold | ActionKind::Check) => 0,
            (None, ActionKind::AllIn) if r.to > 0 => r.to.checked_sub(r.bet_before)?,
            (None, ActionKind::AllIn) => stacks.get(&r.seat)?.checked_sub(out.get(&r.seat).copied().unwrap_or(0))?,
            (None, _) => r.to - r.bet_before,
        };
        if chips < 0 || !out.contains_key(&r.seat) {
            return None;
        }
        *out.get_mut(&r.seat)? += chips;
    }
    let mut sorted: Vec<i64> = out.values().copied().collect();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    if sorted[0] > sorted[1] {
        let top = out.iter().find(|(_, c)| **c == sorted[0]).map(|(s, _)| *s)?;
        out.insert(top, sorted[1]);
    }
    Some(out)
}

/// Chips `hero_seat` won from (positive) or lost to (negative) other seats, rebuilt per pot.
/// Ordinary ties use the global winner names. Side pots with several eligible winner names need
/// a complete board and those winners' shown cards to rank them; unavailable attribution is
/// `None`, as is an unreconciled pot or a hero who was not dealt in. Transfers split fractionally
/// between tied winners, excluding odd-chip rounding.
pub fn flow_to_hero(hand: &HandSummary, pot: i64, winners: &[&str], hero_seat: usize) -> Option<Vec<(usize, f64)>> {
    let c = contributions(hand, pot)?;
    c.get(&hero_seat)?;
    let won: Vec<usize> = hand.players.iter().filter(|(_, n)| winners.contains(&n.as_str())).map(|p| p.0).collect();
    let folded = |seat| hand.history.iter().any(|r| r.seat == seat && r.kind == ActionKind::Fold);
    if won.is_empty() || won.iter().any(|&s| folded(s)) {
        return None;
    }
    let mut levels: Vec<i64> = c.values().copied().filter(|&v| v > 0).collect();
    levels.sort_unstable();
    levels.dedup();
    // Unequal winner commitments can mean different winners of main and side pots.
    // Smaller folded contributions alone do not make an ordinary global tie ambiguous.
    let side_pots = won.iter().any(|s| c[s] != c[&won[0]]);
    let mut flows: HashMap<usize, f64> = hand.players.iter().filter(|(s, _)| *s != hero_seat).map(|&(s, _)| (s, 0.0)).collect();
    let mut paid = Vec::new();
    let mut previous = 0;
    for level in levels {
        let contributors: Vec<usize> = hand.players.iter().filter(|(s, _)| c[s] >= level).map(|p| p.0).collect();
        let mut eligible: Vec<usize> = won.iter().copied().filter(|s| c[s] >= level).collect();
        if eligible.is_empty() {
            return None;
        }
        if side_pots && eligible.len() > 1 {
            if hand.board.len() != 5 {
                return None;
            }
            let board = hand.board.iter().fold(0, |m, c| m | c.bit());
            let ranked: Vec<(usize, u32)> = eligible
                .iter()
                .map(|&s| {
                    let hole = hand.shown.iter().find(|(seat, _)| *seat == s)?.1;
                    Some((s, sv10_cards::eval::eval(board | hole[0].bit() | hole[1].bit())))
                })
                .collect::<Option<_>>()?;
            let best = ranked.iter().map(|r| r.1).max()?;
            eligible = ranked.into_iter().filter(|r| r.1 == best).map(|r| r.0).collect();
        }
        paid.extend(eligible.iter().copied());
        let chips = (level - previous) as f64 / eligible.len() as f64;
        let hero_won = eligible.contains(&hero_seat);
        if contributors.contains(&hero_seat) {
            for seat in contributors.into_iter().filter(|&s| s != hero_seat) {
                let other_won = eligible.contains(&seat);
                *flows.get_mut(&seat)? += match (hero_won, other_won) {
                    (true, false) => chips,
                    (false, true) => -chips,
                    _ => 0.0,
                };
            }
        }
        previous = level;
    }
    // A recorded global winner must win at least one reconstructed pot.
    if won.iter().any(|s| !paid.contains(s)) {
        return None;
    }
    Some(hand.players.iter().filter_map(|(s, _)| flows.get(s).map(|&v| (*s, v))).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side_pot() -> HandSummary {
        hand(
            r#"{"players":[[0,"hero"],[1,"short"],[2,"other"]],"button":0,"bb":20,"stacks":[[0,1000],[1,100],[2,1000]],"history":[{"seat":0,"street":"Preflop","kind":"AllIn","to":1000,"pot_before":30,"to_call_before":20,"bet_before":0,"full_raise":true},{"seat":1,"street":"Preflop","kind":"AllIn","to":100,"pot_before":1030,"to_call_before":990,"bet_before":10,"full_raise":false},{"seat":2,"street":"Preflop","kind":"AllIn","to":1000,"pot_before":1120,"to_call_before":980,"bet_before":20,"full_raise":false}],"board":["2c","3d","7h","9s","Tc"],"shown":[[0,["Kh","Kc"]],[1,["Ah","Ac"]],[2,["Qh","Qc"]]]}"#,
        )
    }

    #[test]
    fn side_pot_flows_reconcile_with_exact_engine_settlement() {
        let h = side_pot();
        let board = h.board.iter().fold(0, |m, c| m | c.bit());
        let values: Vec<_> = h.shown.iter().map(|(_, hole)| sv10_cards::eval::eval(board | hole[0].bit() | hole[1].bit())).collect();
        let payouts = sv10_engine::engine::split_pots(&[1000, 100, 1000], &[false; 3], &values, 0);
        assert_eq!(payouts, [1800, 300, 0]);
        for (hero, expected) in [(0, 800.0), (1, 200.0), (2, -1000.0)] {
            let flows = flow_to_hero(&h, 2100, &["hero", "short"], hero).unwrap();
            assert_eq!(flows.iter().map(|f| f.1).sum::<f64>(), expected);
        }
        let flows: HashMap<_, _> = flow_to_hero(&h, 2100, &["hero", "short"], 0).unwrap().into_iter().collect();
        assert_eq!((flows[&1], flows[&2]), (-100.0, 900.0));
    }

    #[test]
    fn ambiguous_side_pot_winners_without_cards_are_unavailable() {
        let mut h = side_pot();
        h.shown.clear();
        assert_eq!(flow_to_hero(&h, 2100, &["hero", "short"], 0), None);
        // A single global winner eligible for every pot needs no cards to identify it.
        let flow = flow_to_hero(&h, 2100, &["hero"], 0).unwrap();
        assert_eq!(flow.iter().map(|f| f.1).sum::<f64>(), 1100.0);
        h = side_pot();
        h.shown.retain(|p| p.0 != 1);
        assert_eq!(flow_to_hero(&h, 2100, &["hero", "short"], 0), None, "one missing winner's cards leaves allocation unknown");
        h = side_pot();
        h.board.pop();
        assert_eq!(flow_to_hero(&h, 2100, &["hero", "short"], 0), None, "the river is required to compare hands");
    }

    #[test]
    fn multiple_side_pots_and_a_tied_last_pot_reconcile_for_every_seat() {
        let mut h = side_pot();
        h.players = (0..4).map(|s| (s, format!("p{s}"))).collect();
        h.stacks = vec![(0, 100), (1, 200), (2, 300), (3, 300)];
        h.shown = [["Ah", "Ac"], ["Kh", "Kc"], ["Qh", "Qc"], ["Qd", "Qs"]]
            .into_iter()
            .enumerate()
            .map(|(s, cards)| (s, cards.map(|c| sv10_cards::cards::Card::parse(c).unwrap())))
            .collect();
        let mut pot = 30;
        h.history = h
            .stacks
            .iter()
            .map(|&(s, amount)| {
                let mut r = h.history[0].clone();
                r.seat = s;
                r.to = amount;
                r.bet_before = [0, 10, 20, 0][s];
                r.pot_before = pot;
                pot += amount - r.bet_before;
                r
            })
            .collect();
        for (hero, net) in [(0, 300.0), (1, 100.0), (2, -200.0), (3, -200.0)] {
            let flows = flow_to_hero(&h, 900, &["p0", "p1", "p2", "p3"], hero).unwrap();
            assert_eq!(flows.iter().map(|f| f.1).sum::<f64>(), net);
        }
    }

    /// A live hand as stored (2026-09-24): hero (seat 4, big blind) moves all in on the flop with
    /// an `AllIn` record carrying no amount, seat 5 raises to 4,629 and only 2,000 of it is called.
    const ALL_IN: &str = r#"{"players":[[0,"MissCard"],[1,"jonnaBee"],[2,"Bertabot"],[3,"RObert"],[4,"SurSvan"],[5,"POKER_STUDY_AI"]],"button":2,"bb":20,"history":[{"seat":5,"street":"Preflop","kind":"Raise","to":50,"pot_before":30,"to_call_before":20,"bet_before":0,"full_raise":true},{"seat":0,"street":"Preflop","kind":"Fold","to":0,"pot_before":80,"to_call_before":50,"bet_before":0,"full_raise":false},{"seat":1,"street":"Preflop","kind":"Fold","to":0,"pot_before":80,"to_call_before":50,"bet_before":0,"full_raise":false},{"seat":2,"street":"Preflop","kind":"Call","to":50,"pot_before":80,"to_call_before":50,"bet_before":0,"full_raise":false},{"seat":3,"street":"Preflop","kind":"Fold","to":0,"pot_before":130,"to_call_before":40,"bet_before":10,"full_raise":false},{"seat":4,"street":"Preflop","kind":"Call","to":50,"pot_before":130,"to_call_before":30,"bet_before":20,"full_raise":false},{"seat":4,"street":"Flop","kind":"AllIn","to":0,"pot_before":160,"to_call_before":0,"bet_before":0,"full_raise":false},{"seat":5,"street":"Flop","kind":"Raise","to":4629,"pot_before":2110,"to_call_before":1950,"bet_before":0,"full_raise":true},{"seat":2,"street":"Flop","kind":"Fold","to":0,"pot_before":6739,"to_call_before":4629,"bet_before":0,"full_raise":false}],"board":["5d","As","3s","6s","Th"],"shown":[[4,["Ts","Ac"]],[5,["Ah","5h"]]]}"#;

    fn hand(json: &str) -> HandSummary {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn all_in_amounts_come_from_the_pot_and_uncalled_chips_are_returned() {
        let c = contributions(&hand(ALL_IN), 4060).expect("reconciles");
        assert_eq!(c[&4], 2000, "hero's all-in: 20 blind + 30 call + 1,950 shove");
        assert_eq!(c[&5], 2000, "4,629 raise-to, only 2,000 of it called");
        assert_eq!((c[&2], c[&3], c[&0], c[&1]), (50, 10, 0, 0));
        assert_eq!(contributions(&hand(ALL_IN), 4061), None, "a pot the rebuild cannot reproduce is not guessed");
    }

    #[test]
    fn flow_charges_only_the_players_chips_moved_between() {
        let h = hand(ALL_IN);
        let f: HashMap<usize, f64> = flow_to_hero(&h, 4060, &["SurSvan"], 4).unwrap().into_iter().collect();
        assert_eq!((f[&5], f[&2], f[&3], f[&0]), (2000.0, 50.0, 10.0, 0.0));
        assert_eq!(f.values().sum::<f64>(), 2060.0, "a sole winner's flows add up to its net");
        let lost: HashMap<usize, f64> = flow_to_hero(&h, 4060, &["POKER_STUDY_AI"], 4).unwrap().into_iter().collect();
        assert_eq!(lost[&5], -2000.0, "all of hero's loss goes to the one winner");
        assert_eq!((lost[&2], lost[&3], lost[&0]), (0.0, 0.0, 0.0), "the folders took none of it");
    }

    #[test]
    fn split_pots_share_the_transfer_and_move_nothing_between_winners() {
        let h = hand(ALL_IN);
        let f: HashMap<usize, f64> = flow_to_hero(&h, 4060, &["SurSvan", "POKER_STUDY_AI"], 4).unwrap().into_iter().collect();
        assert_eq!((f[&5], f[&2], f[&3]), (0.0, 25.0, 5.0));
        assert_eq!(flow_to_hero(&h, 4060, &["Nobody"], 4), None);
        assert_eq!(flow_to_hero(&h, 4060, &["SurSvan"], 9), None, "hero must be dealt in");
    }

    #[test]
    fn a_walk_returns_the_big_blinds_uncalled_half() {
        // Everyone folds to the big blind (seat 2): its uncalled 10 comes back, pot 20.
        let walk = r#"{"players":[[0,"a"],[1,"b"],[2,"c"]],"button":0,"bb":20,"history":[{"seat":0,"street":"Preflop","kind":"Fold","to":0,"pot_before":30,"to_call_before":20,"bet_before":0,"full_raise":false},{"seat":1,"street":"Preflop","kind":"Fold","to":0,"pot_before":30,"to_call_before":10,"bet_before":10,"full_raise":false}],"board":[],"shown":[]}"#;
        let c = contributions(&hand(walk), 20).expect("reconciles");
        assert_eq!((c[&0], c[&1], c[&2]), (0, 10, 10));
        let f: HashMap<usize, f64> = flow_to_hero(&hand(walk), 20, &["c"], 2).unwrap().into_iter().collect();
        assert_eq!((f[&0], f[&1]), (0.0, 10.0));
    }

    #[test]
    fn a_blind_the_rebuild_does_not_model_is_rejected() {
        // A straddle: the first record sees 70 in the pot, not the 30 of the two blinds.
        let straddle = r#"{"players":[[0,"a"],[1,"b"],[2,"c"]],"button":0,"bb":20,"history":[{"seat":0,"street":"Preflop","kind":"Fold","to":0,"pot_before":70,"to_call_before":40,"bet_before":0,"full_raise":false}],"board":[],"shown":[]}"#;
        assert_eq!(contributions(&hand(straddle), 70), None);
    }
}

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
            (None, ActionKind::AllIn) => stack(r.seat).checked_sub(out.get(&r.seat).copied().unwrap_or(0))?,
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
    (out.values().sum::<i64>() == pot).then_some(out)
}

/// Chips `hero_seat` won from (positive) or lost to (negative) every other dealt-in seat, from
/// the reconciled `contributions` and the pot `winners` (names). Between a winner and a loser the
/// transfer is the smaller of their contributions (neither can win more than it covered), split
/// evenly when several players won; two winners or two losers move nothing between them. `None`
/// when the hand does not reconcile, hero was not dealt in or no winner was dealt in.
pub fn flow_to_hero(hand: &HandSummary, pot: i64, winners: &[&str], hero_seat: usize) -> Option<Vec<(usize, f64)>> {
    let c = contributions(hand, pot)?;
    let hero = *c.get(&hero_seat)?;
    let won: Vec<usize> = hand.players.iter().filter(|(_, n)| winners.contains(&n.as_str())).map(|p| p.0).collect();
    if won.is_empty() {
        return None;
    }
    let share = won.len() as f64;
    let hero_won = won.contains(&hero_seat);
    Some(
        hand.players
            .iter()
            .filter(|(s, _)| *s != hero_seat)
            .map(|&(s, _)| {
                let chips = match (hero_won, won.contains(&s)) {
                    (true, false) => c[&s].min(hero) as f64 / share,
                    (false, true) => -(hero.min(c[&s]) as f64) / share,
                    _ => 0.0,
                };
                (s, chips)
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic hand in the shape the server stores: hero (seat 4, big blind) moves all in on the
    /// flop with an `AllIn` record carrying no amount, seat 5 raises to 5,000 and only 3,000 of it is
    /// matched. The stored pot is 6,070 — the two matched 3,000s plus the small blind's 10 and seat
    /// 2's 60 preflop — so `pot` has to be this hand's own total, not an independent number.
    const ALL_IN: &str = r#"{"players":[[0,"villain0"],[1,"villain1"],[2,"villain2"],[3,"villain3"],[4,"SurSvan"],[5,"villain5"]],"button":2,"bb":20,"history":[{"seat":5,"street":"Preflop","kind":"Raise","to":60,"pot_before":30,"to_call_before":20,"bet_before":0,"full_raise":true},{"seat":0,"street":"Preflop","kind":"Fold","to":0,"pot_before":90,"to_call_before":60,"bet_before":0,"full_raise":false},{"seat":1,"street":"Preflop","kind":"Fold","to":0,"pot_before":90,"to_call_before":60,"bet_before":0,"full_raise":false},{"seat":2,"street":"Preflop","kind":"Call","to":60,"pot_before":90,"to_call_before":60,"bet_before":0,"full_raise":false},{"seat":3,"street":"Preflop","kind":"Fold","to":0,"pot_before":150,"to_call_before":50,"bet_before":10,"full_raise":false},{"seat":4,"street":"Preflop","kind":"Call","to":60,"pot_before":150,"to_call_before":40,"bet_before":20,"full_raise":false},{"seat":4,"street":"Flop","kind":"AllIn","to":0,"pot_before":190,"to_call_before":0,"bet_before":0,"full_raise":false},{"seat":5,"street":"Flop","kind":"Raise","to":5000,"pot_before":3130,"to_call_before":2940,"bet_before":0,"full_raise":true},{"seat":2,"street":"Flop","kind":"Fold","to":0,"pot_before":8130,"to_call_before":5000,"bet_before":0,"full_raise":false}],"board":["Kd","7h","2c","9s","3d"],"shown":[[4,["Ks","Qh"]],[5,["7d","7c"]]]}"#;

    fn hand(json: &str) -> HandSummary {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn all_in_amounts_come_from_the_pot_and_uncalled_chips_are_returned() {
        let c = contributions(&hand(ALL_IN), 6070).expect("reconciles");
        assert_eq!(c[&4], 3000, "hero's all-in: 20 blind + 40 call + 2,940 shove");
        assert_eq!(c[&5], 3000, "5,000 raise-to, matched only up to hero's 3,000");
        assert_eq!((c[&2], c[&3], c[&0], c[&1]), (60, 10, 0, 0));
        assert_eq!(contributions(&hand(ALL_IN), 6071), None, "a pot the rebuild cannot reproduce is not guessed");
    }

    #[test]
    fn flow_charges_only_the_players_chips_moved_between() {
        let h = hand(ALL_IN);
        let f: HashMap<usize, f64> = flow_to_hero(&h, 6070, &["SurSvan"], 4).unwrap().into_iter().collect();
        assert_eq!((f[&5], f[&2], f[&3], f[&0]), (3000.0, 60.0, 10.0, 0.0));
        assert_eq!(f.values().sum::<f64>(), 3070.0, "a sole winner's flows add up to its net");
        let lost: HashMap<usize, f64> = flow_to_hero(&h, 6070, &["villain5"], 4).unwrap().into_iter().collect();
        assert_eq!(lost[&5], -3000.0, "all of hero's loss goes to the one winner");
        assert_eq!((lost[&2], lost[&3], lost[&0]), (0.0, 0.0, 0.0), "the folders took none of it");
    }

    #[test]
    fn split_pots_share_the_transfer_and_move_nothing_between_winners() {
        let h = hand(ALL_IN);
        let f: HashMap<usize, f64> = flow_to_hero(&h, 6070, &["SurSvan", "villain5"], 4).unwrap().into_iter().collect();
        assert_eq!((f[&5], f[&2], f[&3]), (0.0, 30.0, 5.0));
        assert_eq!(flow_to_hero(&h, 6070, &["Nobody"], 4), None);
        assert_eq!(flow_to_hero(&h, 6070, &["SurSvan"], 9), None, "hero must be dealt in");
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

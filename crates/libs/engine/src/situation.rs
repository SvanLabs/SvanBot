//! The decision input shared by live play and simulation.

use crate::engine::{ActionRecord, Hand, Street};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use sv10_cards::cards::Card;

/// One dealt-in player as a decision sees them.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerInfo {
    /// Table seat.
    pub seat: usize,
    /// Player name (the key into opponent models).
    pub name: String,
    /// Chips behind (not yet committed).
    pub stack: i64,
    /// Chips committed on the current street.
    pub bet: i64,
    /// Whether they folded this hand.
    pub folded: bool,
}

/// Everything a decision needs, built identically from a live table or a simulated [`Hand`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Situation {
    /// Seat we decide for.
    pub hero_seat: usize,
    /// Our hole cards.
    pub hole: [Card; 2],
    /// Board cards so far.
    pub board: Vec<Card>,
    /// Current street.
    pub street: Street,
    /// Button seat.
    pub button: usize,
    /// Big blind in chips.
    pub bb: i64,
    /// Every chip in the middle, including current-street bets.
    pub pot: i64,
    /// Chips a call adds.
    pub call_amount: i64,
    /// Authoritative highest street total. `None` only in legacy serialized replays, which fall
    /// back to reconstructing the reachable price from player bets and the hero's call.
    #[serde(default)]
    pub current_bet_to: Option<i64>,
    /// Nothing is owed.
    pub can_check: bool,
    /// Smallest legal raise-to total, if raising is open.
    pub min_raise_to: Option<i64>,
    /// Largest legal raise-to total (all-in).
    pub max_raise_to: Option<i64>,
    /// Players dealt into the hand, in table (seat) order.
    pub players: Vec<PlayerInfo>,
    /// This hand's actions so far, blinds excluded.
    pub history: Vec<ActionRecord>,
}

impl Situation {
    /// Our player entry.
    pub fn hero(&self) -> &PlayerInfo {
        self.players.iter().find(|p| p.seat == self.hero_seat).expect("hero seated")
    }
    /// Highest street bet at the table.
    pub fn current_bet(&self) -> i64 {
        // A blind that is all-in short does not lower the full bring-in. In that case no player's
        // posted bet reaches the price, but the authoritative call amount still carries it.
        self.current_bet_to.unwrap_or_else(|| {
            let posted = self.players.iter().map(|p| p.bet).max().unwrap_or(0);
            posted.max(self.hero().bet.saturating_add(self.call_amount))
        })
    }
    /// Opponents who have not folded.
    pub fn live_opponents(&self) -> impl Iterator<Item = &PlayerInfo> {
        self.players.iter().filter(move |p| p.seat != self.hero_seat && !p.folded)
    }
    /// Position index counted from the button backwards among dealt players:
    /// 0 = button, 1 = cutoff, ... ; blinds are `n-2` (SB) and `n-1` (BB).
    /// Heads-up the button is also the small blind.
    pub fn position_of(&self, seat: usize) -> Position {
        position_of(&self.players.iter().map(|p| p.seat).collect::<Vec<_>>(), self.button, seat)
    }
    /// Whether `seat` acts after every other live player on postflop streets.
    pub fn in_position(&self, seat: usize) -> bool {
        let order = postflop_order(&self.players.iter().map(|p| p.seat).collect::<Vec<_>>(), self.button);
        let live: Vec<usize> = order.into_iter().filter(|s| self.players.iter().any(|p| p.seat == *s && !p.folded)).collect();
        live.last() == Some(&seat)
    }
    /// The situation as hero can actually play it: any opponent's street bet beyond hero's
    /// total reach (bet + stack) can never be matched, is returned to its owner, and must not
    /// count toward the pot hero is playing for.
    ///
    /// The price hero faces is the highest bet left standing, but never above what hero can
    /// reach and never below the authoritative bring-in hero can still cover: a short all-in
    /// blind posts less than the bring-in without lowering it for anyone else.
    pub fn without_uncallable(&self) -> Situation {
        let hero = self.hero();
        let reach = hero.bet + hero.stack;
        let mut out = self.clone();
        for p in out.players.iter_mut().filter(|p| p.seat != self.hero_seat) {
            let excess = p.bet - reach;
            if excess > 0 {
                p.bet = reach;
                p.stack += excess;
                out.pot -= excess;
            }
        }
        let standing = out.players.iter().map(|player| player.bet).max().unwrap_or(0);
        let playable = self.current_bet_to.map(|auth| auth.min(reach)).unwrap_or(0);
        out.current_bet_to = Some(standing.max(playable));
        out
    }

    /// Smaller of our total chips (stack + bet) and the deepest live opponent's.
    pub fn effective_stack(&self) -> i64 {
        let hero = self.hero();
        let hero_total = hero.stack + hero.bet;
        let opp_max = self.live_opponents().map(|p| p.stack + p.bet).max().unwrap_or(0);
        hero_total.min(opp_max)
    }

    /// Chips each player has committed this hand: completed streets from the history (street
    /// totals), the current street from live bets; rescaled to the pot so blinds without a
    /// history record are spread rather than lost.
    pub fn invested(&self) -> HashMap<usize, f64> {
        let mut street_max: HashMap<(usize, Street), i64> = HashMap::new();
        for r in self.history.iter().filter(|r| r.street != self.street) {
            let e = street_max.entry((r.seat, r.street)).or_default();
            *e = (*e).max(r.to);
        }
        let mut out: HashMap<usize, f64> = self.players.iter().map(|p| (p.seat, p.bet as f64)).collect();
        for ((seat, _), to) in street_max {
            *out.entry(seat).or_default() += to as f64;
        }
        let total: f64 = out.values().sum();
        if total > 0.0 {
            let k = self.pot as f64 / total;
            out.values_mut().for_each(|v| *v *= k);
        }
        out
    }

    /// When every opponent who can still act folds, hero does not win the whole pot if someone is
    /// already all-in: the chips above the largest all-in commitment are hero's, the rest goes to
    /// showdown. Returns (side pot won outright, main pot contested), summing to the pot.
    pub fn split_at_all_ins(&self, all_in_seats: &[usize]) -> (f64, f64) {
        let pot = self.pot as f64;
        if all_in_seats.is_empty() {
            return (pot, 0.0);
        }
        let inv = self.invested();
        let cap = all_in_seats.iter().map(|s| inv.get(s).copied().unwrap_or(0.0)).fold(0.0, f64::max);
        let side: f64 = inv.values().map(|v| (v - cap).max(0.0)).sum();
        (side.min(pot), (pot - side).max(0.0))
    }

    /// Pot slices after raising the current-street total to `raise_to` and having every opponent
    /// with chips behind fold. Each slice names the already all-in seats still eligible for it; an
    /// empty list is an uncalled refund or a pot won outright. A tier hero cannot reach is omitted.
    /// This refines only *unequal* all-in commitments. If history cannot reconstruct them, or they
    /// are equal, the caller keeps the existing one-pot estimate. The underlying commitments from
    /// [`Self::invested`] remain approximate when past action history is incomplete.
    pub fn unequal_all_in_tiers_after_raise(&self, all_in_seats: &[usize], raise_to: i64) -> Option<Vec<(f64, Vec<usize>)>> {
        let mut invested = self.invested();
        let mut caps: Vec<f64> = all_in_seats.iter().map(|seat| invested.get(seat).copied().unwrap_or(0.0)).collect();
        if caps.len() < 2 || caps.iter().any(|&cap| cap <= 0.0) {
            return None;
        }
        caps.sort_by(f64::total_cmp);
        caps.dedup();
        if caps.len() < 2 {
            return None;
        }
        let added = raise_to.checked_sub(self.hero().bet)?;
        if added < 0 || added > self.hero().stack {
            return None;
        }
        *invested.entry(self.hero_seat).or_default() += added as f64;
        let hero_invested = invested.get(&self.hero_seat).copied().unwrap_or(0.0);
        let mut levels: Vec<f64> = invested.values().copied().filter(|&amount| amount > 0.0).collect();
        levels.sort_by(f64::total_cmp);
        levels.dedup();
        let mut tiers: Vec<(f64, Vec<usize>)> = Vec::new();
        let mut previous = 0.0;
        for level in levels {
            let amount = (level - previous) * invested.values().filter(|&&contribution| contribution >= level).count() as f64;
            previous = level;
            if hero_invested < level {
                continue;
            }
            let eligible: Vec<usize> =
                all_in_seats.iter().copied().filter(|seat| invested.get(seat).is_some_and(|&v| v >= level)).collect();
            if let Some((last_amount, last_eligible)) = tiers.last_mut()
                && *last_eligible == eligible
            {
                *last_amount += amount;
            } else {
                tiers.push((amount, eligible));
            }
        }
        Some(tiers)
    }

    /// The situation of `seat` in a simulated hand, with `names` per seat.
    pub fn from_hand(hand: &Hand, seat: usize, names: &[String]) -> Situation {
        let legal = hand.legal();
        Situation {
            hero_seat: seat,
            hole: hand.seats[seat].hole,
            board: hand.board.clone(),
            street: hand.street,
            button: hand.button,
            bb: hand.bb,
            pot: hand.pot(),
            call_amount: legal.call_amount,
            current_bet_to: Some(hand.current_bet()),
            can_check: legal.can_check,
            min_raise_to: legal.min_raise_to,
            max_raise_to: legal.max_raise_to,
            players: hand
                .seats
                .iter()
                .enumerate()
                .map(|(i, s)| PlayerInfo { seat: i, name: names[i].clone(), stack: s.stack, bet: s.bet, folded: s.folded })
                .collect(),
            history: hand.history.clone(),
        }
    }
}

/// Table position class used by opponent statistics and ranges.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Position {
    /// Three or more seats before the button.
    Early,
    /// Two seats before the button.
    Middle,
    /// Directly before the button.
    Cutoff,
    /// The button (heads-up: also the small blind).
    Button,
    /// Small blind (three or more players).
    SmallBlind,
    /// Big blind.
    BigBlind,
}

impl Position {
    /// 0..6 in declaration order, for per-position arrays.
    pub fn index(self) -> usize {
        self as usize
    }
    /// Short label: EP, MP, CO, BTN, SB, BB.
    pub fn name(self) -> &'static str {
        ["EP", "MP", "CO", "BTN", "SB", "BB"][self as usize]
    }
}

/// Seats in postflop acting order (first to act first).
pub fn postflop_order(seats: &[usize], button: usize) -> Vec<usize> {
    let mut s = seats.to_vec();
    s.sort();
    let start = s.iter().position(|&x| x > button).unwrap_or(0);
    let mut out: Vec<usize> = s[start..].iter().chain(s[..start].iter()).copied().collect();
    if seats.len() == 2 {
        // Heads-up the button (small blind) acts last postflop.
        out.sort_by_key(|&x| x == button);
    }
    out
}

/// Position of `seat` among the dealt `seats` with the given `button`.
pub fn position_of(seats: &[usize], button: usize, seat: usize) -> Position {
    let n = seats.len();
    let order = postflop_order(seats, button); // sb, bb, ..., button
    let idx = order.iter().position(|&x| x == seat).unwrap_or(0);
    if n == 2 {
        return if seat == button { Position::Button } else { Position::BigBlind };
    }
    match idx {
        0 => Position::SmallBlind,
        1 => Position::BigBlind,
        _ => {
            let from_button = n - 1 - idx;
            match from_button {
                0 => Position::Button,
                1 => Position::Cutoff,
                2 => Position::Middle,
                _ => Position::Early,
            }
        }
    }
}

/// Shared test situations (used by this crate's and `sv10-policy`'s tests).
#[doc(hidden)]
pub mod fixtures {
    use super::*;

    /// River spot from live hand c27d0070 (2026-09-14): a 906,781-chip stack moved all-in,
    /// a short stack called 1,320, and hero (3,716 behind) faces a call of 3,716.
    pub fn uncallable_overshove() -> Situation {
        let p = |seat, name: &str, stack, bet, folded| PlayerInfo { seat, name: name.into(), stack, bet, folded };
        Situation {
            hero_seat: 0,
            hole: [Card::parse("6h").unwrap(), Card::parse("Ah").unwrap()],
            board: sv10_cards::cards::parse_cards(&["2h", "9s", "8s", "2s", "As"]).unwrap(),
            street: Street::River,
            button: 5,
            bb: 20,
            pot: 908_421,
            call_amount: 3_716,
            current_bet_to: Some(906_631),
            can_check: false,
            min_raise_to: None,
            max_raise_to: None,
            players: vec![
                p(0, "SuraGunnar", 3_716, 0, false),
                p(1, "silentflute", 0, 906_631, false),
                p(2, "L1RA_X", 6_918, 0, true),
                p(3, "x909", 8_573, 0, true),
                p(4, "mephisto1419", 0, 1_320, false),
                p(5, "coal78", 2_033, 0, true),
            ],
            history: vec![],
        }
    }

    /// River spot from live hand of Svanar, 2026-09-15 07:42 UTC: two short stacks are all-in, one
    /// deep player (QQ) can still act, hero holds 8s7h (a pair of sevens) with 4,787 behind.
    pub fn river_jam_with_all_ins() -> Situation {
        use crate::engine::{ActionKind::*, ActionRecord};
        let p = |seat, name: &str, stack, bet, folded| PlayerInfo { seat, name: name.into(), stack, bet, folded };
        let r = |seat, street, kind, to, pot_before| ActionRecord {
            seat,
            street,
            kind,
            to,
            pot_before,
            to_call_before: 0,
            bet_before: 0,
            full_raise: false,
            think_ms: None,
            street_open: false,
        };
        use Street::*;
        Situation {
            hero_seat: 4,
            hole: [Card::parse("7h").unwrap(), Card::parse("8s").unwrap()],
            board: sv10_cards::cards::parse_cards(&["2s", "4s", "7d", "4d", "9c"]).unwrap(),
            street: River,
            button: 3,
            bb: 20,
            pot: 12_536,
            call_amount: 0,
            current_bet_to: Some(0),
            can_check: true,
            min_raise_to: Some(20),
            max_raise_to: Some(4_787),
            players: vec![
                p(0, "montana2ab", 10_337, 0, false),
                p(1, "p1", 1_880, 0, true),
                p(2, "p2", 0, 0, false),
                p(3, "p3", 0, 0, false),
                p(4, "Svanar", 4_787, 0, false),
                p(5, "p5", 1_967, 0, true),
            ],
            history: vec![
                r(5, Preflop, Raise, 56, 30),
                r(0, Preflop, Raise, 168, 86),
                r(1, Preflop, Fold, 0, 254),
                r(2, Preflop, Call, 168, 254),
                r(3, Preflop, Call, 168, 422),
                r(4, Preflop, Call, 168, 580),
                r(5, Preflop, Call, 168, 728),
                r(3, Flop, Check, 0, 840),
                r(4, Flop, Raise, 672, 840),
                r(5, Flop, Fold, 0, 1_512),
                r(0, Flop, Call, 672, 1_512),
                r(2, Flop, Call, 672, 2_184),
                r(3, Flop, Raise, 1_202, 2_856),
                r(4, Flop, Call, 1_202, 4_058),
                r(0, Flop, Call, 1_202, 4_588),
                r(2, Flop, Call, 1_202, 5_118),
                r(4, Turn, Check, 0, 5_648),
                r(0, Turn, Check, 0, 5_648),
                r(2, Turn, AllIn, 130, 5_648),
                r(4, Turn, Raise, 3_379, 5_778),
                r(0, Turn, Call, 3_379, 9_157),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{river_jam_with_all_ins, uncallable_overshove};
    use super::*;

    #[test]
    fn short_big_blind_keeps_full_bring_in_after_uncallable_filter() {
        use sv10_rng::{SeedableRng, rngs::SmallRng};
        let mut rng = SmallRng::seed_from_u64(7);
        let hand = Hand::new(&[2_000, 2_000, 15], 0, 10, 20, &mut rng);
        let names = vec!["hero".to_string(), "sb".to_string(), "bb".to_string()];
        let sit = Situation::from_hand(&hand, 0, &names);
        assert_eq!(sit.current_bet(), 20);
        assert_eq!(sit.without_uncallable().current_bet(), 20, "a short all-in blind is not an uncallable excess");
    }

    #[test]
    fn uncallable_chips_return_to_their_owner() {
        let s = uncallable_overshove().without_uncallable();
        let shover = s.players.iter().find(|p| p.seat == 1).unwrap();
        assert_eq!(shover.bet, 3_716);
        assert_eq!(s.pot, 908_421 - (906_631 - 3_716));
        assert_eq!(s.current_bet(), 3_716);
        // Bets hero can cover are untouched.
        assert_eq!(s.players.iter().find(|p| p.seat == 4).unwrap().bet, 1_320);
    }

    #[test]
    fn all_in_split_sums_to_the_pot() {
        let sit = river_jam_with_all_ins();
        let inv = sit.invested();
        assert!((inv.values().sum::<f64>() - 12_536.0).abs() < 1.0);
        // Both short stacks put in 1,500 (seat 3 raised to 1,202 on the flop, seat 2 shoved 130 more on the turn).
        let (side, main) = sit.split_at_all_ins(&[2, 3]);
        assert!((side + main - 12_536.0).abs() < 1e-6);
        // Hero and the deep player are in for 4,749 each: about 3,250 each above the all-in level.
        assert!((6_000.0..7_000.0).contains(&side), "side pot {side}");
        assert_eq!(sit.split_at_all_ins(&[]), (12_536.0, 0.0));
    }

    #[test]
    fn unequal_all_ins_have_separate_showdown_tiers() {
        let mut sit = uncallable_overshove();
        let player = |seat, bet, stack| PlayerInfo { seat, name: format!("p{seat}"), stack, bet, folded: false };
        sit.hero_seat = 0;
        sit.players = vec![player(0, 400, 600), player(1, 100, 0), player(2, 300, 0), player(3, 0, 1_000)];
        sit.pot = 800;
        sit.history.clear();
        assert_eq!(sit.unequal_all_in_tiers_after_raise(&[1, 2], 400), Some(vec![(300.0, vec![1, 2]), (400.0, vec![2]), (100.0, vec![])]));
        let payouts = crate::engine::split_pots(&[400, 100, 300, 0], &[false, false, false, true], &[200, 300, 100, 0], 3);
        assert_eq!(payouts[0], 500, "hero loses the main pot, wins the deeper side pot and takes the uncalled refund");
        // The current wager is below both all-ins, but a proposed raise can still contest both.
        sit.players[0].bet = 0;
        sit.pot = 400;
        sit.current_bet_to = Some(300);
        sit.call_amount = 300;
        sit.can_check = false;
        sit.min_raise_to = Some(500);
        sit.max_raise_to = Some(600);
        assert!(sit.min_raise_to.unwrap() <= 500 && 500 <= sit.max_raise_to.unwrap());
        assert_eq!(sit.unequal_all_in_tiers_after_raise(&[1, 2], 500), Some(vec![(300.0, vec![1, 2]), (400.0, vec![2]), (200.0, vec![])]));
        assert_eq!(sit.unequal_all_in_tiers_after_raise(&[1, 2], 600), Some(vec![(300.0, vec![1, 2]), (400.0, vec![2]), (300.0, vec![])]));
        let raised_payouts = crate::engine::split_pots(&[500, 100, 300, 0], &[false, false, false, true], &[200, 300, 100, 0], 3);
        assert_eq!(raised_payouts[0] - 500, 100, "the called part of hero's raise remains at risk against the all-ins");
        // A snapshot without past betting history cannot assign side pots; preserve the old
        // single-showdown estimate instead of declaring the whole pot uncontested.
        sit.players[1].bet = 0;
        sit.players[2].bet = 0;
        assert_eq!(sit.unequal_all_in_tiers_after_raise(&[1, 2], 500), None);
    }

    #[test]
    fn positions_six_max_sparse_seats() {
        let seats = [0, 1, 2, 3, 4, 5];
        assert_eq!(position_of(&seats, 2, 3), Position::SmallBlind);
        assert_eq!(position_of(&seats, 2, 4), Position::BigBlind);
        assert_eq!(position_of(&seats, 2, 5), Position::Early);
        assert_eq!(position_of(&seats, 2, 0), Position::Middle);
        assert_eq!(position_of(&seats, 2, 1), Position::Cutoff);
        assert_eq!(position_of(&seats, 2, 2), Position::Button);
        let sparse = [0, 2, 5];
        assert_eq!(position_of(&sparse, 5, 0), Position::SmallBlind);
        assert_eq!(position_of(&sparse, 5, 2), Position::BigBlind);
        assert_eq!(position_of(&sparse, 5, 5), Position::Button);
        assert_eq!(postflop_order(&[1, 4], 4), vec![1, 4]);
        assert_eq!(position_of(&[1, 4], 4, 4), Position::Button);
    }
}

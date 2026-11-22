//! What v10.0.2 saw when it decided: every seat's stack, bet and fold, the hand's action record,
//! the button, and hero's legal bounds. v10's policy reads the whole table, so the port takes
//! the same shape rather than v11's [`svanbot_core::spot::Spot`].

use std::collections::BTreeMap;

use super::cards::Card;

/// A betting round.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Street {
    Preflop,
    Flop,
    Turn,
    River,
}

impl Street {
    pub fn index(self) -> usize {
        self as usize
    }

    /// The board cards dealt by this street.
    pub fn board_len(self) -> usize {
        [0, 3, 4, 5][self as usize]
    }
}

/// What a seat did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ActionKind {
    Fold,
    Check,
    Call,
    Raise,
    AllIn,
}

/// One action of the hand, with the table as it stood before it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ActionRecord {
    pub seat: usize,
    pub street: Street,
    pub kind: ActionKind,
    /// The seat's street bet after the action; 0 for a fold or a check.
    pub to: i64,
    /// The pot, current bets included, before the action.
    pub pot_before: i64,
    /// What the seat owed before it acted.
    pub to_call_before: i64,
    /// The seat's street bet before it acted.
    pub bet_before: i64,
    /// Whether an aggressive action was a full raise.
    pub full_raise: bool,
}

impl ActionRecord {
    /// A raise, or an all-in past a call.
    pub fn aggressive(&self) -> bool {
        match self.kind {
            ActionKind::Raise => true,
            ActionKind::AllIn => self.to > self.bet_before + self.to_call_before,
            _ => false,
        }
    }
}

/// One seat at the table.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlayerInfo {
    pub seat: usize,
    pub stack: i64,
    pub bet: i64,
    pub folded: bool,
}

/// A seat's place relative to the button.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Position {
    Early,
    Middle,
    Cutoff,
    Button,
    SmallBlind,
    BigBlind,
}

/// The table at hero's turn.
#[derive(Clone, PartialEq, Debug)]
pub struct Situation {
    pub hero_seat: usize,
    pub hole: [Card; 2],
    pub board: Vec<Card>,
    pub street: Street,
    pub button: usize,
    pub bb: i64,
    pub pot: i64,
    pub call_amount: i64,
    pub current_bet_to: Option<i64>,
    pub can_check: bool,
    pub min_raise_to: Option<i64>,
    pub max_raise_to: Option<i64>,
    pub players: Vec<PlayerInfo>,
    pub history: Vec<ActionRecord>,
}

impl Situation {
    pub fn hero(&self) -> &PlayerInfo {
        self.players
            .iter()
            .find(|p| p.seat == self.hero_seat)
            .expect("hero seated")
    }

    /// The street bet to match.
    pub fn current_bet(&self) -> i64 {
        self.current_bet_to.unwrap_or_else(|| {
            let posted = self.players.iter().map(|p| p.bet).max().unwrap_or(0);
            posted.max(self.hero().bet.saturating_add(self.call_amount))
        })
    }

    /// The seats still in the hand, hero aside.
    pub fn live_opponents(&self) -> impl Iterator<Item = &PlayerInfo> {
        self.players
            .iter()
            .filter(move |p| p.seat != self.hero_seat && !p.folded)
    }

    fn seats(&self) -> Vec<usize> {
        self.players.iter().map(|p| p.seat).collect()
    }

    pub fn position_of(&self, seat: usize) -> Position {
        position_of(&self.seats(), self.button, seat)
    }

    /// Whether `seat` acts last after the flop among the seats still in.
    pub fn in_position(&self, seat: usize) -> bool {
        let live: Vec<usize> = postflop_order(&self.seats(), self.button)
            .into_iter()
            .filter(|s| self.players.iter().any(|p| p.seat == *s && !p.folded))
            .collect();
        live.last() == Some(&seat)
    }

    /// The table with every bet past what hero can reach handed back to its owner.
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
        out.current_bet_to = Some(out.players.iter().map(|p| p.bet).max().unwrap_or(0));
        out
    }

    /// The most hero can win or lose against the deepest live opponent.
    pub fn effective_stack(&self) -> i64 {
        let hero = self.hero();
        let opp_max = self
            .live_opponents()
            .map(|p| p.stack + p.bet)
            .max()
            .unwrap_or(0);
        (hero.stack + hero.bet).min(opp_max)
    }

    /// What each seat has put in the pot, scaled to sum to the pot.
    pub fn invested(&self) -> BTreeMap<usize, f64> {
        let mut street_max: BTreeMap<(usize, Street), i64> = BTreeMap::new();
        for r in self.history.iter().filter(|r| r.street != self.street) {
            let e = street_max.entry((r.seat, r.street)).or_default();
            *e = (*e).max(r.to);
        }
        let mut out: BTreeMap<usize, f64> = self
            .players
            .iter()
            .map(|p| (p.seat, p.bet as f64))
            .collect();
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

    /// The pot split at the deepest of `all_in_seats`: the side pot past it and the main pot.
    pub fn split_at_all_ins(&self, all_in_seats: &[usize]) -> (f64, f64) {
        let pot = self.pot as f64;
        if all_in_seats.is_empty() {
            return (pot, 0.0);
        }
        let inv = self.invested();
        let cap = all_in_seats
            .iter()
            .map(|s| inv.get(s).copied().unwrap_or(0.0))
            .fold(0.0, f64::max);
        let side: f64 = inv.values().map(|v| (v - cap).max(0.0)).sum();
        (side.min(pot), (pot - side).max(0.0))
    }
}

/// The seats in postflop order, the button last; heads-up the button acts first preflop and last
/// after it.
pub fn postflop_order(seats: &[usize], button: usize) -> Vec<usize> {
    let mut sorted = seats.to_vec();
    sorted.sort_unstable();
    let start = sorted.iter().position(|&x| x > button).unwrap_or(0);
    let mut out: Vec<usize> = sorted[start..]
        .iter()
        .chain(sorted[..start].iter())
        .copied()
        .collect();
    if seats.len() == 2 {
        out.sort_by_key(|&x| x == button);
    }
    out
}

pub fn position_of(seats: &[usize], button: usize, seat: usize) -> Position {
    let n = seats.len();
    if n == 2 {
        return if seat == button {
            Position::Button
        } else {
            Position::BigBlind
        };
    }
    let order = postflop_order(seats, button);
    let idx = order.iter().position(|&x| x == seat).unwrap_or(0);
    match idx {
        0 => Position::SmallBlind,
        1 => Position::BigBlind,
        _ => match n - 1 - idx {
            0 => Position::Button,
            1 => Position::Cutoff,
            2 => Position::Middle,
            _ => Position::Early,
        },
    }
}

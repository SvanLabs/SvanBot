//! Exact multiway No-Limit Hold'em hand simulator (2-6 seats) with side pots,
//! full-raise reopening rules and uncalled-bet refunds. Drives self-play and
//! evaluation; the live client never uses it for authority.

use serde::{Deserialize, Serialize};
use sv10_cards::cards::{Card, CardMask};
use sv10_rng::{Rng, RngExt};

/// Betting round.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, PartialOrd, Ord, Hash)]
pub enum Street {
    /// Before any board card.
    Preflop,
    /// Three board cards.
    Flop,
    /// Four board cards.
    Turn,
    /// Five board cards.
    River,
}

impl Street {
    /// 0 (preflop) .. 3 (river), for per-street arrays.
    pub fn index(self) -> usize {
        self as usize
    }
    /// Parse the lowercase protocol name (`"preflop"`, `"flop"`, `"turn"`, `"river"`).
    pub fn from_name(s: &str) -> Option<Street> {
        match s {
            "preflop" => Some(Street::Preflop),
            "flop" => Some(Street::Flop),
            "turn" => Some(Street::Turn),
            "river" => Some(Street::River),
            _ => None,
        }
    }
    /// Lowercase protocol name.
    pub fn name(self) -> &'static str {
        ["preflop", "flop", "turn", "river"][self as usize]
    }
    /// Board cards visible on this street (0, 3, 4, 5).
    pub fn board_len(self) -> usize {
        [0, 3, 4, 5][self as usize]
    }
}

/// What kind of action a recorded action was.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum ActionKind {
    /// Gave up the hand.
    Fold,
    /// Passed with nothing to call.
    Check,
    /// Matched the current bet (partial calls for less are recorded as `AllIn`).
    Call,
    /// Bet or raise; `amount` is the raise-to total for the street.
    Raise,
    /// Put the whole stack in, whether as a call, a short raise or a full raise.
    AllIn,
}

/// An action to take in a [`Hand`] (the policy's output).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Fold.
    Fold,
    /// Check (only when nothing is owed).
    Check,
    /// Call the current bet, capped at the stack.
    Call,
    /// Bet or raise to this street total (clamped to the legal range by callers).
    RaiseTo(i64),
    /// Commit the whole stack.
    AllIn,
}

/// One action as it happened: the shared history format for sim and live play.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionRecord {
    /// Seat that acted.
    pub seat: usize,
    /// Street the action was taken on.
    pub street: Street,
    /// What was done.
    pub kind: ActionKind,
    /// Street bet total after the action (0 for fold/check).
    pub to: i64,
    /// Pot (all streets, current bets included) before the action.
    pub pot_before: i64,
    /// Chips the actor owed before acting.
    pub to_call_before: i64,
    /// The actor's street bet before acting.
    pub bet_before: i64,
    /// Whether this aggressive action was a full raise (reopens betting).
    pub full_raise: bool,
    /// Milliseconds between the previous table event and this action by the server's own clock
    /// (0234): the actor's think time, plus the server's pacing when the previous event was another
    /// action. `None` in simulations, for our own actions and for the first action after a resync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub think_ms: Option<u32>,
    /// Whether the previous table event opened the street (hand start or new cards) rather than
    /// another player's action: the server paces actions after an action (~3 s) but not after cards.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub street_open: bool,
}

/// The actor's legal options.
#[derive(Clone, Debug)]
pub struct Legal {
    /// Nothing is owed.
    pub can_check: bool,
    /// Chips a call adds (capped at the stack).
    pub call_amount: i64,
    /// Smallest raise-to total (an all-in below it when the stack is short); `None` if raising is closed.
    pub min_raise_to: Option<i64>,
    /// Largest raise-to total (all-in).
    pub max_raise_to: Option<i64>,
}

/// One seat's chips and cards during a hand.
#[derive(Clone, Debug)]
pub struct SeatState {
    /// Chips behind.
    pub stack: i64,
    /// Chips committed on the current street.
    pub bet: i64,
    /// Chips committed over the whole hand (blinds included).
    pub invested: i64,
    /// Whether the seat folded.
    pub folded: bool,
    /// Hole cards.
    pub hole: [Card; 2],
    /// Stack before blinds, for net results.
    pub start_stack: i64,
    can_raise: bool,
    needs_action: bool,
}

impl SeatState {
    /// Still in the hand with no chips behind.
    pub fn all_in(&self) -> bool {
        self.stack == 0 && !self.folded
    }
    fn can_act(&self) -> bool {
        !self.folded && self.stack > 0
    }
}

/// A hand in progress or finished: seats, board, betting state and history.
#[derive(Clone, Debug)]
pub struct Hand {
    /// Seats in table order.
    pub seats: Vec<SeatState>,
    /// Button seat index.
    pub button: usize,
    /// Small blind.
    pub sb: i64,
    /// Big blind.
    pub bb: i64,
    /// Current street.
    pub street: Street,
    /// Board cards dealt so far.
    pub board: Vec<Card>,
    /// Actions so far (blinds are not actions).
    pub history: Vec<ActionRecord>,
    runout: [Card; 5],
    current_bet: i64,
    last_full_raise: i64,
    actor: Option<usize>,
    finished: bool,
    payouts: Vec<i64>,
}

impl Hand {
    /// Deal a new hand. `stacks` are per seat (all > 0), seat order is table order.
    pub fn new<R: Rng>(stacks: &[i64], button: usize, sb: i64, bb: i64, rng: &mut R) -> Hand {
        let n = stacks.len();
        assert!((2..=10).contains(&n));
        let mut used: CardMask = 0;
        let mut draw = |rng: &mut R| loop {
            let c = rng.random_range(0..52u8);
            if used & (1 << c) == 0 {
                used |= 1 << c;
                break Card(c);
            }
        };
        let seats: Vec<SeatState> = stacks
            .iter()
            .map(|&s| SeatState {
                stack: s,
                bet: 0,
                invested: 0,
                folded: false,
                hole: [draw(rng), draw(rng)],
                start_stack: s,
                can_raise: true,
                needs_action: true,
            })
            .collect();
        let runout = [draw(rng), draw(rng), draw(rng), draw(rng), draw(rng)];
        Hand::with_cards(seats, runout, button, sb, bb)
    }

    /// Deal a hand with fixed hole cards and runout (tests, paired evaluation).
    pub fn with_cards(mut seats: Vec<SeatState>, runout: [Card; 5], button: usize, sb: i64, bb: i64) -> Hand {
        for s in seats.iter_mut() {
            s.start_stack = s.stack;
        }
        let n = seats.len();
        let mut h = Hand {
            seats,
            button,
            sb,
            bb,
            street: Street::Preflop,
            board: Vec::new(),
            history: Vec::new(),
            runout,
            current_bet: 0,
            last_full_raise: bb,
            actor: None,
            finished: false,
            payouts: vec![0; n],
        };
        let (sb_seat, bb_seat) = h.blind_seats();
        h.post(sb_seat, sb);
        h.post(bb_seat, bb);
        h.current_bet = bb.max(h.seats[sb_seat].bet);
        h.last_full_raise = bb;
        let first = if n == 2 { sb_seat } else { h.next_seat(bb_seat) };
        h.actor = Some(first);
        h.advance_if_needed(first);
        h
    }

    /// A fresh seat with `stack` and `hole`, for building hands with fixed cards.
    pub fn seat_state(stack: i64, hole: [Card; 2]) -> SeatState {
        SeatState { stack, bet: 0, invested: 0, folded: false, hole, start_stack: stack, can_raise: true, needs_action: true }
    }

    /// (small blind, big blind) seats; heads-up the button posts the small blind.
    pub fn blind_seats(&self) -> (usize, usize) {
        let n = self.seats.len();
        if n == 2 {
            (self.button, (self.button + 1) % n)
        } else {
            let sb = (self.button + 1) % n;
            (sb, (sb + 1) % n)
        }
    }

    fn next_seat(&self, s: usize) -> usize {
        (s + 1) % self.seats.len()
    }

    fn post(&mut self, seat: usize, amount: i64) {
        let s = &mut self.seats[seat];
        let a = amount.min(s.stack);
        s.stack -= a;
        s.bet += a;
        s.invested += a;
    }

    /// All chips invested this hand.
    pub fn pot(&self) -> i64 {
        self.seats.iter().map(|s| s.invested).sum()
    }

    /// Highest street bet.
    pub fn current_bet(&self) -> i64 {
        self.current_bet
    }

    /// Seat to act, or `None` once the hand is over.
    pub fn actor(&self) -> Option<usize> {
        if self.finished { None } else { self.actor }
    }

    /// Whether the hand has been settled.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// The full five-card board fixed at deal time (streets reveal it progressively).
    pub fn runout(&self) -> &[Card; 5] {
        &self.runout
    }

    /// The current actor's legal options. Panics if the hand is over.
    pub fn legal(&self) -> Legal {
        let seat = self.actor.expect("no actor");
        let s = &self.seats[seat];
        let to_call = (self.current_bet - s.bet).max(0);
        let call_amount = to_call.min(s.stack);
        let max_to = s.bet + s.stack;
        let others_can_act = self.seats.iter().enumerate().any(|(i, o)| i != seat && o.can_act());
        let (mut min_raise_to, mut max_raise_to) = (None, None);
        if s.can_raise && max_to > self.current_bet && others_can_act {
            let min_to = self.current_bet + self.last_full_raise;
            min_raise_to = Some(min_to.min(max_to));
            max_raise_to = Some(max_to);
        }
        Legal { can_check: to_call == 0, call_amount, min_raise_to, max_raise_to }
    }

    /// Apply the actor's action, advance streets and settle when the hand ends; `Err` for an
    /// illegal action or a finished hand.
    pub fn apply(&mut self, action: Action) -> Result<(), String> {
        let seat = self.actor.ok_or("hand is over")?;
        let legal = self.legal();
        let pot_before = self.pot();
        let to_call_before = (self.current_bet - self.seats[seat].bet).max(0);
        let mut rec = ActionRecord {
            seat,
            street: self.street,
            kind: ActionKind::Fold,
            to: 0,
            pot_before,
            to_call_before,
            bet_before: self.seats[seat].bet,
            full_raise: false,
            think_ms: None,
            street_open: false,
        };
        match action {
            Action::Fold => {
                self.seats[seat].folded = true;
                self.seats[seat].needs_action = false;
            }
            Action::Check => {
                if !legal.can_check {
                    return Err("check not legal".into());
                }
                rec.kind = ActionKind::Check;
                self.seats[seat].needs_action = false;
                self.seats[seat].can_raise = false;
            }
            Action::Call => {
                if legal.can_check {
                    rec.kind = ActionKind::Check;
                } else {
                    let a = legal.call_amount;
                    let s = &mut self.seats[seat];
                    s.stack -= a;
                    s.bet += a;
                    s.invested += a;
                    rec.kind = if s.stack == 0 { ActionKind::AllIn } else { ActionKind::Call };
                    rec.to = s.bet;
                }
                self.seats[seat].needs_action = false;
                self.seats[seat].can_raise = false;
            }
            Action::RaiseTo(_) | Action::AllIn => {
                let (min_to, max_to) = match (legal.min_raise_to, legal.max_raise_to) {
                    (Some(a), Some(b)) => (a, b),
                    _ => {
                        if matches!(action, Action::AllIn) {
                            // All-in without raise rights is a call for the whole stack.
                            return self.apply(Action::Call);
                        }
                        return Err("raise not legal".into());
                    }
                };
                let to = match action {
                    Action::AllIn => max_to,
                    Action::RaiseTo(t) => t.clamp(min_to.min(max_to), max_to),
                    _ => unreachable!(),
                };
                let s = &mut self.seats[seat];
                let add = to - s.bet;
                s.stack -= add;
                s.bet = to;
                s.invested += add;
                let raise_size = to - self.current_bet;
                let full = raise_size >= self.last_full_raise;
                rec.kind = if s.stack == 0 { ActionKind::AllIn } else { ActionKind::Raise };
                rec.to = to;
                rec.full_raise = full;
                if full {
                    self.last_full_raise = raise_size;
                }
                self.current_bet = to;
                for (i, o) in self.seats.iter_mut().enumerate() {
                    if i != seat && o.can_act() {
                        o.needs_action = true;
                        if full {
                            o.can_raise = true;
                        }
                    }
                }
                let s = &mut self.seats[seat];
                s.needs_action = false;
                s.can_raise = false;
            }
        }
        self.history.push(rec);
        self.advance_if_needed(self.next_seat(seat));
        Ok(())
    }

    fn advance_if_needed(&mut self, mut from: usize) {
        loop {
            let live: Vec<usize> = (0..self.seats.len()).filter(|&i| !self.seats[i].folded).collect();
            if live.len() == 1 {
                self.finish();
                return;
            }
            let n = self.seats.len();
            let mut next = None;
            for k in 0..n {
                let i = (from + k) % n;
                let s = &self.seats[i];
                if s.can_act() && (s.needs_action || s.bet < self.current_bet) {
                    // A lone player with chips facing no bet has nobody to play against.
                    let others = self.seats.iter().enumerate().filter(|(j, o)| *j != i && o.can_act()).count();
                    if others == 0 && s.bet >= self.current_bet {
                        continue;
                    }
                    next = Some(i);
                    break;
                }
            }
            if let Some(i) = next {
                self.actor = Some(i);
                return;
            }
            if self.street == Street::River {
                self.finish();
                return;
            }
            self.next_street();
            from = self.next_seat(self.button);
        }
    }

    fn next_street(&mut self) {
        self.street = match self.street {
            Street::Preflop => Street::Flop,
            Street::Flop => Street::Turn,
            Street::Turn => Street::River,
            Street::River => Street::River,
        };
        self.board = self.runout[..self.street.board_len()].to_vec();
        self.current_bet = 0;
        self.last_full_raise = self.bb;
        for s in self.seats.iter_mut() {
            s.bet = 0;
            if s.can_act() {
                s.needs_action = true;
                s.can_raise = true;
            }
        }
    }

    fn finish(&mut self) {
        self.finished = true;
        self.actor = None;
        let payouts = self.settle(&self.runout);
        if self.seats.iter().filter(|s| !s.folded).count() > 1 {
            self.board = self.runout.to_vec();
        }
        for (i, s) in self.seats.iter_mut().enumerate() {
            s.stack += payouts[i];
        }
        self.payouts = payouts;
    }
}

mod settle;
pub use settle::split_pots;
#[cfg(test)]
mod tests;

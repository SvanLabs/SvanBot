//! v10.0.2 at the sim's table: a seat that reads the whole table ([`Seen`]) the way v10's engine
//! showed a hand to its policy, and plays the action that policy takes.
//!
//! Each decision is priced afresh, so the seat keeps nothing between hands and is the same on
//! any thread: given the spot, the table and the seed, it takes the same action.

use svanbot_core::card::Card as CoreCard;
use svanbot_core::rng::Rng;
use svanbot_core::rules::{Action, Chips, Legal, Street as CoreStreet};
use svanbot_core::spot::Spot;

use super::cards::Card;
use super::params::Params;
use super::policy::{self, decide};
use super::situation::{ActionKind, ActionRecord, PlayerInfo, Situation, Street};
use crate::table::{Record, Seen};

/// The Arena's name for this opponent.
pub const NAME: &str = "v10.0.2";

/// v10.0.2 with its knobs at `Params::default()`.
#[derive(Clone, Debug, Default)]
pub struct V10 {
    params: Params,
}

impl V10 {
    /// The action v10.0.2 takes at `seat`, seeded by `seed`.
    pub fn decide(&self, seat: usize, spot: &Spot, seen: &Seen, seed: u64) -> Action {
        let sit = situation(seat, spot, seen);
        let mut rng = Rng::seeded(seed);
        played(decide(&sit, &self.params, &mut rng), spot.legal)
    }
}

/// The hand at `seat`'s turn, as v10's engine put it.
pub fn situation(seat: usize, spot: &Spot, seen: &Seen) -> Situation {
    let bets = seen.seats.iter().map(|s| s.bet as i64);
    let current_bet = bets.max().unwrap_or(0);
    let mut hole = spot.hole.iter().map(card);
    let hole = [
        hole.next().expect("two hole cards"),
        hole.next().expect("two hole cards"),
    ];
    Situation {
        hero_seat: seat,
        hole,
        board: seen.board.iter().copied().map(card).collect(),
        street: street(spot_street(seen.board.len())),
        button: seen.button,
        bb: (spot.big_blind as i64),
        pot: (spot.pot as i64),
        call_amount: spot.legal.call.map_or(0, |chips| chips as i64),
        current_bet_to: Some(current_bet),
        can_check: spot.legal.call.is_none(),
        min_raise_to: spot.legal.raise.map(|r| r.min as i64),
        max_raise_to: spot.legal.raise.map(|r| r.max as i64),
        players: seen
            .seats
            .iter()
            .enumerate()
            .map(|(at, s)| PlayerInfo {
                seat: at,
                stack: (s.stack as i64),
                bet: (s.bet as i64),
                folded: s.folded,
            })
            .collect(),
        history: seen.record.iter().map(record).collect(),
    }
}

/// v10's action as the table's: an all-in is a raise to the stack, or a call when no raise is
/// left; a raise is held to the legal range.
pub fn played(action: policy::Action, legal: Legal) -> Action {
    let call = || match legal.call {
        Some(_) => Action::Call,
        None => Action::Check,
    };
    match action {
        policy::Action::Fold if legal.call.is_none() => Action::Check,
        policy::Action::Fold => Action::Fold,
        policy::Action::Check | policy::Action::Call => call(),
        policy::Action::RaiseTo(to) => match legal.raise {
            Some(range) => {
                let to = Chips::try_from(to.max(0)).unwrap_or(Chips::MAX);
                Action::RaiseTo(to.clamp(range.min, range.max))
            }
            None => call(),
        },
        policy::Action::AllIn => match legal.raise {
            Some(range) => Action::RaiseTo(range.max),
            None => call(),
        },
    }
}

fn card(card: CoreCard) -> Card {
    Card::parse(&card.to_string()).expect("both encodings name the same 52 cards")
}

fn spot_street(board: usize) -> CoreStreet {
    match board {
        0 => CoreStreet::Preflop,
        3 => CoreStreet::Flop,
        4 => CoreStreet::Turn,
        _ => CoreStreet::River,
    }
}

fn street(street: CoreStreet) -> Street {
    match street {
        CoreStreet::Preflop => Street::Preflop,
        CoreStreet::Flop => Street::Flop,
        CoreStreet::Turn => Street::Turn,
        CoreStreet::River => Street::River,
    }
}

fn record(r: &Record) -> ActionRecord {
    let kind = match r.action {
        _ if r.all_in => ActionKind::AllIn,
        Action::Fold => ActionKind::Fold,
        Action::Check => ActionKind::Check,
        Action::Call => ActionKind::Call,
        Action::RaiseTo(_) => ActionKind::Raise,
    };
    ActionRecord {
        seat: r.seat,
        street: street(r.street),
        kind,
        to: (r.to as i64),
        pot_before: (r.pot_before as i64),
        to_call_before: (r.owed_before as i64),
        bet_before: (r.bet_before as i64),
        full_raise: false,
    }
}

//! Server hand histories (`GET /api/me/hand-history`) replayed into `HandSummary`s so past
//! seasons feed the same opponent statistics and training data as live play.
//!
//! The export lists every seat's actions but not seat names (only winners are named), not
//! blinds, not stacks, and no amount for calls or all-ins. The replay here reconstructs street
//! bets, pot sizes and to-call amounts; unknown stacks are estimated from our own start stack.

use crate::model::HandSummary;
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};
use sv10_cards::cards::Card;
use sv10_engine::engine::{ActionKind, ActionRecord, Street};

/// One action in a server hand-history export.
#[derive(Clone, Debug, Deserialize)]
pub struct RawAction {
    /// Seat that acted.
    pub seat: usize,
    /// `fold`, `check`, `call`, `raise` or `all_in`.
    pub action: String,
    /// Raise-to total for raises; 0 for calls and all-ins (the export omits them).
    #[serde(default)]
    pub amount: i64,
    /// Street label; the export gives the street *after* the action.
    pub street: String,
    /// Order within the hand.
    #[serde(default)]
    pub sequence_in_hand: i64,
}

/// A pot winner as named by the export.
#[derive(Clone, Debug, Deserialize)]
pub struct RawWinner {
    /// Winner's seat.
    pub seat: usize,
    /// Winner's name (the only names the export carries).
    #[serde(default)]
    pub name: String,
}

/// One hand from `GET /api/me/hand-history`, from our bot's point of view.
#[derive(Clone, Debug, Deserialize)]
pub struct RawHand {
    /// Server hand id.
    pub hand_id: String,
    /// Server table id.
    #[serde(default)]
    pub table_id: String,
    /// Hand number at the table (orders hands for name attribution).
    #[serde(default)]
    pub hand_number: i64,
    /// Our bot's seat.
    pub seat: usize,
    /// Our stack at the start of the hand.
    #[serde(default)]
    pub stack_start: i64,
    /// Our net chips for the hand.
    #[serde(default)]
    pub profit: i64,
    /// Every seat's actions.
    #[serde(default)]
    pub all_actions: Vec<RawAction>,
    /// Start time (RFC 3339).
    #[serde(default)]
    pub started_at: String,
    /// Board cards as text.
    #[serde(default)]
    pub board: Vec<String>,
    /// Final pot.
    #[serde(default)]
    pub pot_total: i64,
    /// Players dealt in.
    #[serde(default)]
    pub num_players: usize,
    /// Small blind (default 10).
    #[serde(default = "default_sb")]
    pub small_blind: i64,
    /// Big blind (default 20).
    #[serde(default = "default_bb")]
    pub big_blind: i64,
    /// Button seat.
    pub dealer_seat: usize,
    /// Pot winners.
    #[serde(default)]
    pub winners: Vec<RawWinner>,
    /// Our hole cards.
    #[serde(default)]
    pub hole_cards: Vec<String>,
    /// Cards shown at showdown, by seat number as text.
    #[serde(default)]
    pub shown_cards: HashMap<String, Vec<String>>,
}

fn default_sb() -> i64 {
    10
}
fn default_bb() -> i64 {
    20
}

impl RawHand {
    /// Every seat known to be dealt in: actors, winners, shown hands and ours.
    pub fn seats(&self) -> Vec<usize> {
        let mut s: BTreeSet<usize> = self.all_actions.iter().map(|a| a.seat).collect();
        s.extend(self.winners.iter().map(|w| w.seat));
        s.extend(self.shown_cards.keys().filter_map(|k| k.parse::<usize>().ok()));
        s.insert(self.seat);
        s.into_iter().collect()
    }
}

fn street_of(label: &str) -> Option<Street> {
    match label {
        "preflop" => Some(Street::Preflop),
        "flop" => Some(Street::Flop),
        "turn" => Some(Street::Turn),
        "river" => Some(Street::River),
        _ => None,
    }
}

/// Seats clockwise starting after the button.
fn after_button(seats: &[usize], button: usize) -> Vec<usize> {
    let start = seats.iter().position(|&s| s > button).unwrap_or(0);
    (0..seats.len()).map(|i| seats[(start + i) % seats.len()]).collect()
}

/// Replay one exported hand. `names` maps seats to known player names; unnamed seats get
/// `anon` (callers use a reserved prefix so they only feed population statistics).
/// Returns `None` for hands that cannot be replayed consistently.
pub fn to_summary(raw: &RawHand, names: &HashMap<usize, String>, anon: &dyn Fn(usize) -> String) -> Option<HandSummary> {
    let seats = raw.seats();
    if seats.len() < 2 || raw.all_actions.is_empty() {
        return None;
    }
    let bb = raw.big_blind.max(1);
    let sb = raw.small_blind.max(0);
    let est_stack = if raw.stack_start > 0 { raw.stack_start } else { 100 * bb };
    let mut stack: HashMap<usize, i64> = seats.iter().map(|&s| (s, est_stack)).collect();
    let mut bet: HashMap<usize, i64> = HashMap::new();
    let order = after_button(&seats, raw.dealer_seat);
    let (sb_seat, bb_seat) = if seats.len() == 2 {
        // Heads-up: the button posts the small blind.
        let other = *seats.iter().find(|&&s| s != raw.dealer_seat).unwrap_or(&order[0]);
        (if seats.contains(&raw.dealer_seat) { raw.dealer_seat } else { order[1] }, other)
    } else {
        (order[0], order[1])
    };
    let mut pot = 0;
    let mut contrib: HashMap<usize, i64> = seats.iter().map(|&s| (s, 0)).collect();
    for (seat, amt) in [(sb_seat, sb), (bb_seat, bb)] {
        let a = amt.min(stack[&seat]);
        *stack.get_mut(&seat)? -= a;
        bet.insert(seat, a);
        pot += a;
        *contrib.get_mut(&seat)? += a;
    }
    let mut level = bb;
    let mut last_raise = bb;
    let mut street = Street::Preflop;
    let mut history = Vec::new();
    let mut actions = raw.all_actions.clone();
    actions.sort_by_key(|a| a.sequence_in_hand);
    let mut prev_label = "preflop".to_string();
    for a in &actions {
        // `street` on each action is the street after it; the street it was taken on is the
        // previous action's label.
        let Some(on) = street_of(&prev_label) else { break };
        prev_label = a.street.clone();
        if on != street {
            street = on;
            bet.clear();
            level = 0;
            last_raise = bb;
        }
        let seat = a.seat;
        let &behind = stack.get(&seat)?;
        let before = bet.get(&seat).copied().unwrap_or(0);
        let to_call = (level - before).max(0);
        let (kind, to) = match a.action.as_str() {
            "fold" => (ActionKind::Fold, 0),
            "check" => (ActionKind::Check, 0),
            "call" => (ActionKind::Call, before + to_call.min(behind)),
            "raise" => {
                let min_to = level + last_raise;
                let target = if a.amount > level { a.amount } else { min_to };
                (ActionKind::Raise, target.min(before + behind))
            }
            // The export gives our stack and nobody else's, so another seat's shove would be priced
            // from *our* stack (est_stack) and read as an aggression we never faced. The pot is the
            // sum of every contribution, so it bounds what the seat wagered: cap an unknown seat's
            // shove there. The cap never prices the shove below what the rules allow a shove to be
            // -- the call it faces, or a minimum bet when it faces none -- so a shove that really
            // did aggress keeps its kind and stays aggressive to the tallies (1011).
            "all_in" if seat != raw.seat => {
                let call_to = before + to_call.min(behind);
                let floor = if to_call > 0 { call_to } else { before + bb.min(behind) };
                let room = (raw.pot_total - contrib[&seat]).max(0);
                (ActionKind::AllIn, (before + room).min(before + behind).max(floor))
            }
            "all_in" => (ActionKind::AllIn, before + behind),
            _ => return None,
        };
        let full_raise = to - level >= last_raise && to > level;
        history.push(ActionRecord {
            seat,
            street,
            kind,
            to,
            pot_before: pot,
            to_call_before: to_call,
            bet_before: before,
            full_raise,
            think_ms: None,
            street_open: false,
        });
        if to > before {
            let add = to - before;
            *stack.get_mut(&seat)? -= add;
            pot += add;
            *contrib.get_mut(&seat)? += add;
            bet.insert(seat, to);
        }
        if to > level {
            if full_raise {
                last_raise = to - level;
            }
            level = to;
        }
    }
    let board: Vec<Card> = raw.board.iter().filter_map(|c| Card::parse(c)).collect();
    let mut shown: Vec<(usize, [Card; 2])> = raw
        .shown_cards
        .iter()
        .filter_map(|(k, v)| {
            let seat = k.parse::<usize>().ok()?;
            let c: Vec<Card> = v.iter().filter_map(|x| Card::parse(x)).collect();
            (c.len() == 2).then(|| (seat, [c[0], c[1]]))
        })
        .collect();
    shown.sort_by_key(|s| s.0);
    let players = seats.iter().map(|&s| (s, names.get(&s).cloned().unwrap_or_else(|| anon(s)))).collect();
    let stacks = seats.iter().map(|&s| (s, if s == raw.seat && raw.stack_start > 0 { raw.stack_start } else { est_stack })).collect();
    Some(HandSummary { players, button: raw.dealer_seat, bb, history, board, shown, stacks })
}

/// Name each seat of each hand at one table from winner sightings: a seat is attributed to a
/// player when the nearest sightings before and after agree, or when the only sighting is
/// within `reach` hands. Input: hands of one table sorted by hand number.
pub fn attribute_names(hands: &[&RawHand], reach: i64) -> Vec<HashMap<usize, String>> {
    let mut sightings: HashMap<usize, Vec<(i64, String)>> = HashMap::new();
    for h in hands {
        for w in h.winners.iter().filter(|w| !w.name.is_empty()) {
            sightings.entry(w.seat).or_default().push((h.hand_number, w.name.clone()));
        }
    }
    hands
        .iter()
        .map(|h| {
            let mut out = HashMap::new();
            for seat in h.seats() {
                let Some(list) = sightings.get(&seat) else { continue };
                let idx = list.partition_point(|(n, _)| *n < h.hand_number);
                let next = list.get(idx);
                let prev = idx.checked_sub(1).and_then(|i| list.get(i));
                let name = match (prev, next) {
                    (_, Some((n, name))) if *n == h.hand_number => Some(name),
                    (Some((_, a)), Some((_, b))) if a == b => Some(a),
                    (Some((pn, a)), None) if h.hand_number - pn <= reach => Some(a),
                    (None, Some((nn, b))) if nn - h.hand_number <= reach => Some(b),
                    (Some((pn, a)), Some((nn, b))) => {
                        // Different names: trust only a close sighting on the near side.
                        if h.hand_number - pn <= reach / 3 && nn - h.hand_number > reach {
                            Some(a)
                        } else if nn - h.hand_number <= reach / 3 && h.hand_number - pn > reach {
                            Some(b)
                        } else {
                            None
                        }
                    }
                    _ => None,
                };
                if let Some(n) = name {
                    out.insert(seat, n.clone());
                }
            }
            out
        })
        .collect()
}

#[cfg(test)]
mod tests;

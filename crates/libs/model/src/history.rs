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
    for (seat, amt) in [(sb_seat, sb), (bb_seat, bb)] {
        let a = amt.min(stack[&seat]);
        *stack.get_mut(&seat)? -= a;
        bet.insert(seat, a);
        pot += a;
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
mod tests {
    use super::*;

    fn raw(json: &str) -> RawHand {
        serde_json::from_str(json).unwrap()
    }

    const HAND: &str = r#"{"hand_id":"h1","table_id":"t","hand_number":5,"seat":1,"stack_start":2000,"profit":-10,
        "all_actions":[
          {"seat":0,"action":"fold","amount":0,"street":"preflop","sequence_in_hand":1},
          {"seat":1,"action":"fold","amount":0,"street":"preflop","sequence_in_hand":2},
          {"seat":2,"action":"raise","amount":45,"street":"preflop","sequence_in_hand":3},
          {"seat":3,"action":"fold","amount":0,"street":"preflop","sequence_in_hand":4},
          {"seat":4,"action":"call","amount":0,"street":"preflop","sequence_in_hand":5},
          {"seat":5,"action":"fold","amount":0,"street":"flop","sequence_in_hand":6},
          {"seat":4,"action":"check","amount":0,"street":"flop","sequence_in_hand":7},
          {"seat":2,"action":"raise","amount":60,"street":"flop","sequence_in_hand":8},
          {"seat":4,"action":"raise","amount":200,"street":"flop","sequence_in_hand":9},
          {"seat":2,"action":"call","amount":0,"street":"turn","sequence_in_hand":10},
          {"seat":4,"action":"check","amount":0,"street":"turn","sequence_in_hand":11},
          {"seat":2,"action":"check","amount":0,"street":"river","sequence_in_hand":12},
          {"seat":4,"action":"check","amount":0,"street":"river","sequence_in_hand":13},
          {"seat":2,"action":"check","amount":0,"street":"showdown","sequence_in_hand":14}],
        "board":["Qc","Jc","5c","3c","8s"],"pot_total":510,"num_players":6,"small_blind":10,"big_blind":20,
        "dealer_seat":3,"winners":[{"seat":2,"name":"gouda"}],"hole_cards":["6d","5h"],
        "shown_cards":{"4":["Td","As"],"2":["Qh","Ts"]}}"#;

    #[test]
    fn replays_bets_streets_and_pot() {
        let r = raw(HAND);
        let s = to_summary(&r, &HashMap::from([(2, "gouda".to_string())]), &|seat| format!("\u{0}anon{seat}")).unwrap();
        assert_eq!(s.players.len(), 6);
        assert_eq!(s.history.len(), 14);
        // SB (seat 4) calls 45 facing a raise: to-call 35.
        let call = &s.history[4];
        assert_eq!(
            (call.street, call.kind, call.to, call.to_call_before, call.bet_before),
            (Street::Preflop, ActionKind::Call, 45, 35, 10)
        );
        // BB folds preflop even though its label says flop.
        assert_eq!((s.history[5].street, s.history[5].kind), (Street::Preflop, ActionKind::Fold));
        // Flop: pot 110, bet 60, check-raise to 200 is a full raise, call 140.
        let bet = &s.history[7];
        assert_eq!((bet.street, bet.to, bet.pot_before, bet.to_call_before), (Street::Flop, 60, 110, 0));
        let raise = &s.history[8];
        assert!(raise.full_raise && raise.to == 200 && raise.to_call_before == 60);
        let call = &s.history[9];
        assert_eq!((call.street, call.to, call.to_call_before), (Street::Flop, 200, 140));
        let last = s.history.last().unwrap();
        assert_eq!(last.street, Street::River);
        // Pot reconstruction matches the export: 30 blinds + 25 + 35 + 60 + 200 + 140 + ... = 510.
        assert_eq!(last.pot_before, 510);
        assert_eq!(s.shown.len(), 2);
        assert_eq!(s.players.iter().find(|p| p.0 == 2).unwrap().1, "gouda");
    }

    /// Replays real exported pages (HISTORY_SAMPLES=dir of page JSON files) and reports how
    /// often the reconstructed pot matches the server's pot total.
    #[test]
    #[ignore]
    fn replays_real_exports() {
        let Ok(dir) = std::env::var("HISTORY_SAMPLES") else { return };
        let (mut n, mut ok, mut failed, mut named) = (0, 0, 0, 0);
        for f in std::fs::read_dir(dir).unwrap().flatten() {
            let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(f.path()).unwrap()).unwrap();
            let hands: Vec<RawHand> = serde_json::from_value(v["hands"].clone()).unwrap();
            let refs: Vec<&RawHand> = hands.iter().collect();
            named += attribute_names(&refs, 40).iter().map(|m| m.len()).sum::<usize>();
            for h in &hands {
                n += 1;
                match to_summary(h, &HashMap::new(), &|s| format!("x{s}")) {
                    None => failed += 1,
                    Some(s) => {
                        // Final pot less the uncalled part of the last street's top bet.
                        let last = s.history.last().unwrap();
                        let mut pot = last.pot_before + (last.to - last.bet_before).max(0);
                        let mut bets: HashMap<usize, i64> = HashMap::new();
                        for r in s.history.iter().filter(|r| r.street == last.street) {
                            if r.to > 0 {
                                bets.insert(r.seat, r.to);
                            }
                        }
                        let mut v: Vec<i64> = bets.values().copied().collect();
                        v.sort_unstable_by(|a, b| b.cmp(a));
                        if last.street == Street::Preflop {
                            // Blinds count as street bets preflop.
                            v.extend([h.small_blind, h.big_blind]);
                            v.sort_unstable_by(|a, b| b.cmp(a));
                        }
                        if v.len() >= 2 {
                            pot -= v[0] - v[1];
                        } else if v.len() == 1 {
                            pot -= v[0];
                        }
                        if (pot - h.pot_total).abs() <= h.big_blind {
                            ok += 1
                        } else if std::env::var("HISTORY_VERBOSE").is_ok() && n % 3 == 0 {
                            eprintln!(
                                "pot {pot} vs {} dealer {} :: {:?}",
                                h.pot_total,
                                h.dealer_seat,
                                h.all_actions
                                    .iter()
                                    .map(|a| format!("{}:{}:{}:{}", a.seat, a.action, a.amount, a.street))
                                    .collect::<Vec<_>>()
                            );
                        }
                    }
                }
            }
        }
        eprintln!("hands {n} failed {failed} pot-match {ok} ({:.1}%) named seats {named}", 100.0 * ok as f64 / n.max(1) as f64);
    }

    #[test]
    fn names_follow_winner_sightings() {
        let mk = |n: i64, winner: Option<(usize, &str)>| {
            let mut r = raw(HAND);
            r.hand_number = n;
            r.winners = winner.map(|(s, name)| vec![RawWinner { seat: s, name: name.into() }]).unwrap_or_default();
            r
        };
        let hands = [mk(1, Some((4, "a"))), mk(2, None), mk(3, Some((4, "a"))), mk(4, None), mk(90, Some((4, "b")))];
        let refs: Vec<&RawHand> = hands.iter().collect();
        let names = attribute_names(&refs, 30);
        assert_eq!(names[1].get(&4).map(String::as_str), Some("a"));
        assert_eq!(names[3].get(&4).map(String::as_str), Some("a"));
        assert_eq!(names[4].get(&4).map(String::as_str), Some("b"));
    }
}

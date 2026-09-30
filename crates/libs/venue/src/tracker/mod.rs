//! Per-table state built from the server's authoritative `table_state` snapshots plus the event stream (which alone carries action history).

use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use sv10_cards::cards::Card;
use sv10_engine::engine::{ActionKind, ActionRecord, Street};
use sv10_engine::situation::{PlayerInfo, Situation};
use sv10_model::model::HandSummary;

/// One seat as the latest snapshot and events show it.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct SeatView {
    /// Seat number.
    pub seat: usize,
    /// Player name.
    pub name: String,
    /// Chips behind.
    pub stack: i64,
    /// Chips committed this street.
    pub bet: i64,
    /// Dealt into the current hand.
    pub in_hand: bool,
    /// Folded this hand.
    pub folded: bool,
    /// Server seat status text.
    pub status: String,
    /// The player's avatar image (`table_state` `avatar_url`): an http(s) URL or a site path.
    #[serde(default)]
    pub avatar_url: Option<String>,
}

/// The server's `valid_actions` for our turn; only these may be sent.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct LegalActions {
    /// Fold is offered.
    pub can_fold: bool,
    /// Check is offered.
    pub can_check: bool,
    /// Call amount, when a call is offered.
    pub call: Option<i64>,
    /// Smallest raise-to total, when a raise is offered.
    pub raise_min: Option<i64>,
    /// Largest raise-to total, when a raise is offered.
    pub raise_max: Option<i64>,
    /// All-in amount, when offered.
    pub all_in: Option<i64>,
}

impl LegalActions {
    /// Read a `valid_actions` array; unknown entries are ignored.
    pub fn parse(valid: &Value) -> LegalActions {
        let mut l = LegalActions::default();
        for a in valid.as_array().into_iter().flatten() {
            match a["action"].as_str().unwrap_or("") {
                "fold" => l.can_fold = true,
                "check" => l.can_check = true,
                "call" => l.call = a["amount"].as_i64().or(Some(0)),
                "raise" => {
                    l.raise_min = a["min"].as_i64();
                    l.raise_max = a["max"].as_i64();
                }
                "all_in" => l.all_in = a["amount"].as_i64().or(Some(0)),
                _ => {}
            }
        }
        l
    }
}

/// A completed hand extracted at `hand_result`.
#[derive(Clone, Debug)]
pub struct FinishedHand {
    /// Server hand id.
    pub hand_id: String,
    /// Our stack after settlement, when reported.
    pub hero_final_stack: Option<i64>,
    /// The hand in the shared format (models, training, storage).
    pub summary: HandSummary,
    /// Our seat, when seated.
    pub hero_seat: Option<usize>,
    /// Our net chips for the hand, when our start and final stacks are known.
    pub hero_net: Option<i64>,
    /// Our hole cards.
    pub hero_hole: Option<[Card; 2]>,
    /// Final board.
    pub board: Vec<Card>,
    /// Final pot.
    pub pot: i64,
    /// Names of the pot winners.
    pub winners: Vec<String>,
}

/// One sent decision awaiting settlement against the hero's stack before that action.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingCalibration {
    /// Stable spot category used by the applied bias table.
    pub category: String,
    /// Candidate EV before the current calibration bias, in chips.
    pub predicted_incremental_chips: f64,
    /// Hero chips behind immediately before the legalized action commits chips. Candidate EV is
    /// measured from this same frontier because call and raise costs are already subtracted.
    pub hero_stack_before_action: i64,
    /// Pot plus amount owed at the decision, in chips.
    pub scale_chips: i64,
}

/// Realized incremental chips measured from the same pre-action stack frontier as candidate EV.
pub fn realized_incremental_chips(final_stack: i64, hero_stack_before_action: i64) -> i64 {
    final_stack.saturating_sub(hero_stack_before_action)
}

/// State of one table for one bot: the authoritative `table_state` snapshot merged with the event
/// stream, which alone carries the action history.
#[derive(Default, Debug)]
pub struct TableTracker {
    /// Current table.
    pub table_id: Option<String>,
    /// Our seat.
    pub hero_seat: Option<usize>,
    /// Hand in progress.
    pub hand_id: Option<String>,
    /// Button seat.
    pub dealer: usize,
    /// Small blind.
    pub sb: i64,
    /// Big blind.
    pub bb: i64,
    /// Current street, if a hand is running.
    pub street: Option<Street>,
    /// Board cards.
    pub board: Vec<Card>,
    /// Pot from the latest snapshot.
    pub pot: i64,
    /// Seats by number.
    pub seats: BTreeMap<usize, SeatView>,
    /// Our hole cards.
    pub hole: Option<[Card; 2]>,
    /// Actions this hand, blinds excluded.
    pub history: Vec<ActionRecord>,
    /// Highest `table_seq` accepted (duplicate and out-of-order guard).
    pub last_table_seq: i64,
    last_full_raise: i64,
    start_stacks: HashMap<usize, i64>,
    dealt: Vec<(usize, String)>,
    /// Our sent decisions this hand awaiting settlement from their explicit pre-action stack frontier.
    pub pending_calibration: Vec<PendingCalibration>,
    /// Hands completed at the current table.
    pub hands_at_table: u32,
    /// Server time (ms) of the last event that starts a player's clock — hand start, new cards or an
    /// action — and whether it opened the street (0234). `None` until one is seen and after a resync.
    last_event: Option<(i64, bool)>,
}

/// Longest think time recorded (ms): the 45 s turn clock plus pacing; anything longer spans a gap
/// in the event stream, not a decision.
pub const MAX_THINK_MS: i64 = 60_000;

/// Milliseconds since the Unix epoch of an RFC 3339 timestamp (`2026-09-14T09:46:52.751076+00:00`,
/// `...Z`), without a date library: the server stamps every frame with `ts`.
pub fn rfc3339_ms(ts: &str) -> Option<i64> {
    let b = ts.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !(b[10] == b'T' || b[10] == b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| ts.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, sec) = (num(0..4)?, num(5..7)?, num(8..10)?, num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut i = 19;
    let mut ms = 0i64;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        let frac = &ts[start..i];
        ms = frac.chars().chain(std::iter::repeat('0')).take(3).collect::<String>().parse().ok()?;
    }
    let offset_min = match b.get(i) {
        Some(b'Z') | Some(b'z') => 0,
        Some(&sign @ (b'+' | b'-')) => {
            let oh = num(i + 1..i + 3)?;
            let om = num(i + 4..i + 6)?;
            let m = oh * 60 + om;
            if sign == b'+' { m } else { -m }
        }
        _ => return None,
    };
    // Days from the civil date (H. Hinnant's algorithm).
    let yy = if mo <= 2 { y - 1 } else { y };
    let era = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe = yy - era * 400;
    let doy = (153 * ((mo + 9) % 12) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    Some(((days * 86_400 + h * 3_600 + mi * 60 + sec - offset_min * 60) * 1_000) + ms)
}

fn server_ms(msg: &Value) -> Option<i64> {
    msg["ts"].as_str().and_then(rfc3339_ms)
}

fn cards(v: &Value) -> Vec<Card> {
    v.as_array().into_iter().flatten().filter_map(|c| c.as_str().and_then(Card::parse)).collect()
}

fn hole(v: &Value) -> Option<[Card; 2]> {
    let c = cards(v);
    if c.len() == 2 { Some([c[0], c[1]]) } else { None }
}

impl TableTracker {
    /// Forget everything (new table), keeping default blinds 10/20.
    pub fn reset_table(&mut self) {
        *self = TableTracker { bb: 20, sb: 10, ..Default::default() };
    }

    /// Track the envelope watermark; returns false for duplicates/regressions.
    pub fn accept_seq(&mut self, msg: &Value) -> bool {
        match msg["table_seq"].as_i64() {
            Some(seq) if seq <= self.last_table_seq && msg["type"] != "resync_response" => {
                // table_state snapshots can legitimately repeat the watermark after acks.
                msg["type"] == "table_state" && seq == self.last_table_seq
            }
            Some(seq) => {
                self.last_table_seq = self.last_table_seq.max(seq);
                true
            }
            None => true,
        }
    }

    /// Apply `table_joined`: table id, our seat, seated players (resets on a table change).
    pub fn table_joined(&mut self, msg: &Value) {
        let new_table = msg["table_id"].as_str().map(|s| s.to_string());
        if new_table != self.table_id {
            self.reset_table();
        }
        self.table_id = new_table;
        self.hero_seat = msg["seat"].as_u64().map(|s| s as usize);
        for p in msg["players"].as_array().into_iter().flatten() {
            let seat = p["seat"].as_u64().unwrap_or(0) as usize;
            let e = self.seats.entry(seat).or_default();
            e.seat = seat;
            e.name = p["name"].as_str().unwrap_or("").to_string();
            e.stack = p["stack"].as_i64().unwrap_or(0);
        }
    }

    /// Apply an authoritative `table_state` snapshot.
    pub fn table_state(&mut self, msg: &Value) {
        if let Some(t) = msg["table_id"].as_str() {
            self.table_id = Some(t.to_string());
        }
        let street = msg["street"].as_str().and_then(Street::from_name);
        if let Some(h) = msg["hand_id"].as_str()
            && street.is_some()
            && self.hand_id.as_deref() != Some(h)
        {
            self.begin_hand(h, msg["dealer_seat"].as_u64().unwrap_or(0) as usize);
        }
        if street != self.street {
            self.last_full_raise = self.bb.max(1);
        }
        self.street = street;
        self.dealer = msg["dealer_seat"].as_u64().map(|d| d as usize).unwrap_or(self.dealer);
        self.sb = msg["small_blind"].as_i64().unwrap_or(self.sb);
        self.bb = msg["big_blind"].as_i64().unwrap_or(self.bb);
        if street.is_some() {
            self.board = self.recovery_board(cards(&msg["board"]), street == Some(Street::Preflop));
            self.pot = msg["pot"].as_i64().unwrap_or(self.pot);
        }
        let mut present = Vec::new();
        for s in msg["seats"].as_array().into_iter().flatten() {
            let seat = s["seat"].as_u64().unwrap_or(0) as usize;
            let status = s["status"].as_str().unwrap_or("").to_string();
            if status == "empty" {
                self.seats.remove(&seat);
                continue;
            }
            present.push(seat);
            let prev_folded = self.seats.get(&seat).map(|x| x.folded).unwrap_or(false);
            let e = self.seats.entry(seat).or_default();
            e.seat = seat;
            e.name = s["name"].as_str().unwrap_or(&e.name).to_string();
            e.stack = s["stack"].as_i64().unwrap_or(e.stack);
            e.bet = s["bet"].as_i64().unwrap_or(0);
            e.in_hand = s["in_hand"].as_bool().unwrap_or(false);
            // `folded` is authoritative when present; otherwise keep a same-hand marker.
            e.folded = match s["folded"].as_bool() {
                Some(f) => f,
                None => e.in_hand && prev_folded,
            };
            e.status = status;
            // Absent keeps the last avatar; null or anything but an http(s) URL or a site path
            // ("/api/public-avatar/...") clears it (0180).
            if let Some(a) = s.get("avatar_url") {
                e.avatar_url = a
                    .as_str()
                    .filter(|u| u.starts_with("https://") || u.starts_with("http://") || (u.starts_with('/') && !u.starts_with("//")))
                    .map(str::to_string);
            }
        }
        self.seats.retain(|k, _| present.contains(k));
        if let Some(hero) = msg.get("hero") {
            if let Some(seat) = hero["seat"].as_u64() {
                self.hero_seat = Some(seat as usize);
            }
            if let Some(h) = hole(&hero["hole_cards"]) {
                self.hole = Some(h);
            }
        }
        if street == Some(Street::Preflop) {
            for s in self.seats.values() {
                if s.in_hand {
                    self.start_stacks.entry(s.seat).or_insert(s.stack.saturating_add(s.bet));
                }
            }
            if self.dealt.is_empty() {
                self.dealt = self.seats.values().filter(|s| s.in_hand).map(|s| (s.seat, s.name.clone())).collect();
            }
        }
    }

    fn begin_hand(&mut self, hand_id: &str, dealer: usize) {
        self.hand_id = Some(hand_id.to_string());
        self.dealer = dealer;
        self.history.clear();
        self.hole = None;
        self.board.clear();
        self.start_stacks.clear();
        self.dealt.clear();
        self.pending_calibration.clear();
        self.last_full_raise = self.bb.max(1);
        for s in self.seats.values_mut() {
            s.folded = false;
        }
    }

    /// Apply `hand_start`: new hand id, button, our seat and blinds.
    pub fn hand_start(&mut self, msg: &Value) {
        let hid = msg["hand_id"].as_str().unwrap_or("").to_string();
        if self.hand_id.as_deref() != Some(hid.as_str()) {
            self.begin_hand(&hid, msg["dealer_seat"].as_u64().unwrap_or(0) as usize);
        }
        if let Some(seat) = msg["seat"].as_u64() {
            self.hero_seat = Some(seat as usize);
        }
        if let Some(b) = msg.get("blinds") {
            self.sb = b["small_blind"].as_i64().unwrap_or(self.sb);
            self.bb = b["big_blind"].as_i64().unwrap_or(self.bb);
        }
        self.street = Some(Street::Preflop);
        self.last_event = server_ms(msg).map(|t| (t, true));
    }

    /// Forget the event clock (before a resync replays events): the next action gets no think time
    /// rather than one that spans the gap.
    pub fn clear_event_clock(&mut self) {
        self.last_event = None;
    }

    /// Apply `hole_cards`.
    pub fn hole_cards(&mut self, msg: &Value) {
        self.hole = hole(&msg["cards"]);
    }

    /// Apply `player_action`, recording it in the history (its `street` is the street after the action).
    pub fn player_action(&mut self, msg: &Value) {
        let Some(seat) = msg["seat"].as_u64().map(|s| s as usize) else { return };
        let street = self.street.unwrap_or(Street::Preflop);
        let kind = match msg["action"].as_str().unwrap_or("") {
            "fold" => ActionKind::Fold,
            "check" => ActionKind::Check,
            "call" => ActionKind::Call,
            "raise" | "bet" => ActionKind::Raise,
            "all_in" => ActionKind::AllIn,
            _ => return,
        };
        let cur_bet = self.seats.values().map(|s| s.bet).max().unwrap_or(0);
        let seat_view = self.seats.entry(seat).or_default();
        let bet_before = seat_view.bet;
        // Chips put in by this action: prefer the explicit delta, then the stack change,
        // then the amount (raise-to totals vs incremental amounts are flagged by amount_mode).
        let stack_delta = match (msg["stack_before"].as_i64(), msg["stack_after"].as_i64()) {
            (Some(a), Some(b)) => a.saturating_sub(b),
            _ => match msg["stack"].as_i64() {
                Some(st) if seat_view.stack > st => seat_view.stack.saturating_sub(st),
                _ => 0,
            },
        };
        let amount = msg["amount"].as_i64().unwrap_or(0);
        let pot_delta = match (msg["pot_before"].as_i64(), msg["pot_after"].as_i64()) {
            (Some(a), Some(b)) => b.saturating_sub(a),
            _ => 0,
        };
        let delta = match msg["contribution_delta"].as_i64() {
            Some(d) if d > 0 => d,
            _ if pot_delta > 0 && kind != ActionKind::Fold && kind != ActionKind::Check => pot_delta,
            _ if stack_delta > 0 => stack_delta,
            // Shoves arrive with `amount: 0`: an all-in puts in everything the player had behind.
            _ if kind == ActionKind::AllIn && amount <= 0 && seat_view.stack > 0 => seat_view.stack,
            _ if amount > 0 && kind != ActionKind::Fold && kind != ActionKind::Check => {
                if msg["amount_mode"].as_str() == Some("to_total") || kind == ActionKind::Raise || kind == ActionKind::AllIn {
                    amount.saturating_sub(bet_before).max(0)
                } else {
                    amount
                }
            }
            _ => 0,
        };
        let to = if delta > 0 { bet_before.saturating_add(delta) } else { 0 };
        let to_call_before = msg["to_call_before"].as_i64().unwrap_or(cur_bet.saturating_sub(bet_before).max(0));
        let pot_before = msg["pot_before"].as_i64().unwrap_or(self.pot);
        let aggressive = kind == ActionKind::Raise || (kind == ActionKind::AllIn && to > cur_bet);
        let full_raise = aggressive && to.saturating_sub(cur_bet) >= self.last_full_raise;
        if full_raise {
            self.last_full_raise = to.saturating_sub(cur_bet);
        }
        // Think time by the server's clock (0234): only opponents, only when the previous event is known.
        let now = server_ms(msg);
        let (think_ms, street_open) = match (now, self.last_event) {
            (Some(t), Some((prev, open))) if Some(seat) != self.hero_seat && (0..=MAX_THINK_MS).contains(&(t - prev)) => {
                (Some((t - prev) as u32), open)
            }
            _ => (None, false),
        };
        if now.is_some() {
            self.last_event = now.map(|t| (t, false));
        }
        self.history.push(ActionRecord {
            seat,
            street,
            kind,
            to,
            pot_before,
            to_call_before,
            bet_before,
            full_raise,
            think_ms,
            street_open,
        });
        seat_view.bet = bet_before.saturating_add(delta.max(0));
        if let Some(st) = msg["stack_after"].as_i64().or(msg["stack"].as_i64()) {
            seat_view.stack = st;
        }
        if kind == ActionKind::Fold {
            seat_view.folded = true;
        }
        if let Some(p) = msg["pot_after"].as_i64().or(msg["pot"].as_i64()) {
            self.pot = p;
        }
    }

    /// Apply `community_cards`: board and street change (street bets reset).
    pub fn community_cards(&mut self, msg: &Value) {
        self.board = cards(&msg["cards"]);
        if let Some(t) = server_ms(msg) {
            self.last_event = Some((t, true));
        }
        if let Some(s) = msg["street"].as_str().and_then(Street::from_name)
            && Some(s) != self.street
        {
            self.street = Some(s);
            self.last_full_raise = self.bb.max(1);
            for v in self.seats.values_mut() {
                v.bet = 0;
            }
        }
    }

    /// Apply `hand_result` and return the finished hand, or `None` without a hand id.
    pub fn hand_result(&mut self, msg: &Value) -> Option<FinishedHand> {
        let hand_id = msg["hand_id"].as_str().map(|s| s.to_string()).or_else(|| self.hand_id.clone())?;
        let mut shown = Vec::new();
        if let Some(obj) = msg["shown_cards"].as_object() {
            for (k, v) in obj {
                if let (Ok(seat), Some(h)) = (k.parse::<usize>(), hole(v)) {
                    shown.push((seat, h));
                }
            }
        }
        let final_stacks: HashMap<usize, i64> = msg["final_stacks"]
            .as_object()
            .map(|o| o.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.as_i64()?))).collect())
            .unwrap_or_default();
        for (seat, st) in &final_stacks {
            if let Some(s) = self.seats.get_mut(seat) {
                s.stack = *st;
                s.bet = 0;
            }
        }
        let players = if self.dealt.is_empty() {
            self.seats.values().filter(|s| s.in_hand).map(|s| (s.seat, s.name.clone())).collect()
        } else {
            self.dealt.clone()
        };
        let hero_net = self.hero_seat.and_then(|h| Some(final_stacks.get(&h)?.saturating_sub(*self.start_stacks.get(&h)?)));
        let winners = msg["winners"].as_array().into_iter().flatten().filter_map(|w| w["name"].as_str().map(String::from)).collect();
        let finished = FinishedHand {
            hand_id: hand_id.clone(),
            hero_final_stack: self.hero_seat.and_then(|h| final_stacks.get(&h).copied()),
            summary: HandSummary {
                players,
                button: self.dealer,
                bb: self.bb,
                history: self.history.clone(),
                board: self.board.clone(),
                shown,
                stacks: self.start_stacks.iter().map(|(s, v)| (*s, *v)).collect(),
            },
            hero_seat: self.hero_seat,
            hero_net,
            hero_hole: self.hole,
            board: self.board.clone(),
            pot: msg["total_pot"].as_i64().or(msg["pot"].as_i64()).unwrap_or(self.pot),
            winners,
        };
        self.street = None;
        Some(finished)
    }
}

#[cfg(test)]
mod differential;
mod replay;
mod resume;
mod situation;
#[cfg(test)]
mod tests;

pub use self::{replay::replay, resume::OpenHand};

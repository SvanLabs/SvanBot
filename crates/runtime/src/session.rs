//! One bot's session with the venue, without the socket: frames in, messages out.
//!
//! It tracks the table and the hand, builds the [`Spot`] at each turn, asks the decision it is
//! handed, and plays the answer as the nearest action the venue offered. It never sends one the
//! venue left out.
//!
//! **Fair play.** Every frame that lists who sits at the table passes through one seam,
//! [`Session::seat_list`], which is where the fair-play guard lives (V11-01). A stablemate at our
//! table is a shared table: the session leaves at once and raises an incident. A hand dealt with
//! a stablemate in it is a shared hand: the session folds, leaves and raises an incident. A
//! snapshot's seats pass through the same seam. It leaves once: a later seat list with the
//! stablemate still in it sends nothing more, because repeated messages count toward the venue's
//! flood limits. Once its own seat is left, it goes back to the lobby.
//!
//! **Reconnects.** A session outlives its connection. It keeps the table and the highest
//! `table_seq` it applied, so a reconnect asks for the missed events instead of joining the lobby
//! again (`resync`).

mod resync;
mod view;

use crate::lines::Lines;
use crate::status::{Seat, Table};

use std::collections::BTreeMap;

use svanbot_core::card::CardSet;
use svanbot_core::rules::{Action, Chips};
use svanbot_core::spot::Spot;
use svanbot_strategy::policy::{Decision, Trace};

use crate::wire::{Frame, Inbound, Offer, Outbound, Player, Turn, VenueAction};

/// The buy-in the session asks for: the venue's default.
pub const BUY_IN: Chips = 2000;

/// A fair-play incident: raised to the operator and kept, never dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incident {
    /// A stablemate is seated at our table.
    SharedTable {
        table_id: String,
        stablemate: String,
    },
    /// A hand was dealt with a stablemate in it.
    SharedHand { hand_id: String, stablemate: String },
}

/// What the session does in answer to one frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Step {
    pub send: Vec<Outbound>,
    /// Every incident the frame raised, each once.
    pub incidents: Vec<Incident>,
    /// What each decision in this step was made from, with the hand it belongs to. Empty when the
    /// step decided nothing, or when the caller handed back a bare action with no trace.
    pub traced: Vec<(String, Trace)>,
}

impl Step {
    fn send(message: Outbound) -> Self {
        Self {
            send: vec![message],
            incidents: Vec::new(),
            traced: Vec::new(),
        }
    }

    /// Adds `later`'s messages and incidents, leaving out what this step already holds.
    fn then(mut self, later: Self) -> Self {
        for message in later.send {
            if !self.send.contains(&message) {
                self.send.push(message);
            }
        }
        for incident in later.incidents {
            if !self.incidents.contains(&incident) {
                self.incidents.push(incident);
            }
        }
        // A trace is added, never merged: one step decides at most once, and a turn seen twice is
        // refused by `played_token` before it can decide again. A resync skips replayed turns and
        // snapshots (`resync.rs`), so its one decision carries one trace.
        self.traced.extend(later.traced);
        self
    }
}

/// One bot's session.
#[derive(Clone, Debug)]
pub struct Session {
    me: String,
    /// Every name on the roster, lowercased, aliases included.
    roster: Vec<String>,
    /// The table the bot sits at; empty when it sits at none.
    table_id: String,
    /// The highest `table_seq` applied at this table.
    table_seq: u64,
    /// A `resync_request` is out and unanswered.
    resyncing: bool,
    /// Our seat at the table, once the venue names it.
    seat: Option<usize>,
    /// A fair-play `leave_table` is out: the bot is on its way off this table.
    leaving: bool,
    players: BTreeMap<usize, Player>,
    /// Pre-deal stacks from a join or a paid hand, consumed at the next hand start.
    opening: BTreeMap<usize, Player>,
    hand: Option<HandSeen>,
    actions_sent: u64,
    /// The last turn token played: a token is good for one action only.
    played_token: Option<String>,
    /// The last action sent, until the venue acknowledges it.
    unacked: Option<Outbound>,
    /// Every seat's line in the current hand.
    lines: Lines,
}

#[derive(Clone, Debug, Default)]
struct HandSeen {
    hand_id: String,
    hole: CardSet,
    folded: Vec<usize>,
    opening: BTreeMap<usize, Player>,
}

impl Session {
    /// The session of the bot named `me`. The roster lists every name of every bot the owner has
    /// registered, aliases included; `me` may be on it.
    pub fn new(me: &str, roster: impl IntoIterator<Item = String>) -> Self {
        Self {
            me: me.to_owned(),
            roster: roster.into_iter().map(|name| name.to_lowercase()).collect(),
            table_id: String::new(),
            table_seq: 0,
            resyncing: false,
            seat: None,
            leaving: false,
            players: BTreeMap::new(),
            opening: BTreeMap::new(),
            hand: None,
            actions_sent: 0,
            played_token: None,
            unacked: None,
            lines: Lines::default(),
        }
    }

    /// What to send once the venue accepts the connection: a resync of the table the bot sits at,
    /// or else the lobby.
    pub fn on_connect(&mut self) -> Vec<Outbound> {
        if self.table_id.is_empty() {
            return join_lobby();
        }
        self.resyncing = true;
        vec![Outbound::ResyncRequest {
            table_id: self.table_id.clone(),
            last_table_seq: self.table_seq,
        }]
    }

    /// The table as the dashboard sees it: our seat, the hand in progress and every seated
    /// player, or `None` when the bot sits at none. The hole cards stay out of it.
    pub fn table(&self) -> Option<Table> {
        if self.table_id.is_empty() {
            return None;
        }
        let in_hand = self.hand.is_some();
        Some(Table {
            id: self.table_id.clone(),
            seat: self.seat,
            hand_id: self.hand.as_ref().map(|hand| hand.hand_id.clone()),
            street: if in_hand {
                self.lines.street().map(|street| street.name().to_owned())
            } else {
                None
            },
            board: if in_hand {
                self.lines
                    .board_order()
                    .iter()
                    .map(|card| card.to_string())
                    .collect()
            } else {
                Vec::new()
            },
            pot: if in_hand { self.lines.pot() } else { 0 },
            big_blind: if in_hand { self.lines.big_blind() } else { 0 },
            dealer: if in_hand { self.lines.dealer() } else { 0 },
            players: self
                .players
                .values()
                .map(|player| Seat {
                    seat: player.seat,
                    name: player.name.clone(),
                    stack: player.stack,
                    folded: in_hand && self.lines.folded(player.seat),
                })
                .collect(),
        })
    }

    /// Forgets the table: the venue frees a seat 120 s after a disconnect, so after a longer gap
    /// there is no table to resync.
    pub fn forget_table(&mut self) {
        self.table_id.clear();
        self.table_seq = 0;
        self.resyncing = false;
        self.seat = None;
        self.leaving = false;
        self.players.clear();
        self.opening.clear();
        self.hand = None;
    }

    /// Answers one frame. `decide` is asked only at our turn, with the spot as the session sees it
    /// and the seats and names from the same turn or snapshot ([`Session::on_frame`] without them).
    /// A frame whose `table_seq` is not past the watermark was applied already, and is ignored.
    pub fn on_frame_with_players<D: Into<Decision>>(
        &mut self,
        frame: impl Into<Frame>,
        decide: impl FnOnce(&Spot, &[Player]) -> D,
    ) -> Step {
        let Frame { table_seq, inbound } = frame.into();
        if let Inbound::Resync(resync) = inbound {
            // Carries its own watermarks, one per replayed event.
            return self.resync(resync, decide);
        }
        if let Some(seq) = table_seq {
            if seq <= self.table_seq {
                return Step::default();
            }
            self.table_seq = seq;
        }
        match inbound {
            Inbound::YourTurn(turn) => self.your_turn(turn, decide),
            Inbound::TableState(snapshot) => self.install(snapshot, decide),
            event => self.apply(event),
        }
    }

    /// Applies one frame that is not an authority to act.
    fn apply(&mut self, frame: Inbound) -> Step {
        self.lines.on(&frame);
        // Even if hand_start was missed, these frames prove the pending pre-deal stacks
        // are no longer evidence for a future deal. Only the payout can refresh them.
        if matches!(
            frame,
            Inbound::HoleCards { .. }
                | Inbound::PlayerAction { .. }
                | Inbound::CommunityCards { .. }
        ) {
            self.opening.clear();
        }
        match frame {
            Inbound::TableJoined {
                table_id,
                seat,
                players,
            } => {
                self.forget_table();
                self.table_id = table_id;
                self.seat = Some(seat);
                self.opening = players.iter().cloned().map(|p| (p.seat, p)).collect();
                self.seat_list(players)
            }
            Inbound::PlayerJoined(player) => {
                self.opening.insert(player.seat, player.clone());
                self.seat_list(vec![player])
            }
            // Our own seat left, by our leave or the venue's: there is no table to come back to.
            Inbound::PlayerLeft { seat } if self.seat == Some(seat) => self.rejoin(),
            Inbound::PlayerLeft { seat } => {
                self.opening.remove(&seat);
                self.players.remove(&seat);
                Step::default()
            }
            Inbound::HandStart { hand_id, .. } => {
                self.unacked = None;
                self.hand = Some(HandSeen {
                    opening: std::mem::take(&mut self.opening),
                    hand_id,
                    ..HandSeen::default()
                });
                Step::default()
            }
            Inbound::HoleCards { cards } => {
                if let Some(hand) = &mut self.hand {
                    hand.hole = cards.into_iter().collect();
                }
                Step::default()
            }
            Inbound::PlayerAction {
                seat,
                action,
                stack,
                ..
            } => {
                if let Some(player) = self.players.get_mut(&seat) {
                    player.stack = stack;
                }
                if action == "fold"
                    && let Some(hand) = &mut self.hand
                {
                    hand.folded.push(seat);
                }
                Step::default()
            }
            Inbound::HandResult { final_stacks, .. } => {
                self.opening = final_stacks
                    .into_iter()
                    .filter_map(|(seat, stack)| {
                        let mut player = self.players.get(&seat)?.clone();
                        player.stack = stack;
                        Some((seat, player))
                    })
                    .collect();
                self.hand = None;
                Step::default()
            }
            // The docs: a table that closes before our own `player_left` gets one final
            // `leave_table`, and the `not_at_table` that follows is a clean exit.
            Inbound::TableClosed if self.leaving => Step::send(Outbound::LeaveTable),
            Inbound::TableClosed | Inbound::SeasonEnded => self.rejoin(),
            Inbound::ActionAck { client_action_id } => {
                if let Some(Outbound::Action {
                    client_action_id: sent,
                    ..
                }) = &self.unacked
                    && *sent == client_action_id
                {
                    self.unacked = None;
                }
                Step::default()
            }
            // The venue dropped a message. If it was our action, the same id and payload again is a
            // safe retry: the venue answers a duplicate from its cache.
            Inbound::Error { code, .. } if code == "rate_limited" => match &self.unacked {
                Some(action) => Step::send(action.clone()),
                None => Step::default(),
            },
            Inbound::Error { code, table_id } => self.error(&code, table_id),
            _ => Step::default(),
        }
    }

    /// The seam every seat list passes through. Records the seats, and leaves the table if a
    /// stablemate sits at it.
    fn seat_list(&mut self, players: Vec<Player>) -> Step {
        for player in players {
            self.players.insert(player.seat, player);
        }
        if self.leaving {
            return Step::default();
        }
        match self.stablemate() {
            Some(stablemate) => {
                self.leaving = true;
                Step {
                    send: vec![Outbound::LeaveTable],
                    incidents: vec![Incident::SharedTable {
                        table_id: self.table_id.clone(),
                        stablemate,
                    }],
                    traced: Vec::new(),
                }
            }
            None => Step::default(),
        }
    }

    /// Off this table and back to the lobby.
    fn rejoin(&mut self) -> Step {
        self.forget_table();
        Step::send(Outbound::JoinLobby { buy_in: BUY_IN })
    }

    /// A stablemate seated at our table, if there is one.
    fn stablemate(&self) -> Option<String> {
        let me = self.me.to_lowercase();
        self.players
            .values()
            .find(|player| {
                let name = player.name.to_lowercase();
                name != me && self.roster.contains(&name)
            })
            .map(|player| player.name.clone())
    }

    fn your_turn<D: Into<Decision>>(
        &mut self,
        turn: Turn,
        decide: impl FnOnce(&Spot, &[Player]) -> D,
    ) -> Step {
        self.opening.clear();
        if let (Some(id), Some(hand)) = (&turn.hand_id, &mut self.hand)
            && id != &hand.hand_id
        {
            // A start was missed. Later legacy turns must not reuse the old hand's evidence.
            *hand = HandSeen {
                hand_id: id.clone(),
                ..HandSeen::default()
            };
        }
        let left = self.seat_list(turn.players.clone());
        let stablemate = self.stablemate();
        let hand = self.hand.clone().unwrap_or_default();
        let Some(hand_id) = turn
            .hand_id
            .clone()
            .or((!hand.hand_id.is_empty()).then(|| hand.hand_id.clone()))
        else {
            // An action must echo the hand. Without one the venue acts for us at the clock.
            return left;
        };
        if self.played_token.as_ref() == Some(&turn.turn_token) {
            // Played already: the venue rejects a token's second use.
            return left;
        }
        self.played_token = Some(turn.turn_token.clone());
        // In a shared hand the bot folds outright, even when the check is free: it takes no further
        // part in a hand a stablemate is in.
        let decided = match stablemate {
            Some(_) => None,
            None => Some(decide(&self.spot(&turn, &hand), &turn.players).into()),
        };
        let action = match &decided {
            Some(decision) => offered(decision.action, &turn.offer),
            None => VenueAction::Fold,
        };
        // The trace rides out with the action: the session is the only place holding both it and
        // the hand id it belongs to, and the store writes it with the hand (`Store::append_hand`).
        let traced: Vec<(String, Trace)> = decided
            .and_then(|decision| decision.trace.map(|trace| (hand_id.clone(), trace)))
            .into_iter()
            .collect();
        self.actions_sent += 1;
        let action = Outbound::Action {
            hand_id: hand_id.clone(),
            action,
            client_action_id: format!("{hand_id}-{}", self.actions_sent),
            turn_token: turn.turn_token,
        };
        self.unacked = Some(action.clone());
        let mut send = vec![action];
        send.extend(left.send);
        // A hand dealt with a stablemate is a shared hand, whether or not the leave is out already.
        let incidents = stablemate
            .map(|stablemate| Incident::SharedHand {
                hand_id,
                stablemate,
            })
            .into_iter()
            .collect();
        Step {
            send,
            incidents,
            traced,
        }
    }
}

fn join_lobby() -> Vec<Outbound> {
    vec![
        Outbound::JoinLobby { buy_in: BUY_IN },
        Outbound::SetAutoRebuy { enabled: true },
    ]
}

/// The offered action nearest to `decided`. A raise past the range is all in when that is
/// offered, else the top of the range. Anything not offered steps down to a call, then a check,
/// then a fold, and a fold is a check when the check is free.
fn offered(decided: Action, offer: &Offer) -> VenueAction {
    let passive = || match (offer.call, offer.check) {
        (Some(_), _) => VenueAction::Call,
        (None, true) => VenueAction::Check,
        (None, false) => VenueAction::Fold,
    };
    let fold_or_check = if offer.check {
        VenueAction::Check
    } else {
        VenueAction::Fold
    };
    match decided {
        Action::Fold | Action::Check => fold_or_check,
        Action::Call if offer.call.is_some() => VenueAction::Call,
        Action::Call => fold_or_check,
        Action::RaiseTo(to) => match (offer.raise, offer.all_in) {
            (_, Some(all_in)) if to >= all_in => VenueAction::AllIn,
            (Some(range), _) => VenueAction::Raise(to.clamp(range.min, range.max)),
            (None, Some(_)) => VenueAction::AllIn,
            (None, None) => passive(),
        },
    }
}

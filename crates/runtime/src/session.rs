//! One bot's session with the venue, without the socket: frames in, messages out.
//!
//! It tracks the table and the hand, builds the [`Spot`] at each turn, asks the decision it is
//! handed, and plays the answer as the nearest action the venue offered. It never sends one the
//! venue left out.
//!
//! **Fair play.** Every frame that lists who sits at the table passes through one seam,
//! [`Session::seat_list`], which is where the fair-play guard lives (V11-01). A stablemate at our
//! table is a shared table: the session leaves at once and raises an incident. A hand dealt with
//! a stablemate in it is a shared hand: the session folds, leaves and raises an incident.

use std::collections::BTreeMap;

use svanbot_core::card::CardSet;
use svanbot_core::rules::{Action, Legal, RaiseRange};
use svanbot_core::spot::Spot;

use crate::wire::{Inbound, Offer, Outbound, Player, Turn, VenueAction};

/// The buy-in the session asks for: the venue's default.
pub const BUY_IN: u32 = 2000;

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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Step {
    pub send: Vec<Outbound>,
    pub incident: Option<Incident>,
}

/// One bot's session.
#[derive(Clone, Debug)]
pub struct Session {
    me: String,
    /// Every name on the roster, lowercased, aliases included.
    roster: Vec<String>,
    table_id: String,
    players: BTreeMap<usize, Player>,
    hand: Option<HandSeen>,
    actions_sent: u64,
}

#[derive(Clone, Debug, Default)]
struct HandSeen {
    hand_id: String,
    hole: CardSet,
    folded: Vec<usize>,
}

impl Session {
    /// The session of the bot named `me`. The roster lists every name of every bot the owner has
    /// registered, aliases included; `me` may be on it.
    pub fn new(me: &str, roster: impl IntoIterator<Item = String>) -> Self {
        Self {
            me: me.to_owned(),
            roster: roster.into_iter().map(|name| name.to_lowercase()).collect(),
            table_id: String::new(),
            players: BTreeMap::new(),
            hand: None,
            actions_sent: 0,
        }
    }

    /// What to send once the venue accepts the connection.
    pub fn on_connect(&self) -> Vec<Outbound> {
        vec![
            Outbound::JoinLobby { buy_in: BUY_IN },
            Outbound::SetAutoRebuy { enabled: true },
        ]
    }

    /// Answers one frame. `decide` is asked only at our turn, with the spot as the session sees it.
    pub fn on_frame(&mut self, frame: Inbound, decide: impl FnOnce(&Spot) -> Action) -> Step {
        match frame {
            Inbound::TableJoined {
                table_id, players, ..
            } => {
                self.table_id = table_id;
                self.players.clear();
                self.seat_list(players)
            }
            Inbound::PlayerJoined(player) => self.seat_list(vec![player]),
            Inbound::PlayerLeft { seat } => {
                self.players.remove(&seat);
                Step::default()
            }
            Inbound::HandStart { hand_id, .. } => {
                self.hand = Some(HandSeen {
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
            Inbound::YourTurn(turn) => self.your_turn(turn, decide),
            Inbound::HandResult { .. } => {
                self.hand = None;
                Step::default()
            }
            Inbound::TableClosed | Inbound::SeasonEnded => {
                self.players.clear();
                self.hand = None;
                Step {
                    send: vec![Outbound::JoinLobby { buy_in: BUY_IN }],
                    incident: None,
                }
            }
            _ => Step::default(),
        }
    }

    /// The seam every seat list passes through. Records the seats, and leaves the table if a
    /// stablemate sits at it.
    fn seat_list(&mut self, players: Vec<Player>) -> Step {
        for player in players {
            self.players.insert(player.seat, player);
        }
        match self.stablemate() {
            Some(stablemate) => Step {
                send: vec![Outbound::LeaveTable],
                incident: Some(Incident::SharedTable {
                    table_id: self.table_id.clone(),
                    stablemate,
                }),
            },
            None => Step::default(),
        }
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

    fn your_turn(&mut self, turn: Turn, decide: impl FnOnce(&Spot) -> Action) -> Step {
        let shared = self.seat_list(turn.players.clone());
        let hand = self.hand.clone().unwrap_or_default();
        let Some(hand_id) = turn
            .hand_id
            .clone()
            .or((!hand.hand_id.is_empty()).then(|| hand.hand_id.clone()))
        else {
            // An action must echo the hand. Without one the venue acts for us at the clock.
            return shared;
        };
        // In a shared hand the bot folds outright, even when the check is free: it takes no further
        // part in a hand a stablemate is in.
        let action = match &shared.incident {
            Some(_) => VenueAction::Fold,
            None => offered(decide(&self.spot(&turn, &hand)), &turn.offer),
        };
        self.actions_sent += 1;
        let action = Outbound::Action {
            hand_id: hand_id.clone(),
            action,
            client_action_id: format!("{hand_id}-{}", self.actions_sent),
            turn_token: turn.turn_token,
        };
        let mut send = vec![action];
        send.extend(shared.send);
        let incident = shared.incident.map(|incident| match incident {
            Incident::SharedTable { stablemate, .. } => Incident::SharedHand {
                hand_id,
                stablemate,
            },
            hand @ Incident::SharedHand { .. } => hand,
        });
        Step { send, incident }
    }

    fn spot(&self, turn: &Turn, hand: &HandSeen) -> Spot {
        let me = self.me.to_lowercase();
        let mine = turn
            .players
            .iter()
            .find(|player| player.name.to_lowercase() == me);
        let opponents = turn
            .players
            .iter()
            .filter(|player| {
                player.name.to_lowercase() != me && !hand.folded.contains(&player.seat)
            })
            .count();
        let raise = turn.offer.raise.or(turn
            .offer
            .all_in
            .map(|all| RaiseRange { min: all, max: all }));
        Spot {
            hole: hand.hole,
            board: turn.board.iter().copied().collect(),
            pot: turn.pot,
            legal: Legal {
                call: turn.offer.call,
                raise,
            },
            opponents,
            stack: mine.map_or(0, |player| player.stack),
        }
    }
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

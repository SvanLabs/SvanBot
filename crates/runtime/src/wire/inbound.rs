//! Frames from the venue.

use std::fmt;

use serde_json::Value;
use svanbot_core::card::{Card, CardSet};
use svanbot_core::rules::RaiseRange;

use super::snapshot::{Resync, Snapshot, resync, snapshot};

/// A seat as a frame lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Player {
    pub seat: usize,
    pub name: String,
    pub stack: u32,
}

/// The actions a `your_turn` frame offers, exactly as offered: the bot never plays an action
/// the venue left out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Offer {
    pub fold: bool,
    pub check: bool,
    /// The chips a call puts in. A short all-in call comes as a call.
    pub call: Option<u32>,
    /// The raise-to totals allowed, both included.
    pub raise: Option<RaiseRange>,
    /// Offered only when going all in is a raise distinct from the others.
    pub all_in: Option<u32>,
}

/// It is our turn: everything the venue says about the spot, and what an action must echo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    /// From the frame's envelope. An action must echo it.
    pub hand_id: Option<String>,
    pub turn_token: String,
    pub offer: Offer,
    pub pot: u32,
    pub board: Vec<Card>,
    pub players: Vec<Player>,
}

/// One frame with its envelope's `table_seq`: the table-wide watermark that orders events and
/// names the point a resync starts from. Not every frame carries one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub table_seq: Option<u64>,
    pub inbound: Inbound,
}

impl From<Inbound> for Frame {
    fn from(inbound: Inbound) -> Self {
        Self {
            table_seq: None,
            inbound,
        }
    }
}

/// The optional fields of a `player_action`, when the venue sends them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Detail {
    /// `contribution_delta`: the chips the action put in.
    pub added: Option<u32>,
    /// `pot_before`.
    pub pot_before: Option<u32>,
    /// `to_call_before`, 0 when nothing was to call.
    pub to_call: Option<u32>,
}

/// One frame from the venue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inbound {
    Connected {
        name: String,
    },
    /// A protocol or authentication failure. Branch on the code, never on the message.
    /// `already_seated` names the table the bot already sits at.
    Error {
        code: String,
        table_id: Option<String>,
    },
    TableJoined {
        table_id: String,
        seat: usize,
        players: Vec<Player>,
    },
    HandStart {
        hand_id: String,
        seat: usize,
        dealer_seat: usize,
        small_blind: u32,
        big_blind: u32,
    },
    HoleCards {
        cards: Vec<Card>,
    },
    YourTurn(Turn),
    ActionAck {
        client_action_id: String,
    },
    ActionRejected {
        code: String,
    },
    /// Any seat's action. The amount is 0 for a check or a fold.
    PlayerAction {
        seat: usize,
        /// The venue names the seat in the action itself, when it sends one.
        name: Option<String>,
        action: String,
        amount: u32,
        stack: u32,
        detail: Detail,
    },
    CommunityCards {
        cards: Vec<Card>,
    },
    /// Each seat's stack once the pot is paid, lowest seat first, and the pairs shown at the
    /// showdown. A mucked pair is not shown.
    HandResult {
        final_stacks: Vec<(usize, u32)>,
        shown: Vec<(usize, CardSet)>,
    },
    PlayerJoined(Player),
    PlayerLeft {
        seat: usize,
    },
    TableClosed,
    Busted,
    SeasonEnded,
    /// The table as it stands: authoritative for the fields it holds.
    TableState(Snapshot),
    /// The answer to a `resync_request`.
    Resync(Resync),
    /// A frame the bot does not act on, by its type.
    Other {
        kind: String,
    },
}

/// A frame that is not valid JSON, or lacks a field its type needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireError(pub(super) String);

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unreadable frame: {}", self.0)
    }
}

impl std::error::Error for WireError {}

pub(super) type Read<T> = Result<T, WireError>;

/// Reads one frame, without its envelope.
pub fn parse(text: &str) -> Read<Inbound> {
    parse_frame(text).map(|frame| frame.inbound)
}

/// Reads one frame with its envelope's `table_seq`.
pub fn parse_frame(text: &str) -> Read<Frame> {
    let frame: Value = serde_json::from_str(text).map_err(|e| WireError(e.to_string()))?;
    frame_of(&frame)
}

pub(super) fn frame_of(frame: &Value) -> Read<Frame> {
    Ok(Frame {
        table_seq: frame.get("table_seq").and_then(Value::as_u64),
        inbound: inbound(frame)?,
    })
}

fn inbound(frame: &Value) -> Read<Inbound> {
    let kind = text_at(frame, "type")?;
    Ok(match kind.as_str() {
        "connected" => Inbound::Connected {
            name: text_at(frame, "name")?,
        },
        "error" => Inbound::Error {
            code: text_at(frame, "code")?,
            table_id: optional_text(frame, "table_id"),
        },
        "table_joined" => Inbound::TableJoined {
            table_id: text_at(frame, "table_id")?,
            seat: seat_at(frame, "seat")?,
            players: players_at(frame, "players")?,
        },
        "hand_start" => {
            let blinds = frame.get("blinds").unwrap_or(&Value::Null);
            Inbound::HandStart {
                hand_id: text_at(frame, "hand_id")?,
                seat: seat_at(frame, "seat")?,
                dealer_seat: seat_at(frame, "dealer_seat")?,
                small_blind: chips_at(blinds, "small_blind")?,
                big_blind: chips_at(blinds, "big_blind")?,
            }
        }
        "hole_cards" => Inbound::HoleCards {
            cards: cards_at(frame, "cards")?,
        },
        "your_turn" => Inbound::YourTurn(Turn {
            hand_id: optional_text(frame, "hand_id"),
            turn_token: text_at(frame, "turn_token")?,
            offer: offer_at(frame)?,
            pot: chips_at(frame, "pot")?,
            board: cards_at(frame, "community_cards")?,
            players: players_at(frame, "players")?,
        }),
        "action_ack" => Inbound::ActionAck {
            client_action_id: text_at(frame, "client_action_id")?,
        },
        "action_rejected" => Inbound::ActionRejected {
            code: text_at(frame, "code")?,
        },
        "player_action" => Inbound::PlayerAction {
            seat: seat_at(frame, "seat")?,
            name: optional_text(frame, "name"),
            action: text_at(frame, "action")?,
            // `null` for a check or a fold, as the docs' "Null Fields" say.
            amount: match frame.get("amount") {
                None | Some(Value::Null) => 0,
                Some(_) => chips_at(frame, "amount")?,
            },
            stack: chips_at(frame, "stack")?,
            detail: Detail {
                added: optional_chips(frame, "contribution_delta"),
                pot_before: optional_chips(frame, "pot_before"),
                // `null` when nothing was to call.
                to_call: match frame.get("to_call_before") {
                    None => None,
                    Some(Value::Null) => Some(0),
                    Some(_) => optional_chips(frame, "to_call_before"),
                },
            },
        },
        "community_cards" => Inbound::CommunityCards {
            cards: cards_at(frame, "cards")?,
        },
        "hand_result" => Inbound::HandResult {
            final_stacks: final_stacks(frame)?,
            shown: shown_cards(frame)?,
        },
        "player_joined" => Inbound::PlayerJoined(player(frame)?),
        "player_left" => Inbound::PlayerLeft {
            seat: seat_at(frame, "seat")?,
        },
        "table_closed" => Inbound::TableClosed,
        "busted" => Inbound::Busted,
        "season_ended" => Inbound::SeasonEnded,
        "table_state" => Inbound::TableState(snapshot(frame)?),
        "resync_response" => Inbound::Resync(resync(frame)?),
        _ => Inbound::Other { kind },
    })
}

fn optional_chips(frame: &Value, key: &str) -> Option<u32> {
    frame
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
}

/// `shown_cards`, by seat: present only at a showdown, and never with a mucked pair.
fn shown_cards(frame: &Value) -> Read<Vec<(usize, CardSet)>> {
    let Some(shown) = frame.get("shown_cards").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let mut seats = shown
        .keys()
        .map(|seat| {
            let at: usize = seat
                .parse()
                .map_err(|_| WireError(format!("seat {seat:?}")))?;
            Ok((
                at,
                cards_at(&frame["shown_cards"], seat)?.into_iter().collect(),
            ))
        })
        .collect::<Read<Vec<_>>>()?;
    seats.sort_unstable_by_key(|&(seat, _)| seat);
    Ok(seats)
}

pub(super) fn optional_text(frame: &Value, key: &str) -> Option<String> {
    frame.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn field<'a>(frame: &'a Value, key: &str) -> Read<&'a Value> {
    frame
        .get(key)
        .ok_or_else(|| WireError(format!("no `{key}`")))
}

pub(super) fn text_at(frame: &Value, key: &str) -> Read<String> {
    let value = field(frame, key)?;
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| WireError(format!("`{key}` is not text: {value}")))
}

pub(super) fn chips_at(frame: &Value, key: &str) -> Read<u32> {
    let value = field(frame, key)?;
    value
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| WireError(format!("`{key}` is not a chip count: {value}")))
}

pub(super) fn seat_at(frame: &Value, key: &str) -> Read<usize> {
    chips_at(frame, key).map(|n| n as usize)
}

pub(super) fn cards_at(frame: &Value, key: &str) -> Read<Vec<Card>> {
    let value = field(frame, key)?;
    let list = value
        .as_array()
        .ok_or_else(|| WireError(format!("`{key}` is not a list")))?;
    list.iter()
        .map(|card| {
            card.as_str()
                .and_then(|text| text.parse().ok())
                .ok_or_else(|| WireError(format!("`{key}` holds {card}, not a card")))
        })
        .collect()
}

pub(super) fn player(frame: &Value) -> Read<Player> {
    Ok(Player {
        seat: seat_at(frame, "seat")?,
        name: text_at(frame, "name")?,
        stack: chips_at(frame, "stack")?,
    })
}

pub(super) fn players_at(frame: &Value, key: &str) -> Read<Vec<Player>> {
    let value = field(frame, key)?;
    let list = value
        .as_array()
        .ok_or_else(|| WireError(format!("`{key}` is not a list")))?;
    list.iter().map(player).collect()
}

pub(super) fn offer_at(frame: &Value) -> Read<Offer> {
    let value = field(frame, "valid_actions")?;
    let list = value
        .as_array()
        .ok_or_else(|| WireError("`valid_actions` is not a list".into()))?;
    let mut offer = Offer::default();
    for action in list {
        match text_at(action, "action")?.as_str() {
            "fold" => offer.fold = true,
            "check" => offer.check = true,
            "call" => offer.call = Some(chips_at(action, "amount")?),
            "raise" => {
                offer.raise = Some(RaiseRange {
                    min: chips_at(action, "min")?,
                    max: chips_at(action, "max")?,
                });
            }
            "all_in" => offer.all_in = Some(chips_at(action, "amount")?),
            // An action this client does not know is one it never plays.
            _ => {}
        }
    }
    Ok(offer)
}

fn final_stacks(frame: &Value) -> Read<Vec<(usize, u32)>> {
    let value = field(frame, "final_stacks")?;
    let stacks = value
        .as_object()
        .ok_or_else(|| WireError("`final_stacks` is not an object".into()))?;
    let mut seats = stacks
        .iter()
        .map(|(seat, _)| {
            let at: usize = seat
                .parse()
                .map_err(|_| WireError(format!("seat {seat:?}")))?;
            Ok((at, chips_at(value, seat)?))
        })
        .collect::<Read<Vec<_>>>()?;
    seats.sort_unstable();
    Ok(seats)
}

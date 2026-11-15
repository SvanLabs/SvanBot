//! Frames from the venue.

use std::fmt;

use serde_json::Value;
use svanbot_core::card::Card;
use svanbot_core::rules::RaiseRange;

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

/// One frame from the venue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inbound {
    Connected {
        name: String,
    },
    /// A protocol or authentication failure. Branch on the code, never on the message.
    Error {
        code: String,
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
        action: String,
        amount: u32,
        stack: u32,
    },
    CommunityCards {
        cards: Vec<Card>,
    },
    /// Each seat's stack once the pot is paid, lowest seat first.
    HandResult {
        final_stacks: Vec<(usize, u32)>,
    },
    PlayerJoined(Player),
    PlayerLeft {
        seat: usize,
    },
    TableClosed,
    Busted,
    SeasonEnded,
    /// A frame the bot does not act on, by its type.
    Other {
        kind: String,
    },
}

/// A frame that is not valid JSON, or lacks a field its type needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireError(String);

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unreadable frame: {}", self.0)
    }
}

impl std::error::Error for WireError {}

type Read<T> = Result<T, WireError>;

/// Reads one frame.
pub fn parse(text: &str) -> Read<Inbound> {
    let frame: Value = serde_json::from_str(text).map_err(|e| WireError(e.to_string()))?;
    let kind = text_at(&frame, "type")?;
    Ok(match kind.as_str() {
        "connected" => Inbound::Connected {
            name: text_at(&frame, "name")?,
        },
        "error" => Inbound::Error {
            code: text_at(&frame, "code")?,
        },
        "table_joined" => Inbound::TableJoined {
            table_id: text_at(&frame, "table_id")?,
            seat: seat_at(&frame, "seat")?,
            players: players_at(&frame, "players")?,
        },
        "hand_start" => {
            let blinds = frame.get("blinds").unwrap_or(&Value::Null);
            Inbound::HandStart {
                hand_id: text_at(&frame, "hand_id")?,
                seat: seat_at(&frame, "seat")?,
                dealer_seat: seat_at(&frame, "dealer_seat")?,
                small_blind: chips_at(blinds, "small_blind")?,
                big_blind: chips_at(blinds, "big_blind")?,
            }
        }
        "hole_cards" => Inbound::HoleCards {
            cards: cards_at(&frame, "cards")?,
        },
        "your_turn" => Inbound::YourTurn(Turn {
            hand_id: frame
                .get("hand_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
            turn_token: text_at(&frame, "turn_token")?,
            offer: offer_at(&frame)?,
            pot: chips_at(&frame, "pot")?,
            board: cards_at(&frame, "community_cards")?,
            players: players_at(&frame, "players")?,
        }),
        "action_ack" => Inbound::ActionAck {
            client_action_id: text_at(&frame, "client_action_id")?,
        },
        "action_rejected" => Inbound::ActionRejected {
            code: text_at(&frame, "code")?,
        },
        "player_action" => Inbound::PlayerAction {
            seat: seat_at(&frame, "seat")?,
            action: text_at(&frame, "action")?,
            // `null` for a check or a fold, as the docs' "Null Fields" say.
            amount: match frame.get("amount") {
                None | Some(Value::Null) => 0,
                Some(_) => chips_at(&frame, "amount")?,
            },
            stack: chips_at(&frame, "stack")?,
        },
        "community_cards" => Inbound::CommunityCards {
            cards: cards_at(&frame, "cards")?,
        },
        "hand_result" => Inbound::HandResult {
            final_stacks: final_stacks(&frame)?,
        },
        "player_joined" => Inbound::PlayerJoined(player(&frame)?),
        "player_left" => Inbound::PlayerLeft {
            seat: seat_at(&frame, "seat")?,
        },
        "table_closed" => Inbound::TableClosed,
        "busted" => Inbound::Busted,
        "season_ended" => Inbound::SeasonEnded,
        _ => Inbound::Other { kind },
    })
}

fn field<'a>(frame: &'a Value, key: &str) -> Read<&'a Value> {
    frame
        .get(key)
        .ok_or_else(|| WireError(format!("no `{key}`")))
}

fn text_at(frame: &Value, key: &str) -> Read<String> {
    let value = field(frame, key)?;
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| WireError(format!("`{key}` is not text: {value}")))
}

fn chips_at(frame: &Value, key: &str) -> Read<u32> {
    let value = field(frame, key)?;
    value
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| WireError(format!("`{key}` is not a chip count: {value}")))
}

fn seat_at(frame: &Value, key: &str) -> Read<usize> {
    chips_at(frame, key).map(|n| n as usize)
}

fn cards_at(frame: &Value, key: &str) -> Read<Vec<Card>> {
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

fn player(frame: &Value) -> Read<Player> {
    Ok(Player {
        seat: seat_at(frame, "seat")?,
        name: text_at(frame, "name")?,
        stack: chips_at(frame, "stack")?,
    })
}

fn players_at(frame: &Value, key: &str) -> Read<Vec<Player>> {
    let value = field(frame, key)?;
    let list = value
        .as_array()
        .ok_or_else(|| WireError(format!("`{key}` is not a list")))?;
    list.iter().map(player).collect()
}

fn offer_at(frame: &Value) -> Read<Offer> {
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

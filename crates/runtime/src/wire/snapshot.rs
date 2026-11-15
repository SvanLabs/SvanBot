//! `table_state` and `resync_response`: the table as it stands, and the events missed while away
//! (<https://docs.openpoker.ai/llms-full.txt>, rev 2026-09-02, "table_state", "resync_response").

use serde_json::Value;
use svanbot_core::card::Card;

use super::inbound::{
    Frame, Offer, Player, Read, WireError, cards_at, chips_at, frame_of, offer_at, optional_text,
    player, seat_at,
};

/// An occupied seat in a snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seat {
    pub player: Player,
    /// Whether this seat folded in the current hand. `None` for a seat out of the hand, or from an
    /// older snapshot that leaves it out: the hand's own fold marker then stands.
    pub folded: Option<bool>,
}

/// What a snapshot tells the bot about its own seat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hero {
    pub seat: usize,
    pub hole: Vec<Card>,
    /// The actions offered, present only while the bot is the actor.
    pub offer: Option<Offer>,
    /// Present only in the actor's resync snapshot. Without it a snapshot is no authority to act.
    pub turn_token: Option<String>,
}

/// The table as it stands: authoritative for the fields it holds, and not a log of actions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// From the envelope; an action taken on this snapshot must echo it.
    pub hand_id: Option<String>,
    pub pot: u32,
    pub board: Vec<Card>,
    /// The occupied seats only: an empty seat or one without a name is left out.
    pub seats: Vec<Seat>,
    /// `None` for a spectator.
    pub hero: Option<Hero>,
}

/// The answer to a `resync_request`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resync {
    /// Whether the venue still counts the bot as a player at the table.
    pub player: bool,
    pub to_table_seq: Option<u64>,
    /// The missed events, lowest `table_seq` first. Private messages are never replayed.
    pub replayed: Vec<Frame>,
    pub snapshot: Option<Snapshot>,
}

pub(super) fn snapshot(frame: &Value) -> Read<Snapshot> {
    let mut seats = Vec::new();
    for seat in list_at(frame, "seats")? {
        let named = seat.get("name").is_some_and(Value::is_string);
        let empty = seat.get("status").and_then(Value::as_str) == Some("empty");
        if named && !empty {
            seats.push(Seat {
                player: player(seat)?,
                folded: seat.get("folded").and_then(Value::as_bool),
            });
        }
    }
    let hero = match frame.get("hero") {
        None | Some(Value::Null) => None,
        Some(hero) => Some(Hero {
            seat: seat_at(hero, "seat")?,
            hole: match hero.get("hole_cards") {
                None | Some(Value::Null) => Vec::new(),
                Some(_) => cards_at(hero, "hole_cards")?,
            },
            offer: match hero.get("valid_actions") {
                None | Some(Value::Null) => None,
                Some(_) => Some(offer_at(hero)?),
            },
            turn_token: optional_text(hero, "turn_token"),
        }),
    };
    Ok(Snapshot {
        hand_id: optional_text(frame, "hand_id"),
        pot: chips_at(frame, "pot")?,
        board: cards_at(frame, "board")?,
        seats,
        hero,
    })
}

pub(super) fn resync(frame: &Value) -> Read<Resync> {
    let mut replayed = list_at(frame, "replayed_events")?
        .iter()
        .map(frame_of)
        .collect::<Read<Vec<_>>>()?;
    // Stable, so events without a watermark keep their order.
    replayed.sort_by_key(|event| event.table_seq);
    Ok(Resync {
        player: frame.get("role").and_then(Value::as_str) == Some("player"),
        to_table_seq: frame.get("to_table_seq").and_then(Value::as_u64),
        replayed,
        snapshot: match frame.get("snapshot") {
            None | Some(Value::Null) => None,
            Some(table) => Some(snapshot(table)?),
        },
    })
}

fn list_at<'a>(frame: &'a Value, key: &str) -> Read<&'a Vec<Value>> {
    frame
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| WireError(format!("`{key}` is not a list")))
}

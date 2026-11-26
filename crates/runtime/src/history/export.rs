//! Decodes an Open Poker analytics hand into the observations the action model uses.
//! The original export stays in the store. Unnamed seats teach the population model only.

use serde_json::Value;
use svanbot_core::card::{Card, CardSet};
use svanbot_core::rules::Street;

use crate::lines::Lines;
use crate::wire::{Detail, Inbound};

pub(super) fn frames(text: &str) -> Option<Vec<Inbound>> {
    let hand: Value = serde_json::from_str(text).ok()?;
    let hero = seat(&hand["seat"])?;
    let dealer = seat(&hand["dealer_seat"])?;
    let small_blind = hand["small_blind"].as_u64()?;
    let big_blind = hand["big_blind"].as_u64()?;
    small_blind.checked_add(big_blind)?;
    let board = cards(&hand["board"])?;
    let hole = cards(&hand["hole_cards"])?;
    if hole.len() != 2 || !matches!(board.len(), 0 | 3..=5) {
        return None;
    }
    let mut actions: Vec<_> = hand["all_actions"]
        .as_array()?
        .iter()
        .map(|action| Some((action["sequence_in_hand"].as_u64()?, action)))
        .collect::<Option<_>>()?;
    actions.sort_unstable_by_key(|(sequence, _)| *sequence);
    if actions.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return None;
    }
    let mut dealt: Vec<_> = actions
        .iter()
        .filter_map(|(_, action)| seat(&action["seat"]))
        .chain([dealer, hero])
        .collect();
    dealt.sort_unstable();
    dealt.dedup();
    // Do not assign a blind to the wrong seat if an all-in or absent player never acted.
    if hand["num_players"].as_u64() != Some(dealt.len() as u64) {
        dealt.clear();
    }
    let mut frames = vec![
        Inbound::HandStart {
            hand_id: hand["hand_id"].as_str().unwrap_or_default().to_owned(),
            seat: hero,
            dealer_seat: dealer,
            small_blind,
            big_blind,
        },
        Inbound::HoleCards { cards: hole },
    ];
    let mut lines = Lines::default();
    lines.on(&frames[0]);
    let mut street = Street::Preflop;
    let mut highest = big_blind;
    let mut dealt_cards = 0;
    for (_, action) in actions {
        let who = seat(&action["seat"])?;
        let at = match action["street"].as_str()? {
            // A terminating action may be recorded once the venue has moved to showdown.
            "showdown" => street,
            name => Street::named(name)?,
        };
        if at < street {
            return None;
        }
        if at > street {
            let through = board_len(at);
            let frame = Inbound::CommunityCards {
                cards: board.get(dealt_cards..through)?.to_vec(),
            };
            lines.on(&frame);
            frames.push(frame);
            dealt_cards = through;
            highest = 0;
            street = at;
        }
        let kind = action["action"].as_str()?;
        if street == Street::Preflop && dealt.is_empty() {
            // Keep the action classes where possible, but do not treat an unknown blind layout
            // as a precise chip contribution.
            if matches!(kind, "raise" | "all_in" | "bet") {
                break;
            }
        }
        let committed = lines.committed(who, &dealt);
        let to_call = highest.saturating_sub(committed);
        let amount = action["amount"].as_u64().unwrap_or(0);
        let added = match kind {
            "fold" | "check" => 0,
            "call" => to_call,
            "raise" | "bet" | "all_in" if amount > committed => amount - committed,
            // When the export lacks an all-in amount, later pot and bet sizes are unknown.
            "raise" | "bet" | "all_in" => break,
            _ => return None,
        };
        lines.pot().checked_add(added)?;
        highest = highest.max(committed.checked_add(added)?);
        let frame = Inbound::PlayerAction {
            seat: who,
            name: None,
            action: kind.to_owned(),
            amount,
            stack: 0,
            detail: Detail {
                added: Some(added),
                pot_before: Some(lines.pot()),
                to_call: Some(to_call),
            },
        };
        lines.on(&frame);
        frames.push(frame);
    }
    if dealt_cards < board.len() {
        frames.push(Inbound::CommunityCards {
            cards: board[dealt_cards..].to_vec(),
        });
    }
    if let Some(shown) = hand["shown_cards"].as_object() {
        frames.push(Inbound::HandResult {
            final_stacks: Vec::new(),
            shown: shown
                .iter()
                .filter_map(|(who, pair)| {
                    let pair: CardSet = cards(pair)?.into_iter().collect();
                    (pair.len() == 2).then_some((who.parse().ok()?, pair))
                })
                .collect(),
        });
    }
    Some(frames)
}

fn seat(value: &Value) -> Option<usize> {
    value
        .as_u64()
        .filter(|&seat| seat < 6)
        .map(|seat| seat as usize)
}

fn cards(value: &Value) -> Option<Vec<Card>> {
    value
        .as_array()?
        .iter()
        .map(|card| card.as_str()?.parse().ok())
        .collect()
}

const fn board_len(street: Street) -> usize {
    match street {
        Street::Preflop => 0,
        Street::Flop => 3,
        Street::Turn => 4,
        Street::River => 5,
    }
}

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
    let hero_cards: CardSet = hole.iter().copied().collect();
    let mut known: CardSet = board.iter().chain(&hole).copied().collect();
    if known.len() != board.len() + hole.len() {
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
    if hand["num_players"].as_u64() != Some(dealt.len() as u64) || dealt.len() < 2 {
        return None;
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
    let mut active = [false; 6];
    for &seat in &dealt {
        active[seat] = true;
    }
    let mut acted = [false; 6];
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
        // The closing move can already carry the next street's label. Finish the current
        // betting round before revealing any new board cards to the fit.
        let betting_complete = dealt.iter().all(|&seat| {
            !active[seat] || (acted[seat] && lines.committed(seat, &dealt) == highest)
        });
        if at > street && betting_complete {
            let through = board_len(at);
            let frame = Inbound::CommunityCards {
                cards: board.get(dealt_cards..through)?.to_vec(),
            };
            lines.on(&frame);
            frames.push(frame);
            dealt_cards = through;
            highest = 0;
            street = at;
            acted.fill(false);
        }
        let kind = action["action"].as_str()?;
        let committed = lines.committed(who, &dealt);
        let to_call = highest.saturating_sub(committed);
        let amount = action["amount"].as_u64().unwrap_or(0);
        let added = match kind {
            "fold" | "check" => 0,
            "call" => to_call,
            "raise" | "bet" | "all_in" if amount > committed => amount - committed,
            // When the export lacks an all-in amount, later pot and bet sizes are unknown.
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
        acted[who] = true;
        if matches!(kind, "fold" | "all_in") {
            active[who] = false;
        }
    }
    let reported = hand["pot_total"].as_u64()?;
    let mut commitments: Vec<_> = dealt
        .iter()
        .map(|&seat| lines.committed(seat, &dealt))
        .collect();
    commitments.sort_unstable_by(|a, b| b.cmp(a));
    let refund = commitments[0] - commitments[1];
    if reported != lines.pot() && reported != lines.pot().checked_sub(refund)? {
        return None;
    }
    if dealt_cards < board.len() {
        frames.push(Inbound::CommunityCards {
            cards: board[dealt_cards..].to_vec(),
        });
    }
    if let Some(shown) = hand["shown_cards"].as_object() {
        let mut pairs = Vec::with_capacity(shown.len());
        for (who, pair) in shown {
            let who = who
                .parse::<usize>()
                .ok()
                .filter(|seat| dealt.contains(seat))?;
            let pair: CardSet = cards(pair)?.into_iter().collect();
            if who == hero {
                if pair != hero_cards {
                    return None;
                }
            } else {
                if pair.len() != 2 || (known | pair).len() != known.len() + 2 {
                    return None;
                }
                known |= pair;
            }
            pairs.push((who, pair));
        }
        frames.push(Inbound::HandResult {
            final_stacks: Vec::new(),
            shown: pairs,
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

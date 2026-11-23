//! Stored hands made ready for the fit: each opponent's side of a hand as a
//! [`learn::fit::Observed`](svanbot_learn::fit::Observed).
//!
//! An opponent who showed gives their pair. One who reached the showdown and did not show mucked,
//! so their pair lost to the best hand shown there. Anyone else's pair is unseen. Our own seat is
//! never an opponent.

use std::collections::BTreeMap;

use svanbot_core::card::CardSet;
use svanbot_core::eval::strength;
use svanbot_core::spot::ActKind;
use svanbot_learn::fit::{Observed, Outcome};
use svanbot_learn::reads::{Reads, Witness};
use svanbot_strategy::action::Seen;

use crate::lines::Lines;
use crate::record::Hand;
use crate::wire::{Inbound, parse};

/// The opponents' sides of `hand`, numbered `number` so the gate can count them as one hand.
/// Frames that do not parse are skipped.
pub fn observed(hand: &Hand, number: u64) -> Vec<Observed> {
    let mut lines = Lines::default();
    let mut me = None;
    let mut hole = CardSet::EMPTY;
    let mut shown: Vec<(usize, CardSet)> = Vec::new();
    for text in &hand.frames {
        let Ok(frame) = parse(text) else {
            continue;
        };
        match &frame {
            Inbound::HandStart { seat, .. } => me = Some(*seat),
            Inbound::HoleCards { cards } => hole = cards.iter().copied().collect(),
            Inbound::HandResult { shown: at, .. } => shown.clone_from(at),
            _ => {}
        }
        lines.on(&frame);
    }
    let board = lines.board();
    let showdown = board.len() == 5 && !shown.is_empty();
    // The best hand at the showdown: the best shown, or ours if we were in it.
    let mut best = shown.iter().map(|&(_, pair)| strength(pair | board)).max();
    if showdown && me.is_some_and(|seat| !lines.folded(seat)) && hole.len() == 2 {
        best = best.max(Some(strength(hole | board)));
    }
    lines
        .seats()
        .filter(|&seat| Some(seat) != me)
        .map(|seat| {
            let own = shown
                .iter()
                .find(|&&(at, _)| at == seat)
                .map(|&(_, pair)| pair);
            let outcome = match (own, showdown && !lines.folded(seat), best) {
                (Some(pair), _, _) => Outcome::Shown(pair),
                (None, true, Some(winner)) => Outcome::Mucked { board, winner },
                _ => Outcome::Unseen,
            };
            let others_shown = shown
                .iter()
                .filter(|&&(at, _)| at != seat)
                .fold(CardSet::EMPTY, |cards, &(_, pair)| cards | pair);
            Observed {
                hand: number,
                seen: lines.of(seat).iter().map(Seen::of).collect(),
                dead: hole | board | others_shown,
                outcome,
            }
        })
        .collect()
}

/// The bot's reads from its own stored `hands`, oldest first (`docs/adr/0004-reads-are-per-bot.md`).
pub fn reads(hands: &[Hand], me: &str) -> Reads {
    let mut reads = Reads::default();
    for hand in hands {
        for witness in witnesses(hand, me) {
            reads.observe(witness);
        }
    }
    reads
}

/// The opponents `hand` names, with what each did and how each answered bets. Our own seat is
/// never an opponent, and a seat no frame names is left out: the venue name is the opponent's
/// identity. A hand that never names a seat witnesses nobody, which is what downloaded history
/// without per-seat frames gives.
pub fn witnesses(hand: &Hand, me: &str) -> Vec<Witness> {
    let mut lines = Lines::default();
    let mut names: BTreeMap<usize, String> = BTreeMap::new();
    let mut mine = None;
    for text in &hand.frames {
        let Ok(frame) = parse(text) else {
            continue;
        };
        match &frame {
            Inbound::HandStart { seat, .. } => mine = Some(*seat),
            Inbound::TableJoined { players, .. } => {
                for player in players {
                    names.insert(player.seat, player.name.clone());
                }
            }
            Inbound::YourTurn(turn) => {
                for player in &turn.players {
                    names.insert(player.seat, player.name.clone());
                }
            }
            Inbound::PlayerJoined(player) => {
                names.insert(player.seat, player.name.clone());
            }
            Inbound::PlayerAction {
                seat,
                name: Some(name),
                ..
            } => {
                names.insert(*seat, name.clone());
            }
            Inbound::TableState(snapshot) => {
                for seat in &snapshot.seats {
                    names.insert(seat.player.seat, seat.player.name.clone());
                }
            }
            Inbound::Resync(resync) => {
                if let Some(snapshot) = &resync.snapshot {
                    for seat in &snapshot.seats {
                        names.insert(seat.player.seat, seat.player.name.clone());
                    }
                }
                for replayed in &resync.replayed {
                    if let Inbound::PlayerAction {
                        seat,
                        name: Some(name),
                        ..
                    } = &replayed.inbound
                    {
                        names.insert(*seat, name.clone());
                    }
                }
            }
            _ => {}
        }
        lines.on(&frame);
    }
    // The hand's own seat is the authority; the name is the fallback for a hand that starts
    // without one. With neither, we cannot tell which seat is ours, so no seat is an opponent.
    let mine = mine.or_else(|| {
        names
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(me))
            .map(|(&seat, _)| seat)
    });
    let Some(mine) = mine else {
        return Vec::new();
    };
    let mut seen: BTreeMap<usize, Witness> = names
        .into_iter()
        .filter(|(seat, _)| *seat != mine)
        .map(|(seat, name)| {
            (
                seat,
                Witness {
                    name,
                    actions: lines.of(seat).iter().map(Seen::of).collect(),
                    aggressive: false,
                    vs_us: None,
                    vs_others: None,
                },
            )
        })
        .collect();
    // The order the table acted in says who a fold answered: a bet sets the bettor, and the
    // next fold or call faces that seat. A street's first bet has no earlier bettor; a fold to
    // the blinds is answered by the big blind, which is someone else.
    let mut bettor = None;
    let mut street = None;
    for (seat, act) in lines.events() {
        if Some(act.street) != street {
            street = Some(act.street);
            bettor = None;
        }
        if let Some(witness) = seen.get_mut(&seat) {
            match act.kind {
                ActKind::Bet { .. } => witness.aggressive = true,
                ActKind::Fold | ActKind::Call if act.facing => {
                    let folded = matches!(act.kind, ActKind::Fold);
                    if bettor == Some(mine) {
                        witness.vs_us.get_or_insert(folded);
                    } else {
                        witness.vs_others.get_or_insert(folded);
                    }
                }
                _ => {}
            }
        }
        if matches!(act.kind, ActKind::Bet { .. }) {
            bettor = Some(seat);
        }
    }
    seen.into_values().collect()
}

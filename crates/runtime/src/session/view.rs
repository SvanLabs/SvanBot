//! What a live bot decides from: the table as this session sees it, read out as the pure
//! `Spot` that the policy takes. Nothing here writes, and nothing here reads a clock.

use svanbot_core::rules::{Legal, RaiseRange};
use svanbot_core::spot::Spot;

use super::{HandSeen, Session};
use crate::wire::Turn;

impl Session {
    pub(super) fn spot(&self, turn: &Turn, hand: &HandSeen) -> Spot {
        let me = self.me.to_lowercase();
        let mine = turn
            .players
            .iter()
            .find(|player| player.name.to_lowercase() == me);
        let lines: Vec<_> = turn
            .players
            .iter()
            .filter(|player| {
                player.name.to_lowercase() != me && !hand.folded.contains(&player.seat)
            })
            .map(|player| (player.seat, self.lines.of(player.seat).to_vec()))
            .collect();
        let ours = self.seat.map_or(0, |seat| self.lines.turn_order(seat));
        let in_position = lines
            .iter()
            .all(|(seat, _)| self.lines.turn_order(*seat) < ours);
        // The lines keep their seats: a trace names an opponent by seat, and this is the one place
        // that knows it. Both lists are built from the same iteration, so they stay in step.
        let seats: Vec<usize> = lines.iter().map(|(seat, _)| *seat).collect();
        let lines: Vec<_> = lines.into_iter().map(|(_, line)| line).collect();
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
            opponents: lines.len(),
            stack: mine.map_or(0, |player| player.stack),
            committed: self.seat.map_or(0, |seat| {
                let dealt: Vec<usize> = turn.players.iter().map(|player| player.seat).collect();
                self.lines.committed(seat, &dealt)
            }),
            big_blind: self.lines.big_blind(),
            in_position,
            lines,
            seats,
        }
    }
}

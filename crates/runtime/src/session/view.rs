//! What a live bot decides from: the table as this session sees it, read out as the pure
//! `Spot` that the policy takes. Nothing here writes, and nothing here reads a clock.

use svanbot_core::rules::{Chips, Legal, RaiseRange, Street};
use svanbot_core::spot::Spot;

use super::{HandSeen, Session};
use crate::wire::{Player, Turn};

impl HandSeen {
    fn callable_pot(&self, turn: &Turn, hero: &Player) -> Option<Chips> {
        if turn.hand_id.as_ref().is_some_and(|id| id != &self.hand_id) {
            return None;
        }
        let contributed = |player: &Player| {
            let opening = self.opening.get(&player.seat)?;
            (opening.name == player.name).then_some(())?;
            opening.stack.checked_sub(player.stack)
        };
        let cap = contributed(hero)?.checked_add(turn.offer.call?)?;
        let totals: Vec<_> = turn
            .players
            .iter()
            .map(contributed)
            .collect::<Option<_>>()?;
        // Even fresh opening stacks cannot account for dead money from a missing seat.
        (totals
            .iter()
            .try_fold(0u64, |sum, chips| sum.checked_add(*chips))?
            == turn.pot)
            .then(|| svanbot_core::spot::callable_pot(&totals, cap))
    }
}

impl Session {
    /// [`Session::on_frame_with_players`] for a decision that needs only the spot.
    pub fn on_frame<D: Into<svanbot_strategy::policy::Decision>>(
        &mut self,
        frame: impl Into<crate::wire::Frame>,
        decide: impl FnOnce(&Spot) -> D,
    ) -> super::Step {
        self.on_frame_with_players(frame, |spot, _| decide(spot))
    }

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
        let board = turn.board.iter().copied().collect();
        let dealt: Vec<usize> = turn.players.iter().map(|player| player.seat).collect();
        let players_behind = self.seat.and_then(|hero| {
            if !hand.position_known || self.lines.big_blind() == 0 || !dealt.contains(&hero) {
                return None;
            }
            let last = if Street::of(board) == Street::Preflop {
                self.lines.blind_seats(&dealt)?.1
            } else {
                self.lines.dealer()
            };
            let order = |seat: usize| (seat % 6 + 5 - last % 6) % 6;
            Some(
                turn.players
                    .iter()
                    .filter(|player| {
                        player.stack > 0
                            && seats.contains(&player.seat)
                            && order(player.seat) > order(hero)
                    })
                    .count(),
            )
        });
        Spot {
            hole: hand.hole,
            board,
            pot: turn.pot,
            callable_pot: mine.and_then(|hero| hand.callable_pot(turn, hero)),
            legal: Legal {
                call: turn.offer.call,
                raise,
            },
            opponents: lines.len(),
            stack: mine.map_or(0, |player| player.stack),
            committed: self
                .seat
                .map_or(0, |seat| self.lines.committed(seat, &dealt)),
            big_blind: self.lines.big_blind(),
            in_position,
            players_behind,
            lines,
            seats,
        }
    }
}

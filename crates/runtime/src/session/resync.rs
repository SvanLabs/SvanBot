//! Coming back to a table: the errors that name or lose it, the `resync_response`, and installing
//! a snapshot (<https://docs.openpoker.ai/llms-full.txt>, rev 2026-09-02, "Reconnection").

use svanbot_core::spot::Spot;
use svanbot_strategy::policy::Decision;

use super::{HandSeen, Session, Step, join_lobby};
use crate::wire::{Frame, Hero, Inbound, Outbound, Player, Resync, Snapshot, Turn};

impl Session {
    /// An error that changes where the bot sits. `already_seated` names the table to resync, from
    /// its start. Any other error in answer to a resync means the seat is gone: the venue frees it
    /// 120 s after a disconnect.
    pub(super) fn error(&mut self, code: &str, table_id: Option<String>) -> Step {
        match (code, table_id) {
            ("already_seated", Some(table_id)) => {
                self.forget_table();
                self.table_id.clone_from(&table_id);
                self.resyncing = true;
                Step::send(Outbound::ResyncRequest {
                    table_id,
                    last_table_seq: 0,
                })
            }
            _ if self.resyncing => self.back_to_the_lobby(),
            // After a fair-play leave this is the clean exit the docs describe.
            ("not_at_table", _) if self.leaving => self.rejoin(),
            _ => Step::default(),
        }
    }

    /// Applies the replayed events in `table_seq` order, then installs the snapshot over them.
    pub(super) fn resync<D: Into<Decision>>(
        &mut self,
        resync: Resync,
        decide: impl FnOnce(&Spot, &[Player]) -> D,
    ) -> Step {
        self.resyncing = false;
        if !resync.player {
            return self.back_to_the_lobby();
        }
        let mut step = Step::default();
        for Frame { table_seq, inbound } in resync.replayed {
            if let Some(seq) = table_seq {
                if seq <= self.table_seq {
                    continue;
                }
                self.table_seq = seq;
            }
            // Turns and snapshots are never replayed; one that were would be stale.
            if !matches!(
                inbound,
                Inbound::YourTurn(_) | Inbound::TableState(_) | Inbound::Resync(_)
            ) {
                step = step.then(self.apply(inbound));
            }
        }
        if let Some(to) = resync.to_table_seq {
            self.table_seq = self.table_seq.max(to);
        }
        match resync.snapshot {
            Some(snapshot) => step.then(self.install(snapshot, decide)),
            None => step,
        }
    }

    /// Installs a snapshot as the table's state. Its seats pass through the seat-list seam. It is
    /// authority to act only when it carries the hero's offer and turn token.
    pub(super) fn install<D: Into<Decision>>(
        &mut self,
        snapshot: Snapshot,
        decide: impl FnOnce(&Spot, &[Player]) -> D,
    ) -> Step {
        if snapshot.hand_id.is_some() || snapshot.pot != 0 {
            // Current stacks in a live hand are not opening stacks for the next one.
            self.opening.clear();
        }
        match &snapshot.hand_id {
            None => self.hand = None,
            Some(id) => {
                if self.hand.as_ref().is_none_or(|hand| &hand.hand_id != id) {
                    self.hand = Some(HandSeen {
                        hand_id: id.clone(),
                        ..HandSeen::default()
                    });
                }
            }
        }
        if let Some(hand) = &mut self.hand {
            if let Some(hero) = &snapshot.hero
                && !hero.hole.is_empty()
            {
                hand.hole = hero.hole.iter().copied().collect();
            }
            // An explicit fold marker wins; a missing one keeps what the hand already knows.
            for seat in &snapshot.seats {
                let at = seat.player.seat;
                match seat.folded {
                    Some(true) if !hand.folded.contains(&at) => hand.folded.push(at),
                    Some(false) => hand.folded.retain(|&folded| folded != at),
                    _ => {}
                }
            }
        }
        if let Some(hero) = &snapshot.hero {
            self.seat = Some(hero.seat);
        }
        self.players.clear();
        let players: Vec<Player> = snapshot.seats.into_iter().map(|seat| seat.player).collect();
        match snapshot.hero {
            Some(Hero {
                offer: Some(offer),
                turn_token: Some(turn_token),
                ..
            }) => self.your_turn(
                Turn {
                    hand_id: snapshot.hand_id,
                    turn_token,
                    offer,
                    pot: snapshot.pot,
                    board: snapshot.board,
                    players,
                },
                decide,
            ),
            _ => self.seat_list(players),
        }
    }

    fn back_to_the_lobby(&mut self) -> Step {
        self.forget_table();
        Step {
            send: join_lobby(),
            incidents: Vec::new(),
            traced: Vec::new(),
        }
    }
}

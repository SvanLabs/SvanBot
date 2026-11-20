//! The recorder: the venue's frames, as received, gathered into one [`Hand`] per deal.
//!
//! A hand is every frame from its `hand_start` to its `hand_result`, kept as the venue's own JSON
//! text, so nothing is lost in translation and the learner can read it again with any later parser.

use svanbot_core::card::CardSet;
use svanbot_core::rules::Street;
use svanbot_strategy::policy::Trace;

use crate::wire::{Frame, Inbound};

/// One hand as the bot saw it.
#[derive(Clone, Debug, PartialEq)]
pub struct Hand {
    pub hand_id: String,
    pub table_id: String,
    /// The venue's frames, as received, oldest first.
    pub frames: Vec<String>,
    /// What made each of our decisions in it, oldest first. Empty when the caller traced nothing:
    /// a hand played before the trace existed is not a hand that made no decisions.
    pub traced: Vec<Trace>,
}

impl Hand {
    /// The decisions that disagree with the frames about the cards they saw, as
    /// `(index of the decision, which of "hole"/"board")`.
    ///
    /// The trace keeps its own copy of the cards so the replayer needs no frame parser — a later
    /// change to how frames are read must not change what a past decision says it held. The frames
    /// keep the venue's own JSON. Two copies can drift, and a drift is invisible: the trace comes
    /// from the decision path, the frames from the wire, and nothing else compares them. This is
    /// that comparison, for the caller that asserts on a live hand.
    ///
    /// A hand whose frames name no cards has nothing to compare, and compares equal: a missing
    /// card is not a disagreement about one.
    pub fn cards_disagree(&self) -> Vec<(usize, &'static str)> {
        let mut seen = Seen::default();
        let mut named = false;
        let mut at = 0;
        let mut wrong = Vec::new();
        // Advance the trace past every decision whose street the cards now reach. A decision is
        // compared at the moment its street's cards have arrived, not when the whole hand has
        // been read: a reconnect replays several streets at once (`already_seated` asks from the
        // start, `last_table_seq: 0`), and comparing only at the end would jump the board to its
        // final street and call every earlier decision unreached.
        let advance = |seen: &Seen, at: &mut usize, wrong: &mut Vec<(usize, &'static str)>| {
            while let Some(trace) = self.traced.get(*at) {
                if trace.street != Street::of(seen.board) {
                    break;
                }
                // A decision always has the bot's two hole cards, so an empty hole means they have
                // not arrived yet — a replay can list the board before `hole_cards`. Comparing the
                // decision now would blame its real hole against an empty one; wait for them.
                if trace.hole != CardSet::EMPTY && seen.hole == CardSet::EMPTY {
                    break;
                }
                *at += 1;
                if trace.hole != seen.hole {
                    wrong.push((*at - 1, "hole"));
                }
                if trace.board != seen.board {
                    wrong.push((*at - 1, "board"));
                }
            }
        };
        for frame in &self.frames {
            let Ok(frame) = crate::wire::parse(frame) else {
                continue;
            };
            // A reconnect is not a bug, and the venue sends the missed cards inside
            // `resync_response` rather than as top-level frames: the replayed events, then the
            // snapshot, which is authoritative for the hand it names. Reading only top-level
            // frames would call every reconnect a disagreement. Events for another hand, or a
            // spectator's snapshot with no hero, are not this hand's cards and are left out.
            match &frame {
                Inbound::HoleCards { cards } => {
                    named = true;
                    seen.hole = cards.iter().copied().collect();
                    advance(&seen, &mut at, &mut wrong);
                }
                Inbound::CommunityCards { cards } => {
                    named = true;
                    seen.board |= cards.iter().copied().collect::<CardSet>();
                    advance(&seen, &mut at, &mut wrong);
                }
                Inbound::Resync(resync) => {
                    for event in &resync.replayed {
                        match &event.inbound {
                            Inbound::HoleCards { cards } => {
                                named = true;
                                seen.hole = cards.iter().copied().collect();
                            }
                            Inbound::CommunityCards { cards } => {
                                named = true;
                                seen.board |= cards.iter().copied().collect::<CardSet>();
                            }
                            _ => continue,
                        }
                        advance(&seen, &mut at, &mut wrong);
                    }
                    let ours = resync.snapshot.as_ref().filter(|snapshot| {
                        snapshot
                            .hand_id
                            .as_ref()
                            .is_none_or(|id| *id == self.hand_id)
                    });
                    if let Some(snapshot) = ours {
                        if !snapshot.board.is_empty() {
                            named = true;
                            seen.board |= snapshot.board.iter().copied().collect::<CardSet>();
                        }
                        if let Some(hero) = &snapshot.hero
                            && !hero.hole.is_empty()
                        {
                            named = true;
                            seen.hole = hero.hole.iter().copied().collect();
                        }
                        advance(&seen, &mut at, &mut wrong);
                    }
                }
                _ => {}
            }
        }
        // A decision on a street whose card frame never came is one the hand made with no frame
        // to agree with, which is a disagreement, not a decision to skip. A hand whose frames
        // name no cards at all never had anything to agree with, so it is left alone: a missing
        // card is not a disagreement about one.
        if named {
            for index in at..self.traced.len() {
                wrong.push((index, "unreached"));
            }
        }
        wrong
    }
}

/// The cards a hand's frames have named so far: the hole from `hole_cards`, and the board as each
/// `community_cards` frame extends it.
#[derive(Clone, Copy, Debug, Default)]
struct Seen {
    hole: CardSet,
    board: CardSet,
}

/// Gathers frames into hands. It outlives a connection, like the session: a hand that a resync
/// finishes is still the same hand.
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    table_id: String,
    open: Option<Hand>,
    /// The traces the session reported since the last frame, waiting for their hand.
    pending: Vec<(String, Trace)>,
}

/// How many traces wait for a hand that has not opened. A trace can arrive before its hand when
/// a resync replays a turn first; past a hand's worth of them the hand is never coming, and
/// holding them would be a leak the length of the process.
pub const PENDING_LIMIT: usize = 64;

impl Recorder {
    /// Files the traces the session reported for a frame just applied.
    ///
    /// A trace for the hand that is open goes into it. One for a hand that has not opened yet waits
    /// for it: a resync can replay a turn before the hand's own frames. A trace whose hand has been
    /// written already is dropped — a trace is kept with its hand or not at all
    /// (`Store::append_hand`) — and so is a waiting trace once more are waiting than any hand makes
    /// decisions.
    pub fn trace(&mut self, traced: Vec<(String, Trace)>) {
        for (id, trace) in traced {
            match &mut self.open {
                Some(hand) if hand.hand_id == id => hand.traced.push(trace),
                _ => self.pending.push((id, trace)),
            }
        }
        if self.pending.len() > PENDING_LIMIT {
            let overflow = self.pending.len() - PENDING_LIMIT;
            self.pending.drain(..overflow);
        }
    }

    /// Moves the traces that arrived before their hand into it.
    fn settle(&mut self) {
        let Some(hand) = &mut self.open else {
            return;
        };
        let (mine, later): (Vec<_>, Vec<_>) = self
            .pending
            .drain(..)
            .partition(|(id, _)| *id == hand.hand_id);
        hand.traced.extend(mine.into_iter().map(|(_, trace)| trace));
        self.pending = later;
    }

    /// Takes one frame: `text` as received, and `frame` as parsed from it. Returns the hand that
    /// this frame finishes, if it finishes one.
    pub fn record(&mut self, text: &str, frame: &Frame) -> Option<Hand> {
        match &frame.inbound {
            Inbound::Connected { .. } => None,
            Inbound::TableJoined { table_id, .. } => {
                self.table_id.clone_from(table_id);
                self.open = None;
                None
            }
            Inbound::HandStart { hand_id, .. } => {
                self.open = Some(Hand {
                    hand_id: hand_id.clone(),
                    table_id: self.table_id.clone(),
                    frames: vec![text.to_owned()],
                    traced: Vec::new(),
                });
                self.settle();
                None
            }
            Inbound::HandResult { .. } => self.close(text),
            Inbound::Resync(resync)
                if resync
                    .replayed
                    .iter()
                    .any(|event| matches!(event.inbound, Inbound::HandResult { .. })) =>
            {
                self.close(text)
            }
            _ => {
                if let Some(hand) = &mut self.open {
                    hand.frames.push(text.to_owned());
                }
                None
            }
        }
    }

    fn close(&mut self, text: &str) -> Option<Hand> {
        let mut hand = self.open.take()?;
        hand.frames.push(text.to_owned());
        Some(hand)
    }
}

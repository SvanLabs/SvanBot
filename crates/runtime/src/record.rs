//! The recorder: the venue's frames, as received, gathered into one [`Hand`] per deal.
//!
//! A hand is every frame from its `hand_start` to its `hand_result`, kept as the venue's own JSON
//! text, so nothing is lost in translation and the learner can read it again with any later parser.

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

/// Gathers frames into hands. It outlives a connection, like the session: a hand that a resync
/// finishes is still the same hand.
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    table_id: String,
    open: Option<Hand>,
    /// The traces the session reported since the last frame, waiting for their hand.
    pending: Vec<(String, Trace)>,
}

impl Recorder {
    /// Files the traces the session reported for a frame just applied.
    ///
    /// A trace whose hand has not arrived, or has already been written, is dropped: a trace is kept
    /// with its hand or not at all (`Store::append_hand`).
    pub fn trace(&mut self, traced: Vec<(String, Trace)>) {
        match &mut self.open {
            Some(hand) => hand.traced.extend(
                traced
                    .into_iter()
                    .filter(|(id, _)| *id == hand.hand_id)
                    .map(|(_, t)| t),
            ),
            None => self.pending.extend(traced),
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

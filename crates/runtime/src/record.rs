//! The recorder: the venue's frames, as received, gathered into one [`Hand`] per deal.
//!
//! A hand is every frame from its `hand_start` to its `hand_result`, kept as the venue's own JSON
//! text, so nothing is lost in translation and the learner can read it again with any later parser.

use crate::wire::{Frame, Inbound};

/// One hand as the bot saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hand {
    pub hand_id: String,
    pub table_id: String,
    /// The venue's frames, as received, oldest first.
    pub frames: Vec<String>,
}

/// Gathers frames into hands. It outlives a connection, like the session: a hand that a resync
/// finishes is still the same hand.
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    table_id: String,
    open: Option<Hand>,
}

impl Recorder {
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
                });
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

//! The venue's wire format, from <https://docs.openpoker.ai/llms-full.txt> (rev 2026-09-02,
//! "WebSocket Protocol"), and the only place it is known. Inbound frames become [`Inbound`] values
//! with `core`'s cards, each in a [`Frame`] with its envelope's `table_seq`; our [`Outbound`]
//! messages become the venue's JSON.
//!
//! Frames the bot does not act on read as [`Inbound::Other`], never as an error: the venue adds
//! message types without notice.

/// The venue documentation revision this parser implements, recorded on every stored hand as
/// its provenance: a parser update that changes how frames are read is a new revision here.
pub const DOCS_REVISION: &str = "2026-09-02";

mod inbound;
mod outbound;
mod snapshot;

pub use inbound::{Detail, Frame, Inbound, Offer, Player, Turn, WireError, parse, parse_frame};
pub use outbound::{Outbound, VenueAction};
pub use snapshot::{Hero, Resync, Seat, Snapshot};

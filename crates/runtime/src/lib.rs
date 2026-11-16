//! Everything that touches the world.
//!
//! It will hold the venue client, the store, updates and slots, the clock, and fresh entropy for
//! seeds. The client covers the WebSocket and REST API in
//! <https://docs.openpoker.ai/llms-full.txt>, the action clock, resync, the rate limits, and the
//! seat-list seam where the fair-play guard lives. The venue's wire format is translated to
//! `core`'s types here and nowhere else, so no layer below this one knows it.
//!
//! `runtime` sits on every pure layer (`docs/adr/0003-crate-graph.md`). It is one of the two
//! layers that do I/O, so it turns off the `clippy.toml` bans on the network, files, the clock and
//! the terminal, here, once.
#![allow(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::disallowed_macros
)]

pub mod client;
pub mod download;
pub mod fleet;
pub mod health;
pub mod history;
pub mod ledger;
pub mod limit;
pub mod lines;
pub mod probing;
pub mod record;
pub mod release;
pub mod rest;
pub mod session;
pub mod slot;
pub mod store;
pub mod update;
pub mod wire;

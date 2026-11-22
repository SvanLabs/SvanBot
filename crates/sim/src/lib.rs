//! Hands played out from a seed.
//!
//! It holds equity by enumeration and by sampling ([`equity`]), and the table that deals a hand
//! and plays it out ([`table`]), and the sparring archetypes ([`sparring`]). It will also hold the
//! vectorised loops behind them. The same seed gives the same result on any machine.
//!
//! `sim` sits on `core` (`docs/adr/0003-crate-graph.md`) and is pure: no network, no store, no
//! clock, and the same result for the same inputs.

pub mod equity;
pub mod sparring;
pub mod table;
pub mod v10;

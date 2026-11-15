//! Hands played out from a seed.
//!
//! It holds equity by enumeration and by sampling ([`equity`]), and the table that deals a hand
//! and plays it out ([`table`]). It will also hold the vectorised loops behind them, and the
//! sparring archetypes. The same seed gives the same result on any machine.
//!
//! `sim` sits on `core` (`docs/adr/0003-crate-graph.md`) and is pure: no network, no store, no
//! clock, and the same result for the same inputs.

pub mod equity;
pub mod table;

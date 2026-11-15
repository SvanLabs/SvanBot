//! Hands played out from a seed.
//!
//! It holds equity by enumeration and by sampling. It will also hold dealing, the vectorised loops
//! behind them, and the sparring archetypes. The same seed gives the same result on any machine.
//!
//! `sim` sits on `core` (`docs/adr/0003-crate-graph.md`) and is pure: no network, no store, no
//! clock, and the same result for the same inputs.

pub mod equity;

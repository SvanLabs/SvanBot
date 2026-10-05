//! Equity: Monte Carlo and shared-deal equity, exact board strengths, preflop class tables and the precomputed suit-canonical strength tables.
//!
//! Part of the svanbot10 workspace; `sv10-core` re-exports every module under one path.

#![warn(missing_docs)]
// Numeric kernels index several parallel arrays by the same index.
#![allow(clippy::needless_range_loop)]

pub mod abandon;
pub mod equity;
pub mod preflop;
mod preflop_data;
pub mod tables;

//! Decisions and evaluation: the exploitative EV policy, simulated agents and clone fitting, paired evaluation, hardware auto-tuning.
//!
//! Part of the SvanBot workspace; `sv10-core` re-exports every module under one path.

#![warn(missing_docs)]
// Numeric kernels index several parallel arrays by the same index.
#![allow(clippy::needless_range_loop)]

pub mod agents;
pub mod bench;
pub mod hardware;
pub mod policy;
pub mod sim;

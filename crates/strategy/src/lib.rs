//! How a bot decides at a spot.
//!
//! It will hold the action model and the ranges that follow from it, reads, the adaptivity class
//! and the exploit budget, and the policy: the re-solve, the pricer and the compute budget. A
//! decision is a function of the spot, the reads and the budget it is handed. When the action
//! clock limits the work, the caller passes the limit in as an input.
//!
//! `strategy` sits on `sim` and `core` (`docs/adr/0003-crate-graph.md`) and is pure: no
//! network, no store, no clock, and the same result for the same inputs.

//! Turns hands into fits and evidence into verdicts.
//!
//! It fits the action model on every hand and gates it on held-out hands ([`fit`]), fits the
//! population priors ([`prior`]), and watches the ranges for drift ([`drift`]). It will also
//! propose challengers, run paired sims, score the Arena, and give probe verdicts and the harm
//! guard. It takes hands as values and returns its results. Reading and writing the store is
//! `runtime`'s job.
//!
//! `learn` sits on `strategy`, `sim` and `core` (`docs/adr/0003-crate-graph.md`) and is pure: no
//! network, no store, no clock, and the same result for the same inputs.

pub mod drift;
pub mod fit;
pub mod prior;

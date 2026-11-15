//! Turns hands into fits and evidence into verdicts.
//!
//! It will fit action models, priors and reads, propose challengers, run paired sims, score
//! the Arena, and give probe verdicts and the harm guard. It takes hands as values and returns
//! its results. Reading and writing the store is `runtime`'s job.
//!
//! `learn` sits on `strategy`, `sim` and `core` (`docs/adr/0003-crate-graph.md`) and is pure: no
//! network, no store, no clock, and the same result for the same inputs.

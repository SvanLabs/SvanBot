//! Turns hands into fits and evidence into verdicts.
//!
//! It fits the action model on every hand and gates it on held-out hands ([`fit`]), fits the
//! population priors ([`prior`]), watches the ranges for drift ([`drift`]), classifies how each
//! opponent adapts ([`adapt`]), and gates reads on what they could cost ([`exploit`]), runs paired sims ([`paired`]), and gives probe verdicts and the harm guard ([`probe`]), and scores the Arena ([`arena`]). It plans
//! the unattended search for a challenger ([`search`]). It takes hands as values and returns its results. Reading and writing the store is
//! `runtime`'s job.
//!
//! `learn` sits on `strategy`, `sim` and `core` (`docs/adr/0003-crate-graph.md`) and is pure: no
//! network, no store, no clock, and the same result for the same inputs.

pub mod adapt;
pub mod arena;
pub mod drift;
pub mod exploit;
pub mod fit;
pub mod margin;
pub mod paired;
pub mod play;
pub mod prior;
pub mod probe;
pub mod profile;
pub mod read_sim;
pub mod reads;
pub mod ring;
pub mod search;

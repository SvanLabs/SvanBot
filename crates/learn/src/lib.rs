//! Turns hands into fits and evidence into verdicts.
//!
//! It fits the action model on every hand and gates it on held-out hands ([`fit`]), fits the
//! population priors ([`prior`]), watches the ranges for drift ([`drift`]), classifies how each
//! opponent adapts ([`adapt`]), and gates reads on what they could cost ([`exploit`]), runs paired
//! sims ([`paired`]), and gives probe verdicts and the harm guard ([`probe`]), and scores the Arena
//! ([`arena`]). It keeps each bot's last five champions and the regression rule that a promotion
//! must clear against them ([`archive`]), refreshes a dominated lineage to its best eligible
//! archived champion of its own style ([`refresh`]), plans and runs the bounded round-robin
//! over a bot's gauntlet ([`round_robin`]), and searches an exploiter against a brain's champion
//! ([`exploiter`]). It clones a bot's own field ([`field`]) and plans the
//! unattended search for a challenger ([`search`]), and offers a candidate one bot's search cleared
//! to another's, inside the receiver's style ([`transfer`]). It takes hands as values and returns
//! its results. Reading and writing the store is `runtime`'s job.
//!
//! `learn` sits on `strategy`, `sim` and `core` (`docs/adr/0003-crate-graph.md`) and is pure: no
//! network, no store, no clock, and the same result for the same inputs.

pub mod adapt;
pub mod archive;
pub mod arena;
pub mod calibration;
pub mod drift;
pub mod exploit;
pub mod exploiter;
pub mod field;
pub mod fit;
pub mod margin;
pub mod paired;
pub mod play;
pub mod prior;
pub mod probe;
pub mod profile;
pub mod read_sim;
pub mod reads;
pub mod reference;
pub mod refresh;
pub mod ring;
pub mod round_robin;
pub mod search;
pub mod transfer;

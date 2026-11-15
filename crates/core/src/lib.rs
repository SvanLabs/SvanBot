//! The rules of no-limit hold'em as the venue plays them, and the values every layer above shares.
//!
//! It holds the cards and their notation (`Ah`, `Ts`), the evaluator and the seeded generator. It
//! will also hold the rules of a hand (blinds, legal actions, raise-to sizes, side pots, showdown),
//! spots and digests.
//!
//! `core` is the bottom layer (`docs/adr/0003-crate-graph.md`). It depends on no other crate,
//! ours or third-party, and it is pure: no network, no store, no clock, and the same result for
//! the same inputs.

pub mod card;
pub mod eval;
pub mod rng;

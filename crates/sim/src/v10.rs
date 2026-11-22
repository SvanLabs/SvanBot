//! v10.0.2's policy, ported as a sparring opponent (V11-30).
//!
//! The Arena plays v10.0.2 so the ship bar has a row to read. It is written fresh from the
//! `v10.0.2` tag, pure and seeded like the rest of `sim`, and a parity test holds it to the
//! actions the real v10.0.2 binary took on the same synthetic spots: without parity the row
//! would not carry the name. Its encodings are v10's own (a card is `suit * 13 + rank`), because
//! its draws pick card numbers and its tables index by combo.
//!
//! Its generator is `core::rng`'s, which is the same xoshiro256++ with the same seeding that
//! v10's `SmallRng` used; [`draw`] holds v10's ways of turning its words into numbers.

pub mod cards;
pub mod deals;
pub mod draw;
pub mod equity;
pub mod eval;
pub mod preflop;
pub mod strengths;

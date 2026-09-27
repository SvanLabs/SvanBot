//! Multiway no-limit hold'em rules (blinds, raises, side pots, luck-free expected nets) and the decision input `Situation`.
//!
//! Part of the svanbot10 workspace; `sv10-core` re-exports every module under one path.

#![warn(missing_docs)]
// Numeric kernels index several parallel arrays by the same index.
#![allow(clippy::needless_range_loop)]

pub mod engine;
pub mod situation;

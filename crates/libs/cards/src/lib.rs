//! Cards, the verified 7-card evaluator, and the 1,326 hole-card combos with weighted ranges.
//!
//! Part of the svanbot10 workspace; `sv10-core` re-exports every module under one path.

#![warn(missing_docs)]
// Numeric kernels index several parallel arrays by the same index.
#![allow(clippy::needless_range_loop)]

pub mod cards;
pub mod eval;
pub mod range;

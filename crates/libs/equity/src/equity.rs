//! Equity estimation: Monte Carlo against weighted ranges, shared importance-weighted deals for
//! comparing candidate actions, and exact board strengths.

// One module per machinery (0261); the names below are re-exported so every caller keeps its
// path (`sv10_core::equity::SharedDeals` and friends are unchanged).
mod deals;
mod ranges;
mod sampler;
mod strengths;

pub use deals::SharedDeals;
pub use ranges::{equity_vs_ranges, equity_vs_ranges_parallel};
pub use sampler::ComboSampler;
pub use strengths::{combo_strengths, exact_strengths, river_equity_exact, river_strengths};

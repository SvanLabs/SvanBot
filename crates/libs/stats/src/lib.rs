//! Pure statistics for SvanBot (0231): no store, no network, no poker types, so every function is
//! unit-tested on plain numbers and shared by the fleet, the learner, the analyst and the tools.
//!
//! Carved out of `sv10-bot`, where each primitive had grown its own private copy (the running
//! mean and interval in `analysis` and `headtohead`, the sample-variance interval in `nnresidual`
//! and `margins`, the logistic helpers in `foldcal` and `playerfold`, the normal quantile in
//! `headtohead`). The formulas are kept operation for operation, so every result is bit-identical
//! to what those modules computed.
//!
//! - [`moments`]: means and 95%-style half-widths, from running sums or from samples.
//! - [`normal`]: the inverse standard normal CDF and the Bonferroni family-wise z.
//! - [`logistic`]: sigmoid, clamped logit and log-loss.
//! - [`optimize`]: golden-section minimisation of a unimodal function.
//! - [`grading`]: chess-style decision grades and accuracy (0220).
//! - [`margins`]: calibration residuals by predicted-EV bin (0222).
//! - [`exp_memo`]: an exact memo for `f32::exp`, bit-identical by construction (0353).

#![warn(missing_docs)]

pub mod exp_memo;
pub mod grading;
pub mod logistic;
pub mod margins;
pub mod moments;
pub mod normal;
pub mod optimize;

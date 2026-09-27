//! Opponent modelling: per-player statistics, hand histories, neural features, range reconstruction and its maximum-likelihood calibration.
//!
//! Part of the SvanBot workspace; `sv10-core` re-exports every module under one path.

#![warn(missing_docs)]
// Numeric kernels index several parallel arrays by the same index.
#![allow(clippy::needless_range_loop)]

pub mod adapt;
pub mod allin;
pub mod calibrate;
pub mod features;
pub mod flow;
pub mod history;
pub mod model;
pub mod oprange;
pub mod phh;
pub mod residual;
pub mod sizetell;

//! Multiway all-in call shift fit (0219), split from the raise-war module (0262).

use super::deep::DeepCallFit;
use super::{Commit, fit_call_band};

/// The multiway call shift on multiway spots (`crate::multiway`, oldest first), gated exactly like
/// [`fit_deep_call`]: `review multiway-calls` reports it; nothing installs it while it fails (0219).
pub fn fit_multiway_call(commits: &[Commit]) -> DeepCallFit {
    fit_call_band(commits, |_| true)
}

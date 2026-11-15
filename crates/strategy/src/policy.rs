//! The policy: how the bot decides at any spot. A heads-up spot after the flop is re-solved
//! ([`crate::resolve`]); the pricer ([`crate::pricer`]) decides every other one.
//!
//! The compute budget comes in from the caller, as the pricer's samples and the re-solve's
//! iterations ([`crate::resolve::iterations_for`]), so the policy never reads a clock.

use svanbot_core::rules::Action;
use svanbot_core::spot::Spot;

use crate::action::ActionModel;
use crate::pricer::{self, Budget};
use crate::resolve::resolve;

/// The action to play at `spot`: the re-solve's after `iterations` when it covers the spot, and
/// the pricer's otherwise. Both read the opponents' ranges from their lines, by `model` or, with
/// none, by the baseline model.
pub fn decide(spot: &Spot, model: Option<&ActionModel>, budget: Budget, iterations: u32) -> Action {
    // Until a fit passes the gate, the baseline reads the opponents' lines.
    let baseline = ActionModel::baseline();
    let model = Some(model.unwrap_or(&baseline));
    match resolve(spot, model, iterations, budget.seed) {
        Some(resolved) => resolved.action,
        None => pricer::decide(spot, model, budget).action,
    }
}

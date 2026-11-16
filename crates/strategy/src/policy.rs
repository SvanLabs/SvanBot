//! The policy: how the bot decides at any spot. A heads-up spot after the flop is re-solved
//! ([`crate::resolve`]); the pricer ([`crate::pricer`]) decides every other one.
//!
//! The compute budget comes in from the caller, as the pricer's samples and the re-solve's
//! iterations ([`crate::resolve::iterations_for`]), so the policy never reads a clock.

use svanbot_core::rules::Action;
use svanbot_core::spot::Spot;

use crate::action::ActionModel;
use crate::params::Params;
use crate::pricer::{self, Budget};
use crate::resolve::resolve;

/// The action to play at `spot`: the re-solve's after `iterations` when it covers the spot, and
/// the pricer's otherwise. Both read the opponents' ranges from their lines, by `model` or, with
/// none, by the baseline model. `params` is the parameter set it plays by.
pub fn decide(
    spot: &Spot,
    model: Option<&ActionModel>,
    params: &Params,
    budget: Budget,
    iterations: u32,
) -> Action {
    decide_drawn(spot, model, params, budget, iterations).action
}

/// A decision, and what it was drawn from when it was drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub action: Action,
    /// The re-solve's draw. The pricer draws nothing: it takes its best action every time.
    pub drawn: Option<Drawn>,
}

/// A draw among actions: the chance each had, what each was valued at in chips, and the one
/// taken. The luck correction of a hand reads it (`learn::probe::corrected`).
#[derive(Clone, Debug, PartialEq)]
pub struct Drawn {
    pub strategy: Vec<f64>,
    pub values: Vec<f64>,
    pub taken: usize,
}

/// [`decide`], with the draw behind the action.
pub fn decide_drawn(
    spot: &Spot,
    model: Option<&ActionModel>,
    params: &Params,
    budget: Budget,
    iterations: u32,
) -> Decision {
    // Until a fit passes the gate, the baseline reads the opponents' lines.
    let baseline = ActionModel::baseline();
    match resolve(spot, model, params.exploit, iterations, budget.seed) {
        Some(resolved) => Decision {
            action: resolved.action,
            drawn: Some(Drawn {
                strategy: resolved
                    .strategy
                    .iter()
                    .map(|&(_, chance)| chance)
                    .collect(),
                values: resolved.values,
                taken: resolved.taken,
            }),
        },
        None => Decision {
            action: pricer::price(spot, Some(model.unwrap_or(&baseline)), budget, params).action,
            drawn: None,
        },
    }
}

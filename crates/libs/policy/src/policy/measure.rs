//! The equity a decision is priced on: the shared deal set, the measurement against a subset of the
//! responders, and the refusal when neither can produce one (#424).

use super::*;
use sv10_equity::equity::equity_vs_ranges_parallel;

/// The shared deals this decision prices against, or `None` when there is nothing to reuse or the
/// draw could not fill the budget it was asked for (#424). A short deal set answers a mean over a
/// fraction of the deals asked for — on the issue's sparse-range harness, 1,700 of 2,500 at eleven
/// opponents, 63 at thirteen — and nothing downstream could tell, so it is treated as absent: the
/// measurement then takes a fresh draw over the estimated ranges, or refuses. An exact deal set is a
/// complete enumeration, so its count is the whole answer whatever the budget.
pub(super) fn shared_deals<R: Rng>(sit: &Situation, all_ranges: &[&Range], params: &Params, rng: &mut R) -> Option<SharedDeals> {
    (params.reuse_deals && !all_ranges.is_empty())
        .then(|| SharedDeals::new_parallel(sit.hole, &sit.board, all_ranges, params.samples, params.deal_chunks, rng))
        .filter(|d| d.is_exact() || d.len() >= params.samples)
}

/// Hero's equity against a subset of the responders: from the shared deal set when it can answer,
/// from a fresh draw over `refs` otherwise. `None` when neither can measure it — this is the one
/// number every candidate is priced with, so the caller refuses the decision rather than pricing it
/// on `0.0`, which reads as "hero never wins".
pub(super) fn measured_equity<R: Rng>(
    deals: Option<&SharedDeals>,
    sit: &Situation,
    params: &Params,
    subset: &[(usize, Option<&Range>)],
    refs: &[&Range],
    samples: usize,
    rng: &mut R,
) -> Option<f64> {
    match deals.and_then(|d| d.equity(subset)) {
        Some(e) => Some(e),
        None => equity_vs_ranges_parallel(sit.hole, &sit.board, refs, samples, params.deal_chunks, rng),
    }
}

/// The decision for a spot the draw could not measure (#424). Nothing can be priced without an
/// equity, so the action is the one that costs nothing — check where the rules allow it, fold
/// otherwise — and `equity` is left empty so the store, the dashboard and any later fit read a
/// missing measurement instead of a measurement of zero.
pub(super) fn unmeasured(sit: &Situation) -> Decision {
    let action = if sit.can_check { Action::Check } else { Action::Fold };
    let (action_name, _) = action_label(action);
    let call = sit.call_amount as f64;
    let pot = sit.pot as f64;
    let chosen =
        Candidate { action: action_name.clone(), amount: None, ev: 0.0, fold_prob: 0.0, equity_called: 0.0, category: None, bias: 0.0 };
    Decision {
        action,
        action_name,
        amount: None,
        equity: None,
        pot_odds: if call > 0.0 { call / (pot + call) } else { 0.0 },
        candidates: vec![chosen.clone()],
        reason: "no equity measurement: the deal draw could not fill its budget; taking the safe action".into(),
        chosen,
    }
}

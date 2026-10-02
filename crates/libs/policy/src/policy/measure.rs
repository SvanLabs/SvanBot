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

/// The existing one-pot estimate, retained as the default and as the fallback when commitments
/// cannot be reconstructed. It preserves the live all-in safeguard from 2026-09-15.
pub(super) fn legacy_all_in_fold_branch<R: Rng>(
    sit: &Situation,
    responders: &[Responder<'_>],
    all_in_idx: &[usize],
    deals: Option<&SharedDeals>,
    params: &Params,
    rng: &mut R,
) -> Option<f64> {
    if all_in_idx.is_empty() {
        return Some(sit.pot as f64);
    }
    let seats: Vec<usize> = all_in_idx.iter().map(|&i| responders[i].seat).collect();
    let (side, main) = sit.split_at_all_ins(&seats);
    let subset: Vec<(usize, Option<&Range>)> = all_in_idx.iter().map(|&i| (i, None)).collect();
    let refs: Vec<&Range> = all_in_idx.iter().map(|&i| responders[i].range).collect();
    let eq = measured_equity(deals, sit, params, &subset, &refs, params.samples / 2, rng)?;
    Some(side + eq * main)
}

/// Opt-in correction of unequal all-in tiers. The new raise is in the resulting pots; subtract
/// its added chips from hero's payout to price the action relative to folding.
#[allow(clippy::too_many_arguments)]
pub(super) fn all_in_fold_branch<R: Rng>(
    sit: &Situation,
    responders: &[Responder<'_>],
    all_in_idx: &[usize],
    deals: Option<&SharedDeals>,
    params: &Params,
    raise_to: i64,
    legacy: f64,
    rng: &mut R,
) -> Option<f64> {
    if !params.tiered_all_in_fold_pricing {
        return Some(legacy);
    }
    let seats: Vec<usize> = all_in_idx.iter().map(|&i| responders[i].seat).collect();
    let tiers = match sit.unequal_all_in_tiers_after_raise(&seats, raise_to) {
        Some(tiers) => tiers,
        None => return Some(legacy),
    };
    let payout = all_in_fold_payout(&tiers, |eligible| {
        let indices: Vec<usize> = all_in_idx.iter().copied().filter(|&i| eligible.contains(&responders[i].seat)).collect();
        let subset: Vec<(usize, Option<&Range>)> = indices.iter().map(|&i| (i, None)).collect();
        let refs: Vec<&Range> = indices.iter().map(|&i| responders[i].range).collect();
        measured_equity(deals, sit, params, &subset, &refs, params.samples / 2, rng)
    })?;
    Some(payout - (raise_to - sit.hero().bet) as f64)
}

/// Hero's payout when every opponent with chips behind folds: uncontested tiers are certain,
/// while each showdown tier uses equity against only the all-in seats eligible for that tier.
pub(super) fn all_in_fold_payout(tiers: &[(f64, Vec<usize>)], mut equity: impl FnMut(&[usize]) -> Option<f64>) -> Option<f64> {
    let mut payout = 0.0;
    for (amount, eligible) in tiers {
        payout += amount * if eligible.is_empty() { 1.0 } else { equity(eligible)? };
    }
    Some(payout)
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

#[cfg(test)]
mod tests {
    use super::*;
    use sv10_cards::cards::Card;
    use sv10_cards::range::combo_index;
    use sv10_engine::situation::PlayerInfo;
    use sv10_rng::SeedableRng;

    #[test]
    fn all_in_fold_payout_uses_each_tiers_eligible_range() {
        let tiers = [(300.0, vec![1, 2]), (400.0, vec![2]), (100.0, vec![])];
        let payout = all_in_fold_payout(&tiers, |seats| match seats {
            [1, 2] => Some(0.1),
            [2] => Some(0.9),
            _ => panic!("unexpected showdown tier: {seats:?}"),
        });
        assert_eq!(payout, Some(490.0));
        // The engine settlement fixture has hero between the shallow and deeper all-in hands:
        // the three-way main pot is lost, the deeper side pot is won, and 100 is uncontested.
        let settled = all_in_fold_payout(&tiers, |seats| Some(if seats == [2] { 1.0 } else { 0.0 }));
        assert_eq!(settled, Some(500.0));
        let after_raise = [(300.0, vec![1, 2]), (400.0, vec![2]), (200.0, vec![])];
        let payout = all_in_fold_payout(&after_raise, |seats| Some(if seats == [2] { 1.0 } else { 0.0 }));
        assert_eq!(payout.map(|won| won - 500.0), Some(100.0));
    }

    #[test]
    fn tiered_pricing_is_opt_in_and_matches_exact_settlement() {
        let card = |name| Card::parse(name).unwrap();
        let mut sit = sv10_engine::situation::fixtures::uncallable_overshove();
        sit.hole = [card("Kh"), card("Kd")];
        sit.board = sv10_cards::cards::parse_cards(&["2c", "3c", "4s", "9s", "Tc"]).unwrap();
        sit.players = [(0, 0, 600), (1, 100, 0), (2, 300, 0), (3, 0, 1_000)]
            .into_iter()
            .map(|(seat, bet, stack)| PlayerInfo { seat, name: format!("p{seat}"), bet, stack, folded: false })
            .collect();
        sit.pot = 400;
        sit.current_bet_to = Some(300);
        sit.call_amount = 300;
        sit.min_raise_to = Some(500);
        sit.max_raise_to = Some(600);
        sit.history.clear();

        let one_combo = |a, b| {
            let mut range = Range::empty();
            range.w[combo_index(card(a), card(b)).unwrap()] = 1.0;
            range
        };
        let ranges = [one_combo("Ah", "Ad"), one_combo("Qh", "Qd"), one_combo("Jh", "Jd")];
        let models = ModelStore::default();
        let responders: Vec<Responder<'_>> = ranges
            .iter()
            .enumerate()
            .map(|(i, range)| Responder {
                seat: i + 1,
                stack_total: if i < 2 { [100, 300][i] } else { 1_000 },
                bet: if i < 2 { [100, 300][i] } else { 0 },
                profile: models.profile("p"),
                range,
                acted_this_street: false,
                called_all_in: false,
            })
            .collect();
        let all_in_idx = [0, 1];
        let mut rng = sv10_rng::rngs::SmallRng::seed_from_u64(7);
        let deals = SharedDeals::new(sit.hole, &sit.board, &ranges.iter().collect::<Vec<_>>(), 60, &mut rng);
        assert_eq!(deals.equity(&[(0, None), (1, None)]), Some(0.0));
        assert_eq!(deals.equity(&[(1, None)]), Some(1.0));
        let params = Params { samples: 60, ..Params::default() };
        assert!(!params.tiered_all_in_fold_pricing);
        let legacy = legacy_all_in_fold_branch(&sit, &responders, &all_in_idx, Some(&deals), &params, &mut rng).unwrap();
        assert_eq!(legacy, 0.0);
        assert_eq!(all_in_fold_branch(&sit, &responders, &all_in_idx, Some(&deals), &params, 500, legacy, &mut rng), Some(0.0));
        let corrected = Params { tiered_all_in_fold_pricing: true, ..params };
        assert_eq!(all_in_fold_branch(&sit, &responders, &all_in_idx, Some(&deals), &corrected, 500, legacy, &mut rng), Some(100.0));
    }
}

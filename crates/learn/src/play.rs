//! Simulation deciders for a policy and a stablemate.

use svanbot_core::rules::Action;
use svanbot_core::spot::Spot;
use svanbot_strategy::action::ActionModel;
use svanbot_strategy::policy::{Personal, Policy};
use svanbot_strategy::pricer::Budget;
use svanbot_strategy::profile::SeatedStats;
use svanbot_strategy::resolve::iterations_for;

use crate::arena::opponent_name;
use crate::paired::{Challenger, Draws, Match, duel_seen};
use crate::probe::Decided;
use crate::reads::Reads;
use crate::ring::ring_seen;

pub const SAMPLES: u64 = 2_000;
pub const STABLEMATE: &str = "stablemate ";

/// Whether `name` is a row measured apart from the release rows: a stablemate row or a bot's own
/// field. The ship bar reads neither, and neither enters a policy's standard comparison.
pub fn apart(name: &str) -> bool {
    name.starts_with(STABLEMATE) || crate::field::is_row(name)
}

pub type Take = fn(&Reads, &[(usize, &str)], &ActionModel, f64) -> Vec<Personal>;

/// A decider reading the named opponents through the supplied read policy.
pub fn reading(
    policy: Policy,
    fixed: Option<u64>,
    seats: usize,
    take: Take,
    draws: Option<Draws>,
) -> impl Fn(&Spot, u64, &Reads) -> Action {
    reading_with(policy, fixed, seats, take, None, move |drawn| {
        if let Some(draws) = &draws {
            draws.borrow_mut().push(drawn);
        }
    })
}

/// The same decision maker without a handle, for comparisons shared across threads.
pub fn reading_untracked(
    policy: Policy,
    fixed: Option<u64>,
    seats: usize,
    take: Take,
) -> impl Fn(&Spot, u64, &Reads) -> Action + Sync {
    reading_model(policy, fixed, seats, take, None)
}

/// Evaluates the population model with the normal policy and read path.
pub fn reading_model(
    policy: Policy,
    fixed: Option<u64>,
    seats: usize,
    take: Take,
    model: Option<ActionModel>,
) -> impl Fn(&Spot, u64, &Reads) -> Action + Sync {
    reading_with(policy, fixed, seats, take, model, |_| {})
}

fn reading_with<F>(
    policy: Policy,
    fixed: Option<u64>,
    seats: usize,
    take: Take,
    model: Option<ActionModel>,
    report: F,
) -> impl Fn(&Spot, u64, &Reads) -> Action
where
    F: Fn(Decided),
{
    let names: Vec<_> = (0..seats).map(|seat| opponent_name(seats, seat)).collect();
    let fitted = model.is_some();
    let population = model.unwrap_or_else(ActionModel::baseline);
    move |spot, seed, reads| {
        let named: Vec<_> = spot
            .seats
            .iter()
            .map(|&seat| (seat, names[seat].as_str()))
            .collect();
        let personal = take(reads, &named, &population, policy.params.exploit);
        let stats = profile_stats(&policy, reads, &named);
        let budget = Budget {
            samples: SAMPLES,
            seed,
        };
        let decision = policy.decide(
            spot,
            fitted.then_some(&population),
            &personal,
            budget,
            iterations(spot, fixed),
            &stats,
        );
        if let Some(drawn) = decision.drawn {
            report(Decided {
                strategy: drawn.strategy,
                values: drawn.values,
                taken: drawn.taken,
            });
        }
        decision.action
    }
}

/// A stablemate plays the policy without opponent reads.
pub fn player(policy: Policy, fixed: Option<u64>) -> impl Fn(&Spot, u64) -> Action {
    move |spot, seed| {
        let budget = Budget {
            samples: SAMPLES,
            seed,
        };
        policy
            .decide(spot, None, &[], budget, iterations(spot, fixed), &[])
            .action
    }
}

fn iterations(spot: &Spot, fixed: Option<u64>) -> u32 {
    match fixed {
        Some(fixed) => u32::try_from(fixed).unwrap_or(u32::MAX),
        None => iterations_for(spot.pot, spot.big_blind, spot.board.len()),
    }
}

pub fn profile_stats(policy: &Policy, reads: &Reads, named: &[(usize, &str)]) -> Vec<SeatedStats> {
    if policy.uses_stats() {
        reads.profile_stats(named)
    } else {
        Vec::new()
    }
}

pub fn rival_duel(
    game: &Match,
    deal: u64,
    challenger: &impl Challenger,
    rival: &impl Fn(&Spot, u64) -> Action,
) -> f64 {
    duel_seen(
        game,
        deal,
        challenger,
        &|_, spot, _, seed| rival(spot, seed),
        || {},
    )
}

pub fn rival_ring_at(
    game: &Match,
    seats: usize,
    deal: u64,
    challenger: &impl Challenger,
    rival: &impl Fn(&Spot, u64) -> Action,
) -> f64 {
    ring_seen(
        game,
        seats,
        deal,
        challenger,
        &|_, spot, _, seed| rival(spot, seed),
        |_| {},
    )
}

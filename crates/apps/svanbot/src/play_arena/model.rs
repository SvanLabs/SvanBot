//! A local six-max comparison of two population models on identical Arena deals.

use std::path::Path;
use std::process::ExitCode;

use svanbot_core::{rules::Action, spot::Spot};
use svanbot_learn::arena::BoutReader;
use svanbot_learn::margin::{Bounds, MIN_BOUTS, model_comparison};
use svanbot_learn::play;
use svanbot_learn::reads::Reads;
use svanbot_runtime::fleet::{Fleet, League};
use svanbot_runtime::release::digest;
use svanbot_strategy::action::ActionModel;
use svanbot_strategy::policy::Policy;

use super::{CommandResult, arena, deciders, settings, usage};
use crate::{arena::Opponent, fit::read_model};

pub(super) fn compare(fleet: &Fleet, args: &[String]) -> CommandResult {
    let [candidate, incumbent, tail @ ..] = args else {
        return Err(usage());
    };
    let (tail, confirmation) = match tail {
        [settings @ .., flag, seed] if flag == "--confirm" => {
            let seed = seed.parse::<u64>().map_err(|_| usage())?;
            (settings, Some(seed))
        }
        _ => (tail, None),
    };
    let settings = settings(tail)?;
    if let Some(seed) = confirmation {
        if seed == settings.seed {
            return Err((2, "confirmation requires a fresh seed".into()));
        }
        if settings.fixed.is_some() || settings.bouts < MIN_BOUTS as u64 {
            return Err((
                2,
                format!("confirmation requires the live budget and at least {MIN_BOUTS} bouts"),
            ));
        }
    }
    let candidate = load(candidate, "candidate")?;
    let incumbent = if incumbent == "baseline" {
        None
    } else {
        Some(load(incumbent, "incumbent")?)
    };
    let champion = fleet.champion().map_err(|error| (2, error))?;
    let policy = Policy::reference(champion);
    let league = League::SixMax;
    let seats = league.seats();
    let fixed = settings.fixed;
    let incumbent_id = incumbent
        .as_ref()
        .map_or_else(|| "baseline".into(), model_id);
    let candidate_id = model_id(&candidate);
    let active_budget = fixed.map_or_else(
        || "live budget".into(),
        |n| format!("fixed iterations: {n}"),
    );
    let commit = std::env::var("SVANBOT_COMMIT").unwrap_or_else(|_| "local".into());
    println!(
        "{} on {} with {} bout(s) per opponent, {active_budget}\nbuild {commit}\nchampion {}\nincumbent {incumbent_id}\ncandidate {candidate_id}",
        policy.name(),
        league.name(),
        settings.bouts,
        champion.digest()
    );

    let decider = |model| play::reading_model(policy, fixed, seats, Reads::deciding, model);
    let candidate = decider(Some(candidate));
    let reference = decider(incumbent);
    let measure = |seed| {
        println!("Seed {seed}");
        let game = deciders::game(seed);
        let rows = arena::play_rows(
            &Opponent::all(),
            Opponent::name,
            settings.bouts,
            |opponent, number| {
                let next = arena::play_bout_for(
                    &game,
                    league,
                    opponent,
                    number,
                    &policy,
                    fixed,
                    &reader(&candidate, &policy),
                );
                let base = arena::play_bout_for(
                    &game,
                    league,
                    opponent,
                    number,
                    &policy,
                    fixed,
                    &reader(&reference, &policy),
                );
                next.into_iter()
                    .zip(base)
                    .map(|(next, base)| next - base)
                    .collect()
            },
        );
        let samples: Vec<_> = rows.iter().map(|(_, sums)| sums.as_slice()).collect();
        let scored = model_comparison(&samples).ok_or((1, "invalid paired bout results".into()))?;
        for ((row, _), bounds) in rows.iter().zip(&scored.rows) {
            print_bounds(&row.opponent, *bounds);
        }
        print_bounds("overall", scored.overall);
        Ok::<bool, (u8, String)>(fixed.is_none() && scored.passes())
    };

    if !measure(settings.seed)? {
        println!("The sim margins were not established. Keep the incumbent.");
        return Ok(ExitCode::from(1));
    }
    let Some(seed) = confirmation else {
        println!("The first run met the margins. Fresh confirmation is required.");
        return Ok(ExitCode::from(1));
    };
    if !measure(seed)? {
        println!("The confirmation missed the margins. Keep the incumbent.");
        return Ok(ExitCode::from(1));
    }
    println!("Both paired sims met the margins.");
    Ok(ExitCode::SUCCESS)
}

fn reader<'a, F>(decide: &'a F, policy: &Policy) -> BoutReader<&'a F>
where
    F: Fn(&Spot, u64, &Reads) -> Action,
{
    BoutReader::new(decide).with_profile_stats(policy.uses_stats())
}

fn load(path: &str, role: &str) -> Result<ActionModel, (u8, String)> {
    read_model(Path::new(path)).ok_or_else(|| (2, format!("could not read the {role} model")))
}

fn model_id(model: &ActionModel) -> String {
    digest(model.to_text().as_bytes())
}

fn print_bounds(name: &str, Bounds { mean, radius }: Bounds) {
    let (lower, upper) = (mean - radius, mean + radius);
    println!("{name:>16} {mean:+.2} mbb/100, interval [{lower:+.2}, {upper:+.2}]");
}

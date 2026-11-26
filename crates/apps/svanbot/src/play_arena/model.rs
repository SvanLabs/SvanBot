//! A local six-max comparison of two population models on identical Arena deals.

use std::path::Path;
use std::process::ExitCode;

use svanbot_core::{rules::Action, spot::Spot};
use svanbot_learn::arena::{BOUT, BoutReader};
use svanbot_learn::margin::MIN_BOUTS;
use svanbot_learn::play;
use svanbot_learn::probe::Sequence;
use svanbot_learn::reads::Reads;
use svanbot_runtime::fleet::{Fleet, League};
use svanbot_runtime::release::digest;
use svanbot_strategy::action::ActionModel;
use svanbot_strategy::policy::Policy;

use super::{CommandResult, arena, deciders, settings, usage};
use crate::{arena::Opponent, fit::read_model};

const MAX_DRAWBACK: f64 = 5.0;

pub(super) fn compare(fleet: &Fleet, args: &[String]) -> CommandResult {
    let [candidate, incumbent, tail @ ..] = args else {
        return Err(usage());
    };
    let settings = settings(tail)?;
    let candidate = load(candidate, "candidate")?;
    let incumbent = if incumbent == "baseline" {
        None
    } else {
        Some(load(incumbent, "incumbent")?)
    };
    let champion = fleet.champion().map_err(|error| (2, error))?;
    let policy = Policy::reference(champion);
    let game = deciders::game(settings.seed);
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
        "{} on {} with {} bout(s) per opponent, seed {}, {active_budget}\nbuild {commit}\nchampion {}\nincumbent {incumbent_id}\ncandidate {candidate_id}",
        policy.name(),
        league.name(),
        settings.bouts,
        game.seed,
        champion.digest()
    );

    let decider = |model| play::reading_model(policy, fixed, seats, Reads::deciding, model);
    let candidate = decider(Some(candidate));
    let reference = decider(incumbent);
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
            let reference = arena::play_bout_for(
                &game,
                league,
                opponent,
                number,
                &policy,
                fixed,
                &reader(&reference, &policy),
            );
            next.into_iter()
                .zip(reference)
                .map(|(next, reference)| next - reference)
                .collect()
        },
    );
    let mut pooled = vec![0.0; settings.bouts as usize];
    let alpha = 0.05 / (rows.len() + 1) as f64;
    let mut rows_clear = true;
    for (row, sums) in &rows {
        rows_clear &= bounds(&row.opponent, sums.iter().copied(), alpha) >= -MAX_DRAWBACK;
        for (average, sum) in pooled.iter_mut().zip(sums) {
            *average += sum / rows.len() as f64;
        }
    }
    let lower = bounds("overall", pooled.into_iter(), alpha);
    let passes = settings.bouts >= MIN_BOUTS as u64 && fixed.is_none() && lower > 0.0 && rows_clear;
    println!(
        "{}",
        if passes {
            "Single run met the sim margins. Fresh confirmation is required."
        } else {
            "The sim margins were not established. Keep the incumbent."
        }
    );
    Ok(if passes {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

fn reader<'a, F>(decide: &'a F, policy: &Policy) -> BoutReader<&'a F>
where
    F: Fn(&Spot, u64, &Reads) -> Action,
{
    BoutReader::new(decide).with_profile_stats(!policy.style.profile.is_empty())
}

fn load(path: &str, role: &str) -> Result<ActionModel, (u8, String)> {
    read_model(Path::new(path)).ok_or_else(|| (2, format!("could not read the {role} model")))
}

fn model_id(model: &ActionModel) -> String {
    digest(model.to_text().as_bytes())
}

fn bounds(name: &str, sums: impl Iterator<Item = f64>, alpha: f64) -> f64 {
    let mut sequence = Sequence::new(200.0);
    for sum in sums {
        sequence.observe(sum * (100.0 / BOUT as f64));
    }
    let mean = sequence.mean();
    let radius = sequence.radius(alpha);
    let lower = mean - radius;
    println!(
        "{name:>16} {mean:+.2} bb/100, interval [{lower:+.2}, {:+.2}]",
        mean + radius
    );
    lower
}

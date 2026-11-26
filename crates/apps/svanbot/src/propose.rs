//! `svanbot propose`: the learner puts a challenger forward for the canary, then exits. It
//! touches neither the venue nor a store: it reads the fleet's champion and writes the fleet's
//! proposal.
//!
//! `svanbot propose [--probe] [--deals <n>] [--seed <n>] [<name>=<value> …]`
//!
//! - With numbers, the challenger is the champion with those numbers changed. With none, each
//!   number is tried a step down and a step up, and the best of them is the challenger.
//! - A paired sim plays each challenger against the champion, heads up.
//! - A challenger the sim puts above the champion, the whole 95% interval above 0, is proposed
//!   as a promotion: the canary runs its harm guard of [`GUARD_HANDS`] hands, and the fleet
//!   adopts it only if that clears.
//! - With `--probe`, the challenger is proposed as a probe whatever the sim said. A probe's
//!   verdict is evidence, and it promotes nothing.
//!
//! It exits 1 when nothing was proposed.

use std::process::ExitCode;

use svanbot_core::rules::Blinds;
use svanbot_core::spot::Spot;
use svanbot_learn::paired::{Match, Verdict};
use svanbot_learn::probe::Kind;
use svanbot_learn::search::Stage;
use svanbot_runtime::fleet::{Fleet, Proposal};
use svanbot_strategy::params::Params;
use svanbot_strategy::policy::Policy;
use svanbot_strategy::pricer::Budget;
use svanbot_strategy::resolve::iterations_for;

use svanbot::learn::{GUARD_HANDS, GUARD_MARGIN, PROBE_ORBITS};
use svanbot_learn::search::RINGS;

use crate::paired::sim;

/// The loss, in bb/100, a probe counts as harm.
const PROBE_MARGIN: f64 = 5.0;
/// The steps the search tries on each number: the search's widest ring.
const STEPS: [f64; 2] = RINGS[0];
/// Deals a `svanbot learn` sim plays unless told otherwise.
const LEARN_DEALS: u64 = 2_000;
/// Deals sampled by the pricer per decision in a sim.
const SAMPLES: u64 = 2_000;

pub(crate) fn run(args: &[String]) -> ExitCode {
    let fleet = Fleet::in_dir(crate::fleet_dir());
    let champion = match fleet.champion() {
        Ok(champion) => champion,
        Err(why) => {
            eprintln!("svanbot propose: the champion file is refused: {why}");
            return ExitCode::from(2);
        }
    };
    let Some(asked) = Asked::read(args, champion) else {
        eprintln!(
            "svanbot propose: usage: svanbot propose [--probe] [--deals <n>] [--seed <n>] [<name>=<value> …]"
        );
        eprintln!(
            "svanbot propose: the numbers are {}",
            champion.named().map(|(name, _)| name).join(", ")
        );
        return ExitCode::from(2);
    };
    let game = Match {
        blinds: Blinds { small: 10, big: 20 },
        stack: 2000,
        seed: asked.seed,
    };
    let mut best: Option<(Params, Verdict)> = None;
    for challenger in asked.challengers {
        let found = sim(
            &game,
            asked.deals,
            &player(Policy::reference(challenger)),
            &player(Policy::reference(champion)),
        );
        println!(
            "{} against champion {}: {:+.0} mbb/100, 95% CI ±{:.0}, over {} deals (seed {})",
            describe(&challenger, &champion),
            champion.digest(),
            found.mbb_per_100,
            found.ci95,
            found.deals,
            game.seed
        );
        let lower = |verdict: &Verdict| verdict.mbb_per_100 - verdict.ci95;
        if best.is_none_or(|(_, held)| lower(&found) > lower(&held)) {
            best = Some((challenger, found));
        }
    }
    let Some((challenger, found)) = best else {
        return ExitCode::from(2);
    };
    let proposal = Proposal {
        challenger,
        kind: if asked.probe {
            Kind::Probe
        } else {
            Kind::Guard { hands: GUARD_HANDS }
        },
        harm_margin: if asked.probe {
            PROBE_MARGIN
        } else {
            GUARD_MARGIN
        },
        sim_estimate: Some(found.mbb_per_100 / 1000.0),
        max_orbits: PROBE_ORBITS,
        against: Some(champion.digest()),
    };
    if !asked.probe && !found.above_zero() {
        println!("svanbot propose: no challenger is above the champion: nothing proposed");
        return ExitCode::from(1);
    }
    match fleet.propose(&proposal) {
        Ok(()) => {
            println!(
                "svanbot propose: {} proposed as a {}",
                challenger.digest(),
                if asked.probe { "probe" } else { "promotion" }
            );
            ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("svanbot propose: {why}");
            ExitCode::from(2)
        }
    }
}

/// `svanbot learn [--deals <n>]`: one step of the unattended search ([`svanbot::learn`]). With
/// champion slots it is one bot's step, rotating through them, and a confirmation is played on
/// fresh 6-max ring deals; without a slot it is the fleet champion's heads-up step. It exits 0
/// whatever the step was, waiting included, and 2 when the fleet's files cannot be read or
/// written.
pub(crate) fn learn(args: &[String]) -> ExitCode {
    let deals = match args {
        [] => LEARN_DEALS,
        [flag, deals] if flag == "--deals" => match deals.parse::<u64>() {
            Ok(deals) if deals > 1 => deals,
            _ => return learn_usage(),
        },
        _ => return learn_usage(),
    };
    let fleet = Fleet::in_dir(crate::fleet_dir());
    let slots = match fleet.champion_slots() {
        Ok(slots) => slots,
        Err(why) => {
            eprintln!("svanbot learn: the champion slots are refused: {why}");
            return ExitCode::from(2);
        }
    };
    if !slots.is_empty() {
        // A screening run is a fifth of the confirmation: enough to rank the challengers, not
        // enough to decide one.
        let screen_deals = (deals / 5).max(2);
        let run = |challenger: &Policy,
                   champion: &Policy,
                   step: &svanbot_learn::search::Step,
                   deals: u64| {
            let game = Match {
                blinds: Blinds { small: 10, big: 20 },
                stack: 2000,
                seed: step.seed,
            };
            let challenger = player(*challenger);
            let champion = player(*champion);
            match step.stage {
                // Screening is cheap and heads up; only the promising challengers buy the
                // fresh 6-max confirmation.
                Stage::First => sim(&game, deals, &challenger, &champion),
                Stage::Confirmation => {
                    crate::paired::ring_sim(&game, deals, &challenger, &champion)
                }
            }
        };
        return match svanbot::learn::bot_step(&fleet, screen_deals, deals, svanbot::now_ms(), &run)
        {
            Ok(note) => {
                println!("svanbot learn: {note}");
                ExitCode::SUCCESS
            }
            Err(why) => {
                eprintln!("svanbot learn: {why}");
                ExitCode::from(2)
            }
        };
    }
    let champion = match fleet.champion() {
        Ok(champion) => champion,
        Err(why) => {
            eprintln!("svanbot learn: the champion file is refused: {why}");
            return ExitCode::from(2);
        }
    };
    let run = |step: &svanbot_learn::search::Step, deals: u64| {
        // The search asks only round the champion it was given, which is this one.
        debug_assert_eq!(step.champion, champion.digest());
        let game = Match {
            blinds: Blinds { small: 10, big: 20 },
            stack: 2000,
            seed: step.seed,
        };
        sim(
            &game,
            deals,
            &player(Policy::reference(step.challenger)),
            &player(Policy::reference(champion)),
        )
    };
    match svanbot::learn::step(&fleet, deals, svanbot::now_ms(), &run) {
        Ok(note) => {
            println!("svanbot learn: {note}");
            ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("svanbot learn: {why}");
            ExitCode::from(2)
        }
    }
}

fn learn_usage() -> ExitCode {
    eprintln!("svanbot learn: usage: svanbot learn [--deals <n>]");
    ExitCode::from(2)
}

/// What the command line asked for.
struct Asked {
    probe: bool,
    deals: u64,
    seed: u64,
    challengers: Vec<Params>,
}

impl Asked {
    fn read(args: &[String], champion: Params) -> Option<Self> {
        let mut asked = Self {
            probe: false,
            deals: 400,
            seed: 1,
            challengers: Vec::new(),
        };
        let mut challenger = champion;
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            match (arg.as_str(), arg.split_once('=')) {
                ("--probe", _) => asked.probe = true,
                ("--deals", _) => asked.deals = args.next()?.parse().ok()?,
                ("--seed", _) => asked.seed = args.next()?.parse().ok()?,
                (_, Some((name, value))) => {
                    challenger = challenger.with(name, value.parse().ok()?)?;
                }
                _ => return None,
            }
        }
        asked.challengers = if challenger == champion {
            // The search: one number moved one step, each way.
            champion
                .named()
                .into_iter()
                .flat_map(|(name, value)| STEPS.map(|step| champion.with(name, value * step)))
                .flatten()
                .collect()
        } else {
            vec![challenger]
        };
        (asked.deals > 1).then_some(asked)
    }
}

/// The policy playing under its kind, style and `params`, as a sim's seat.
fn player(policy: Policy) -> impl Fn(&Spot, u64) -> svanbot_core::rules::Action + Sync {
    move |spot, seed| {
        let budget = Budget {
            samples: SAMPLES,
            seed,
        };
        let iterations = iterations_for(spot.pot, spot.big_blind, spot.board.len());
        policy
            .decide(spot, None, &[], budget, iterations, &[])
            .action
    }
}

/// The challenger's digest and the numbers it changes.
fn describe(challenger: &Params, champion: &Params) -> String {
    let changed: Vec<String> = challenger
        .named()
        .into_iter()
        .zip(champion.named())
        .filter(|((_, new), (_, old))| new != old)
        .map(|((name, new), _)| format!("{name}={new}"))
        .collect();
    format!("{} ({})", challenger.digest(), changed.join(", "))
}

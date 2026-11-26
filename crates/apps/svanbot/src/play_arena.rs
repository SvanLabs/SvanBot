//! Commands for evaluating and assigning policies.

use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use svanbot::learn::{GUARD_HANDS, GUARD_MARGIN};
use svanbot::play;
use svanbot::play::guard::{Guarded, guard};
use svanbot_learn::arena::BOUT;
use svanbot_learn::margin::{Admission, MARGIN};
use svanbot_runtime::fleet::{ArenaRow, ArenaRun, Check, Fleet, League};
use svanbot_runtime::ship::{self, Bar, Bout, CAP_HOURS};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::policy::Policy;
use svanbot_strategy::style::{self, Style};

use crate::arena::{self, Opponent};
use crate::{arena_ship, deciders};

mod model;

const USAGE: &str = "usage: svanbot play arena <solving|pricing> <standard|tight|loose> [league] <bouts> [seed] [iterations] [out.json]
       svanbot play compare-model <candidate> <baseline|incumbent> <bouts> [seed] [iterations] [--confirm <fresh-seed>]
       svanbot play check <kind> <style> <bouts|--until-decided> [seed] [iterations]
       svanbot play assign <bot> <kind> <style>
       svanbot play seed <bot>
       svanbot play revert <bot>
       svanbot play status
       svanbot play guard";

type CommandResult = Result<ExitCode, (u8, String)>;

pub(crate) fn run(args: &[String]) -> ExitCode {
    match command(args) {
        Ok(code) => code,
        Err((code, error)) => {
            eprintln!("svanbot play: {error}");
            ExitCode::from(code)
        }
    }
}

fn command(args: &[String]) -> CommandResult {
    let dir = crate::fleet_dir();
    let fleet = Fleet::in_dir(&dir);
    let data_dir = dir
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    match args {
        [name, args @ ..] if name == "compare-model" => model::compare(&fleet, args),
        [name, kind, style, tail @ ..] if name == "arena" => {
            let (kind, style) = policy_names(kind, style)?;
            play_arena(&fleet, kind, style, tail)
        }
        [name, kind, style, tail @ ..] if name == "check" => {
            let (kind, style) = policy_names(kind, style)?;
            check(&fleet, kind, style, tail)
        }
        [name, bot, kind, style] if name == "assign" => {
            let (kind, style) = policy_names(kind, style)?;
            say(play::assign(
                &fleet,
                data_dir,
                bot,
                kind,
                style,
                svanbot::now_ms(),
            ))
        }
        [name, bot] if name == "seed" => say(play::seed(&fleet, data_dir, bot)),
        [name, bot] if name == "revert" => say(play::revert(&fleet, bot)),
        [name] if name == "status" => {
            print!("{}", play::status(&fleet).map_err(|error| (2, error))?);
            Ok(ExitCode::SUCCESS)
        }
        [name] if name == "guard" => {
            for (bot, result) in guard(&fleet, data_dir, GUARD_HANDS as usize, GUARD_MARGIN)
                .map_err(|error| (1, error))?
            {
                match result {
                    Guarded::NotYet(note) => println!("{bot}: {note}"),
                    Guarded::Holds(note) => println!("{bot}: holds, {note}"),
                    Guarded::Trips(_) => (),
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        _ => Err(usage()),
    }
}

fn say(result: Result<String, String>) -> CommandResult {
    println!("{}", result.map_err(|error| (2, error))?);
    Ok(ExitCode::SUCCESS)
}

fn usage() -> (u8, String) {
    (2, USAGE.into())
}

fn policy_names(
    kind: &str,
    style_name: &str,
) -> Result<(PolicyKind, &'static Style), (u8, String)> {
    match (PolicyKind::named(kind), style::named(style_name)) {
        (Some(kind), Some(style)) => Ok((kind, style)),
        _ => Err((2, format!("unknown policy {kind}/{style_name}\n{USAGE}"))),
    }
}

struct Settings {
    bouts: u64,
    seed: u64,
    fixed: Option<u64>,
}

fn settings(args: &[String]) -> Result<Settings, (u8, String)> {
    let number = |at: usize| args.get(at).map(|text| text.parse::<u64>());
    let (Some(Ok(bouts @ 1..)), Ok(seed), Ok(fixed)) =
        (number(0), number(1).transpose(), number(2).transpose())
    else {
        return Err(usage());
    };
    if args.len() > 3 || bouts > u64::MAX / BOUT {
        return Err(usage());
    }
    Ok(Settings {
        bouts,
        seed: seed.unwrap_or(1),
        fixed,
    })
}

fn play_arena(
    fleet: &Fleet,
    kind: PolicyKind,
    style: &'static Style,
    args: &[String],
) -> CommandResult {
    let (league, args) = match args.first().and_then(|name| League::named(name)) {
        Some(league) => (league, &args[1..]),
        None => (League::HeadsUp, args),
    };
    if args.len() > 4 {
        return Err(usage());
    }
    let settings = settings(&args[..args.len().min(3)])?;
    let champion = fleet.champion().map_err(|error| (2, error))?;
    let policy = Policy::new(kind, style, champion);
    let game = deciders::game(settings.seed);
    let mut opponents = Opponent::shared();
    opponents.extend(style::ALL.map(Opponent::Stablemate));
    let rows = arena::play_rows(
        &opponents,
        Opponent::name,
        settings.bouts,
        |opponent, number| {
            arena::play_bout(&game, league, opponent, number, &policy, settings.fixed)
        },
    );
    let name = format!("{}@{}", policy.name(), champion.digest());
    let run = arena::report(league, name, game.seed, settings.fixed, rows);
    fleet
        .record_policy_arena(&run)
        .map_err(|error| (1, error))?;
    if let Some(path) = args.get(3) {
        std::fs::write(path, run.as_text() + "\n")
            .map_err(|error| (1, format!("{path} was not written: {error}")))?;
    }
    Ok(ExitCode::SUCCESS)
}

fn check(fleet: &Fleet, kind: PolicyKind, style: &'static Style, args: &[String]) -> CommandResult {
    let champion = fleet.champion().map_err(|error| (2, error))?;
    let reference = Policy::reference(champion);
    let candidate = Policy::new(kind, style, champion);
    if let Some((flag, tail)) = args.split_first()
        && flag == "--until-decided"
    {
        return check_until_decided(fleet, reference, candidate, tail);
    }
    let settings = settings(args)?;
    let game = deciders::game(settings.seed);
    let opponents = Opponent::all();
    let measure = |league, policy: Policy| {
        let rows = arena::play_rows(
            &opponents,
            Opponent::name,
            settings.bouts,
            |opponent, number| {
                arena::play_bout(&game, league, opponent, number, &policy, settings.fixed)
            },
        );
        let name = format!("{}@{}", policy.name(), champion.digest());
        arena::measurement(league, name, game.seed, settings.fixed, rows)
    };
    let judge = |league| {
        let base = measure(league, reference);
        let next = measure(league, candidate);
        play::judge(&base, &next, MARGIN).map_err(|error| (2, error))
    };
    let (heads, heads_up) = judge(League::HeadsUp)?;
    let (ring, six_max) = judge(League::SixMax)?;
    let admitted = heads.admission == Admission::Admitted && ring.admission == Admission::Admitted;
    record_check(fleet, candidate, settings, admitted, heads_up, six_max)
}

fn check_until_decided(
    fleet: &Fleet,
    reference: Policy,
    candidate: Policy,
    args: &[String],
) -> CommandResult {
    let number = |index: usize| args.get(index).map(|text| text.parse::<u64>()).transpose();
    let (Ok(seed), Ok(fixed)) = (number(0), number(1)) else {
        return Err(usage());
    };
    if args.len() > 2 {
        return Err(usage());
    }
    let game = deciders::game(seed.unwrap_or(1));
    let fresh = |league, policy: Policy| ArenaRun {
        commit: std::env::var("SVANBOT_COMMIT").unwrap_or_else(|_| ArenaRun::LOCAL_COMMIT.into()),
        champion: Some(format!("{}@{}", policy.name(), policy.params.digest())),
        seed: game.seed,
        league,
        iterations: fixed,
        rows: Opponent::all()
            .into_iter()
            .map(|opponent| ArenaRow {
                opponent: opponent.name(),
                deals: 0,
                mbb_per_100: 0.,
                ci95: 0.,
                bouts: Vec::new(),
            })
            .collect(),
    };
    let mut runs = [
        fresh(League::HeadsUp, reference),
        fresh(League::HeadsUp, candidate),
        fresh(League::SixMax, reference),
        fresh(League::SixMax, candidate),
    ];
    let started = Instant::now();
    let cap = Duration::from_secs(3600 * CAP_HOURS);
    let mut play = |runs: &[ArenaRun], bouts: &[Bout]| {
        arena_ship::heat(runs, bouts, |index, run, opponent, number| {
            let policy = if index.is_multiple_of(2) {
                &reference
            } else {
                &candidate
            };
            arena::play_bout(&game, run.league, opponent, number, policy, fixed)
                .into_iter()
                .sum()
        })
    };
    let outcome = ship::sample_with(
        &mut runs,
        &mut play,
        &mut || started.elapsed() >= cap,
        &mut |_| {},
        &|runs| play::check_bar(runs, MARGIN),
    );
    let [(heads, heads_up), (_, six_max)] =
        play::check_readings(&runs, MARGIN).map_err(|error| (1, error))?;
    let admitted = outcome == Bar::Clears;
    if let Bar::Refused(reasons) = outcome {
        for reason in reasons {
            eprintln!("svanbot play check: {reason}");
        }
    }
    let settings = Settings {
        bouts: heads.bouts as u64,
        seed: game.seed,
        fixed,
    };
    record_check(fleet, candidate, settings, admitted, heads_up, six_max)
}

fn record_check(
    fleet: &Fleet,
    policy: Policy,
    settings: Settings,
    admitted: bool,
    heads_up: String,
    six_max: String,
) -> CommandResult {
    println!("{heads_up}\n{six_max}");
    fleet
        .record_check(&Check {
            kind: policy.kind,
            style: policy.style.name.into(),
            reference: policy.params.digest(),
            seed: settings.seed,
            iterations: settings.fixed,
            bouts: settings.bouts,
            admitted,
            heads_up,
            six_max,
            checked_ms: svanbot::now_ms(),
        })
        .map_err(|error| (1, error))?;
    Ok(ExitCode::from(u8::from(!admitted)))
}

//! Commands for evaluating and assigning policies.

use std::path::Path;
use std::process::ExitCode;

use svanbot::play;
use svanbot_learn::arena::BOUT;
use svanbot_learn::margin::{Admission, MARGIN};
use svanbot_runtime::fleet::{Check, Fleet, League};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::policy::Policy;
use svanbot_strategy::style::{self, Style};

use crate::arena::{self, Opponent};
use crate::deciders;

const USAGE: &str = "usage: svanbot play arena <solving|pricing> <standard|tight|loose> [league] <bouts> [seed] [iterations] [out.json]
       svanbot play check <kind> <style> <bouts> [seed] [iterations]
       svanbot play assign <bot> <kind> <style>
       svanbot play revert <bot>
       svanbot play status";

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
        [name, bot] if name == "revert" => say(play::revert(&fleet, bot)),
        [name] if name == "status" => {
            print!("{}", play::status(&fleet).map_err(|error| (2, error))?);
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
    let mut opponents = Opponent::all();
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
    let settings = settings(args)?;
    let champion = fleet.champion().map_err(|error| (2, error))?;
    let reference = Policy::reference(champion);
    let candidate = Policy::new(kind, style, champion);
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
    println!("{heads_up}\n{six_max}");
    fleet
        .record_check(&Check {
            kind,
            style: style.name.into(),
            reference: champion.digest(),
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

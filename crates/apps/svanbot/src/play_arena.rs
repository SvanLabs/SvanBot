//! The policy Arena, with stablemate rows and its own fleet history.

use std::process::ExitCode;

use svanbot_runtime::fleet::{Fleet, League};
use svanbot_strategy::kind::PolicyKind;
use svanbot_strategy::policy::Policy;
use svanbot_strategy::style;

use crate::arena::{self, Opponent};
use crate::deciders;

const USAGE: &str = "usage: svanbot play arena <solving|pricing> <standard|tight|loose> [league] <bouts> [seed] [iterations] [out.json]";

pub(crate) fn run(args: &[String]) -> ExitCode {
    match play_arena(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, error)) => {
            eprintln!("svanbot play: {error}");
            ExitCode::from(code)
        }
    }
}

fn play_arena(args: &[String]) -> Result<(), (u8, String)> {
    let [command, kind, style_name, rest @ ..] = args else {
        return Err((2, USAGE.into()));
    };
    let (Some(kind), Some(style)) = (PolicyKind::named(kind), style::named(style_name)) else {
        return Err((2, USAGE.into()));
    };
    if command != "arena" {
        return Err((2, USAGE.into()));
    }

    let (league, args) = match rest.first().and_then(|name| League::named(name)) {
        Some(league) => (league, &rest[1..]),
        None => (League::HeadsUp, rest),
    };
    let number = |at: usize| args.get(at).map(|text| text.parse::<u64>());
    let (Some(Ok(bouts @ 1..)), Ok(seed), Ok(fixed)) =
        (number(0), number(1).transpose(), number(2).transpose())
    else {
        return Err((2, USAGE.into()));
    };
    if args.len() > 4 || bouts > u64::MAX / svanbot_learn::arena::BOUT {
        return Err((2, USAGE.into()));
    }
    let fleet = Fleet::in_dir(crate::fleet_dir());
    let champion = fleet.champion().map_err(|error| (2, error))?;
    let policy = Policy::new(kind, style, champion);
    let game = deciders::game(seed.unwrap_or(1));
    let mut opponents = Opponent::all();
    opponents.extend(style::ALL.map(Opponent::Stablemate));
    let rows = arena::play_rows(&opponents, Opponent::name, bouts, |opponent, number| {
        arena::play_bout(&game, league, opponent, number, &policy, fixed)
    });
    let name = format!("{}@{}", policy.name(), champion.digest());
    let run = arena::report(league, name, game.seed, fixed, rows);
    fleet
        .record_policy_arena(&run)
        .map_err(|error| (1, error))?;
    if let Some(path) = args.get(3) {
        std::fs::write(path, run.as_text() + "\n")
            .map_err(|error| (1, format!("{path} was not written: {error}")))?;
    }
    Ok(())
}

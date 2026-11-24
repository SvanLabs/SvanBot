//! `svanbot paired compare <base.json> <next.json>`: one build against another (V11-39). Each
//! file is a run `svanbot arena` wrote, and the two must be the same bouts: one league, one
//! seed, the same iterations and the same opponents with the same number of bouts. Every bout
//! then dealt both builds the same cards with the same seeds, so `next` less `base`, bout by
//! bout, is a paired verdict ([`compare`]) without either build in this binary.
//!
//! It prints a line a row. It exits 1 when any row is wholly below zero, 2 when the runs cannot
//! be read or are not the same bouts, and 0 otherwise.

use std::process::ExitCode;

use svanbot_learn::arena::compare;
use svanbot_runtime::fleet::ArenaRun;

pub(crate) fn run(args: &[String]) -> ExitCode {
    let [base, next] = args else {
        eprintln!("svanbot paired compare: usage: svanbot paired compare <base.json> <next.json>");
        return ExitCode::from(2);
    };
    let read = |path: &String| {
        std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|text| ArenaRun::from_text(&text))
            .map_err(|why| format!("{path} is not an Arena run: {why}"))
    };
    let lines = read(base)
        .and_then(|base| Ok((base, read(next)?)))
        .and_then(|(base, next)| rows(&base, &next));
    match lines {
        Ok(lines) => {
            let mut worse = false;
            for (line, below) in lines {
                println!("{line}");
                worse |= below;
            }
            if worse {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(why) => {
            eprintln!("svanbot paired compare: refused: {why}");
            ExitCode::from(2)
        }
    }
}

/// Each row's line, and whether it is wholly below zero; or why the runs are not the same bouts.
fn rows(base: &ArenaRun, next: &ArenaRun) -> Result<Vec<(String, bool)>, String> {
    if base.champion.is_none() || next.champion.is_none() {
        return Err("both runs must have a recorded champion".into());
    }
    if base.seed != next.seed {
        return Err(format!(
            "the runs were played from different seeds ({} and {})",
            base.seed, next.seed
        ));
    }
    if base.league != next.league {
        return Err("the runs are of different leagues".into());
    }
    if base.iterations != next.iterations {
        return Err("the runs re-solved with different iterations".into());
    }
    if base.rows.len() != next.rows.len() {
        return Err("the runs have different rows".into());
    }
    base.rows
        .iter()
        .zip(&next.rows)
        .map(|(base_row, next_row)| {
            if base_row.opponent != next_row.opponent {
                return Err(format!(
                    "the runs' rows differ: {} against {}",
                    base_row.opponent, next_row.opponent
                ));
            }
            let found = compare(&base_row.bouts, &next_row.bouts)
                .ok_or_else(|| format!("the {} rows are not the same bouts", base_row.opponent))?;
            Ok((
                format!(
                    "{} {}: {:+.0} mbb/100, 95% CI ±{:.0}, over {} deals",
                    base.league.name(),
                    base_row.opponent,
                    found.mbb_per_100,
                    found.ci95,
                    found.deals
                ),
                found.below_zero(),
            ))
        })
        .collect()
}

//! `svanbot play promote <bot>`: a bot's waiting search candidate played through the designated
//! gauntlet, then put forward for the bot's own live guard (V11-77, V11-82).
//!
//! The fresh 6-max confirmation is the one the search already kept ([`Candidate::verdict`]).
//! The gauntlet is played here: the challenger and the incumbent behind the same cards against
//! every designated opponent, bout by bout, read through the same anytime-valid sequence the
//! margin rule uses. Positive means the challenger won.
//!
//! Nothing is written unless the gate admits the whole evidence; a proposal that is admitted
//! waits for the bot, and its guard starts only where the operator enabled the live path
//! (`SVANBOT_PER_BOT_PROMOTION=yes`).

use std::process::ExitCode;

use svanbot::learn::{GUARD_HANDS, GUARD_MARGIN};
use svanbot_learn::arena::BOUT;
use svanbot_learn::margin::read;
use svanbot_learn::paired::Verdict;
use svanbot_learn::promotion::{DESIGNATED, GAUNTLET_MARGIN, Gate, GauntletRow, gate};
use svanbot_runtime::fleet::{Fleet, League, Promotion};
use svanbot_strategy::policy::Policy;
use svanbot_strategy::style;

use crate::arena::{self, Opponent};
use crate::deciders;

/// Plays `bot`'s waiting candidate through the designated gauntlet and puts it forward when the
/// gate admits the whole evidence. `tail` is `[bouts] [seed] [iterations]`, as the Arena reads
/// them; the sequence is read against the gate's own 5 bb/100 row bound.
pub(super) fn promote(fleet: &Fleet, bot: &str, tail: &[String]) -> super::CommandResult {
    let settings = super::settings(tail)?;
    let slot = fleet
        .champion_slot(bot)
        .map_err(|error| (2, error))?
        .ok_or_else(|| (2, format!("{bot} has no champion slot")))?;
    let search = fleet.bot_search(bot, &slot).map_err(|error| (2, error))?;
    let candidate = search.candidate.ok_or_else(|| {
        (
            2,
            format!("{bot} has no candidate waiting for its own probe"),
        )
    })?;
    let style = style::named(candidate.style).ok_or_else(|| {
        (
            2,
            format!("the candidate names unknown style {:?}", candidate.style),
        )
    })?;
    if !candidate.verdict.above_zero() {
        println!(
            "{bot}: the fresh confirmation is not above the incumbent: {:+.0} mbb/100, 95% ±{:.0}; nothing proposed",
            candidate.verdict.mbb_per_100, candidate.verdict.ci95
        );
        return Ok(ExitCode::from(1));
    }

    let incumbent = Policy::new(candidate.kind, style, slot);
    let challenger = Policy::new(candidate.kind, style, candidate.challenger);
    println!(
        "{bot}: {} against its slot, 6-max, {} bout(s), seed {}, {}",
        challenger.name(),
        settings.bouts,
        settings.seed,
        settings.fixed.map_or_else(
            || "live budget".into(),
            |n| format!("fixed iterations: {n}")
        ),
    );
    let game = deciders::game(settings.seed);
    let shared = Opponent::shared();
    let unit = 100_000.0 / BOUT as f64;
    let designated: Vec<Opponent> = DESIGNATED
        .iter()
        .map(|name| {
            shared
                .iter()
                .copied()
                .find(|opponent| opponent.name() == *name)
                .ok_or_else(|| (1, format!("the gauntlet has no {name} opponent")))
        })
        .collect::<Result<_, _>>()?;
    // Any idle worker takes the next row-bout: the challenger and the incumbent play the same
    // cards, and the per-deal difference is what one bout sums.
    let played = arena::play_rows(
        &designated,
        Opponent::name,
        settings.bouts,
        |opponent, number| {
            let next = arena::play_bout(
                &game,
                League::SixMax,
                opponent,
                number,
                &challenger,
                settings.fixed,
            );
            let base = arena::play_bout(
                &game,
                League::SixMax,
                opponent,
                number,
                &incumbent,
                settings.fixed,
            );
            next.iter()
                .zip(&base)
                .map(|(next, base)| (next - base) * unit)
                .collect()
        },
    );
    let mut rows = Vec::with_capacity(played.len());
    for (row, differences) in played {
        let reading = read(&differences, GAUNTLET_MARGIN);
        println!(
            "{:>16} {:+.0} mbb/100 ±{:.0} over {} bout(s)",
            row.opponent, reading.mean, reading.radius, reading.bouts
        );
        rows.push(GauntletRow {
            opponent: row.opponent,
            verdict: Verdict {
                deals: differences.len() * BOUT as usize,
                mbb_per_100: reading.mean,
                ci95: reading.radius,
            },
        });
    }

    let promotion =
        Promotion::from_candidate(bot, slot, &candidate, rows, GUARD_HANDS, GUARD_MARGIN);
    match gate(&promotion.evidence) {
        Gate::Admitted => {}
        Gate::Refused(why) => {
            println!(
                "{bot}: the gauntlet refuses the promotion: {}",
                why.join("; ")
            );
            return Ok(ExitCode::from(1));
        }
        Gate::Incomplete(why) => {
            println!("{bot}: the gauntlet is incomplete: {}", why.join("; "));
            return Ok(ExitCode::from(1));
        }
    }
    match fleet.propose_bot(&promotion) {
        Ok(true) => {
            println!(
                "{bot}: promotion of {} proposed; its guard starts on the bot's own hands where the live path is enabled",
                candidate.challenger.digest()
            );
            Ok(ExitCode::SUCCESS)
        }
        Ok(false) => {
            println!("{bot} already has a promotion in flight; nothing proposed");
            Ok(ExitCode::from(1))
        }
        Err(why) => Err((2, why)),
    }
}

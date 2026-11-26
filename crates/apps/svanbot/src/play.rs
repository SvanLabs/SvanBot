//! Policy checks and assignments.

use std::fmt::Write as _;
use std::path::Path;

use svanbot_learn::probe::Verdict;
use svanbot_learn::refresh::Refresh;
use svanbot_runtime::fleet::{Assignment, Fleet};
use svanbot_strategy::{
    kind::PolicyKind,
    params::Params,
    style::{self, Style},
};

pub mod guard;
mod judge;
pub use judge::{check_bar, check_readings, judge};

pub fn assign(
    fleet: &Fleet,
    data_dir: &Path,
    bot: &str,
    kind: PolicyKind,
    style: &'static Style,
    now_ms: i64,
) -> Result<String, String> {
    let style =
        style::named(style.name).ok_or_else(|| format!("unknown style {:?}", style.name))?;
    if kind == PolicyKind::Solving && style.name == style::STANDARD.name {
        return revert(fleet, bot);
    }
    if Path::new(bot).file_name().and_then(|name| name.to_str()) != Some(bot)
        || !data_dir.join(format!("{bot}.db")).is_file()
    {
        return Err(format!("{bot} has no bot store"));
    }
    let policy = format!("{}/{}", kind.name(), style.name);
    let check = fleet
        .checks()?
        .into_iter()
        .find(|check| check.kind == kind && check.style == style.name)
        .ok_or_else(|| format!("{policy} has no margin check"))?;
    if !check.admitted {
        return Err(format!("{policy} was not admitted by its margin check"));
    }
    if check.reference != fleet.champion()?.digest() {
        return Err(format!("{policy} has a stale margin check"));
    }
    fleet.assign(bot, Assignment { kind, style }, now_ms)?;
    Ok(format!("{bot} assigned {policy}"))
}

pub fn revert(fleet: &Fleet, bot: &str) -> Result<String, String> {
    fleet.revert(bot)?;
    Ok(format!("{bot} will play the reference policy"))
}

/// Seeds `bot`'s champion slot from the fleet champion, under the style it is assigned. It is
/// the first slice of Phase 2: the numbers are the reference's, so a bot that is seeded plays
/// exactly as it did, and its own search and promotion move the slot from there.
pub fn seed(fleet: &Fleet, data_dir: &Path, bot: &str) -> Result<String, String> {
    if Path::new(bot).file_name().and_then(|name| name.to_str()) != Some(bot)
        || !data_dir.join(format!("{bot}.db")).is_file()
    {
        return Err(format!("{bot} has no bot store"));
    }
    let champion = fleet.seed_champion(bot)?;
    Ok(format!(
        "{bot}'s champion slot seeded from the reference at {}",
        champion.digest()
    ))
}

/// Puts a refresh's selected champion in `bot`'s slot, but only once the live guard cleared it.
/// The guard is the same one every promotion runs (`probe::Kind::Guard`), and `Verdict::Cleared`
/// is its only pass: any other verdict leaves the incumbent in the slot. `guard` is the verdict
/// of the guard run for this refresh.
pub fn apply_refresh(
    fleet: &Fleet,
    refresh: &Refresh,
    guard: Verdict,
) -> Result<Option<Params>, String> {
    let Some(champion) = refresh.through_guard(guard) else {
        return Ok(None);
    };
    fleet.set_champion(&refresh.bot, &champion)?;
    Ok(Some(champion))
}

pub fn status(fleet: &Fleet) -> Result<String, String> {
    let champion = fleet.champion()?.digest();
    let mut text = format!("Reference: solving/standard@{champion}\nAssignments:\n");
    let assignments = fleet.assignments()?;
    if assignments.is_empty() {
        text.push_str("  none\n");
    }
    for (bot, assignment, since) in assignments {
        let _ = writeln!(
            text,
            "  {bot}: {}/{} since {since}",
            assignment.kind.name(),
            assignment.style.name
        );
    }
    text.push_str("Checks:\n");
    let checks = fleet.checks()?;
    if checks.is_empty() {
        text.push_str("  none\n");
    }
    for check in checks {
        let result = if check.admitted {
            "admitted"
        } else {
            "not admitted"
        };
        let stale = if check.reference != champion {
            " (stale)"
        } else {
            ""
        };
        let _ = writeln!(
            text,
            "  {}/{}: {result}{stale} against {}\n    {}\n    {}",
            check.kind.name(),
            check.style,
            check.reference,
            check.heads_up,
            check.six_max
        );
    }
    Ok(text)
}

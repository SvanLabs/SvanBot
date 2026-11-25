//! Policy checks and assignments.

use std::fmt::Write as _;
use std::path::Path;

use svanbot_runtime::fleet::{Assignment, Fleet};
use svanbot_strategy::{
    kind::PolicyKind,
    style::{self, Style},
};

pub mod guard;
mod judge;
pub use judge::judge;

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

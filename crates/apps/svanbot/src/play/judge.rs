use svanbot_learn::margin::{self, Admission, Reading};
use svanbot_learn::play::STABLEMATE;
use svanbot_runtime::fleet::{ArenaRow, ArenaRun};

pub fn judge(
    reference: &ArenaRun,
    candidate: &ArenaRun,
    margin: f64,
) -> Result<(Reading, String), String> {
    if reference.league != candidate.league {
        return Err("the runs use a different league".into());
    }
    if reference.seed != candidate.seed {
        return Err("the runs use a different seed".into());
    }
    if reference.iterations != candidate.iterations {
        return Err("the runs use different iterations".into());
    }
    let base: Vec<_> = standard(reference).collect();
    let next: Vec<_> = standard(candidate).collect();
    if base.len() != next.len()
        || base
            .iter()
            .zip(&next)
            .any(|(a, b)| a.opponent != b.opponent)
    {
        return Err("the runs have different standard rows".into());
    }
    let base: Vec<_> = base.iter().map(|row| row.bouts.as_slice()).collect();
    let next: Vec<_> = next.iter().map(|row| row.bouts.as_slice()).collect();
    let differences =
        margin::pooled(&base, &next).ok_or("the runs do not have the same nonempty bouts")?;
    let reading = margin::read(&differences, margin);
    let admission = match reading.admission {
        Admission::Admitted => "admitted",
        Admission::Refused => "refused",
        Admission::Unresolved => "unresolved",
    };
    let line = format!(
        "{}: {:+.0} mbb/100 ±{:.0} over {} bouts: {admission}",
        reference.league.name(),
        reading.mean,
        reading.radius,
        reading.bouts,
    );
    Ok((reading, line))
}

fn standard(run: &ArenaRun) -> impl Iterator<Item = &ArenaRow> {
    run.rows
        .iter()
        .filter(|row| !row.opponent.starts_with(STABLEMATE))
}

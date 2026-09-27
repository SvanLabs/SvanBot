//! Background jobs (0222): run a job body on the blocking pool so store reads and long
//! computations never stall the async workers that carry the table connections, and log a
//! panicking job by name instead of dropping its `JoinError` (LESSONS 17, 20).

/// Run `body` on the blocking pool. `None` means the job panicked; the panic is logged with
/// the job's name, and the calling loop carries on with its next round.
pub async fn blocking<T: Send + 'static>(name: &'static str, body: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let started = std::time::Instant::now();
    let result = tokio::task::spawn_blocking(body).await;
    if let Some(line) = over_budget(name, started.elapsed()) {
        tracing::warn!("{line}");
    }
    match result {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::error!("background job {name} failed: {e}");
            None
        }
    }
}

/// The longest any job may run on the live system: two minutes is already a long stall for a bot
/// holding a seat at a table. Backups are the exception — they are bulk I/O and take as long as
/// they take.
pub const JOB_BUDGET: std::time::Duration = std::time::Duration::from_secs(120);

/// Jobs allowed past [`JOB_BUDGET`]: the backups.
const EXEMPT: [&str; 1] = ["database backup"];

/// The warning for a job that ran past [`JOB_BUDGET`], naming it, so an overrun is found in the log
/// instead of felt as a slow machine (0334). `None` within budget or for an exempt job.
pub fn over_budget(name: &str, took: std::time::Duration) -> Option<String> {
    (took > JOB_BUDGET && !EXEMPT.contains(&name)).then(|| {
        format!(
            "background job {name} took {:.0} s, over the {} s budget for live-system work (0334)",
            took.as_secs_f64(),
            JOB_BUDGET.as_secs()
        )
    })
}

#[cfg(test)]
mod tests {
    /// 0334: a job past two minutes is named in the log; the backup is allowed to take its time.
    #[test]
    fn a_job_over_two_minutes_is_named_and_the_backup_is_exempt() {
        use std::time::Duration;
        assert_eq!(super::over_budget("history import", Duration::from_secs(90)), None);
        let line = super::over_budget("history import", Duration::from_secs(185)).unwrap();
        assert!(line.contains("history import took 185 s") && line.contains("120 s"), "{line}");
        assert_eq!(super::over_budget("database backup", Duration::from_secs(600)), None, "backups take their time");
    }

    #[tokio::test]
    async fn a_panicking_job_returns_none_and_the_next_round_runs() {
        assert_eq!(super::blocking("boom", || -> u8 { panic!("job body panicked") }).await, None);
        assert_eq!(super::blocking("fine", || 7).await, Some(7));
    }
}

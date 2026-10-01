//! Autonomy watchdog (0222): the fleet is meant to run with nobody watching, so every loop that
//! keeps it learning must announce when it stops. The results monitor watches the bots (stalls,
//! errors, nemeses); this watches the loops behind them: the learner's cycles, the analyst's
//! re-solves, the hourly fold calibration and the hourly backup with its integrity check.
//!
//! Each loop leaves a timestamp in the store. [`stale_loops`] compares each with its limit (a
//! little over the loop's own cadence); the fleet head checks every 10 minutes and logs once when a
//! loop goes stale and once when it recovers, so a silent stop reaches the dashboard's activity log.

use serde_json::Value;

/// One watched loop: where its heartbeat lives and how old it may get.
#[derive(Clone, Copy, Debug)]
pub struct WatchedLoop {
    /// Short name for the log line.
    pub name: &'static str,
    /// Store key holding the loop's status JSON.
    pub key: &'static str,
    /// JSON pointer to the Unix-seconds heartbeat inside it.
    pub pointer: &'static str,
    /// Oldest acceptable heartbeat, in seconds.
    pub limit_secs: f64,
    /// What a stale heartbeat means, for the log line.
    pub meaning: &'static str,
}

/// The watched loops; the learner's limit follows its own idle bound ([`crate::pacing::Pacing`]).
pub fn loops(learner_max_idle_secs: f64) -> [WatchedLoop; 5] {
    [
        WatchedLoop {
            name: "learner",
            key: "learner.pacing",
            pointer: "/last_run",
            limit_secs: learner_max_idle_secs + 2.0 * 3600.0,
            meaning: "no scheduled champion search has run",
        },
        WatchedLoop {
            name: "analyst",
            key: "analyst.status",
            pointer: "/updated",
            limit_secs: 3600.0,
            meaning: "big decisions are not being re-solved or graded",
        },
        WatchedLoop {
            name: "fold calibration",
            key: crate::foldcal::FOLD_CAL_KEY,
            pointer: "/fitted_at",
            limit_secs: 3.0 * 3600.0,
            meaning: "fold estimates are not being recalibrated to live results",
        },
        WatchedLoop {
            name: "backups",
            key: crate::tasks::INTEGRITY_STATUS_KEY,
            pointer: "/checked_at",
            limit_secs: 3.0 * 3600.0,
            meaning: "no hourly backup or integrity check has completed",
        },
        WatchedLoop {
            name: "experiment poller",
            // The poller's own mode record is its heartbeat (`/last_attempt_at` moves every poll,
            // succeeded or failed), so a dead poller is visible without a second key: the mode
            // would otherwise sit `Active` on a reading nobody refreshes (0327).
            key: crate::experiment::MODE_KEY,
            pointer: "/last_attempt_at",
            limit_secs: 6.0 * crate::experiment::mode::POLL_SECS as f64,
            meaning: "the leaderboard is not being read, so the experiment mode cannot advance or end",
        },
    ]
}

/// A loop whose heartbeat is missing, unreadable or too old.
#[derive(Clone, Debug, PartialEq)]
pub struct Stale {
    /// The loop's name.
    pub name: &'static str,
    /// The log line.
    pub message: String,
}

/// Which of `loops` are stale at `now` (Unix seconds). `read` fetches a store key; a read error or
/// a missing or malformed heartbeat counts as stale, since the loop cannot be shown to be alive.
/// The learner is skipped while the operator has paused it (`learner.status` `automatic: false`).
pub fn stale_loops(now: f64, loops: &[WatchedLoop], read: impl Fn(&str) -> anyhow::Result<Option<String>>) -> Vec<Stale> {
    let learner_paused = read("learner.status")
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str::<Value>(&j).ok())
        .is_some_and(|v| v["automatic"] == Value::Bool(false));
    let settings = read(crate::pacing::SETTINGS_KEY).ok().flatten();
    let pacing = crate::pacing::Pacing::from_env().with_settings(&crate::pacing::LearnerSettings::parse(settings.as_deref()));
    loops
        .iter()
        .filter(|l| !(l.name == "learner" && learner_paused))
        .filter_map(|l| {
            let status = read(l.key);
            let mut watched = *l;
            if l.name == "learner"
                && let Some(v) = status.as_ref().ok().and_then(|j| j.as_deref()).and_then(|j| serde_json::from_str::<Value>(j).ok())
                && let Some(start) = v["last_run"].as_f64().filter(|t| *t > 0.0)
            {
                // Cooldown starts when the search finishes. Fresh refits do not renew this
                // deadline: a stopped champion search must still become visible.
                let end = v["last_end"].as_f64().unwrap_or(start).max(start);
                watched.limit_secs = watched.limit_secs.max(end - start + pacing.cooldown_secs + 2.0 * 3600.0);
            }
            staleness(now, &watched, status).map(|message| Stale { name: l.name, message })
        })
        .collect()
}

/// Why one loop is stale, given the read of its status key; `None` while it is alive.
fn staleness(now: f64, l: &WatchedLoop, status: anyhow::Result<Option<String>>) -> Option<String> {
    let json = match status {
        Err(e) => return Some(format!("{} status unreadable ({e}): {}", l.name, l.meaning)),
        Ok(None) => return Some(format!("{} has never reported: {}", l.name, l.meaning)),
        Ok(Some(j)) => j,
    };
    let heartbeat = serde_json::from_str::<Value>(&json).ok().and_then(|v| v.pointer(l.pointer).and_then(Value::as_f64));
    match heartbeat {
        None => Some(format!("{} status has no heartbeat: {}", l.name, l.meaning)),
        Some(t) if now - t > l.limit_secs => {
            Some(format!("{} last reported {:.1} h ago (limit {:.1} h): {}", l.name, (now - t) / 3600.0, l.limit_secs / 3600.0, l.meaning))
        }
        Some(_) => None,
    }
}

/// Log lines for a change from the stale set `before` to `now`: one warning per newly stale loop,
/// one info line per recovered loop, nothing for a loop that stays stale.
pub fn transitions(before: &[Stale], now: &[Stale]) -> Vec<(&'static str, String)> {
    let was = |n: &str| before.iter().any(|s| s.name == n);
    let is = |n: &str| now.iter().any(|s| s.name == n);
    let mut out: Vec<(&'static str, String)> =
        now.iter().filter(|s| !was(s.name)).map(|s| ("warn", format!("autonomy: {}", s.message))).collect();
    out.extend(before.iter().filter(|s| !is(s.name)).map(|s| ("info", format!("autonomy: {} is reporting again", s.name))));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const NOW: f64 = 1_790_380_000.0;

    fn store(entries: &[(&str, &str)]) -> impl Fn(&str) -> anyhow::Result<Option<String>> {
        let m: HashMap<String, String> = entries.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k: &str| Ok(m.get(k).cloned())
    }

    fn healthy() -> Vec<(&'static str, String)> {
        vec![
            ("learner.pacing", format!(r#"{{"last_run":{}}}"#, NOW - 3600.0)),
            ("analyst.status", format!(r#"{{"updated":{}}}"#, NOW - 60.0)),
            (crate::foldcal::FOLD_CAL_KEY, format!(r#"{{"fitted_at":{}}}"#, NOW - 1800.0)),
            (crate::tasks::INTEGRITY_STATUS_KEY, format!(r#"{{"checked_at":{}}}"#, NOW - 600.0)),
            (crate::experiment::MODE_KEY, format!(r#"{{"last_attempt_at":{}}}"#, NOW - 60.0)),
        ]
    }

    fn with(overrides: &[(&'static str, String)]) -> Vec<(&'static str, String)> {
        let mut v = healthy();
        for (k, val) in overrides {
            v.retain(|(key, _)| key != k);
            v.push((k, val.clone()));
        }
        v
    }

    fn check(entries: &[(&'static str, String)]) -> Vec<Stale> {
        let e: Vec<(&str, &str)> = entries.iter().map(|(k, v)| (*k, v.as_str())).collect();
        stale_loops(NOW, &loops(6.0 * 3600.0), store(&e))
    }

    /// 0327: the poller that reads the official leaderboard must keep reporting even when
    /// supervision is retrying it. Its mode record is the heartbeat; a stalled or repeatedly
    /// failing poller is still named rather than hidden by automatic retries.
    #[test]
    fn a_stalled_leaderboard_poller_is_named() {
        let silent = with(&[(crate::experiment::MODE_KEY, format!(r#"{{"status":"active","last_attempt_at":{}}}"#, NOW - 31.0 * 60.0))]);
        let stale = check(&silent);
        assert_eq!(stale.len(), 1, "{stale:?}");
        assert_eq!(stale[0].name, "experiment poller");
        assert!(stale[0].message.contains("0.5 h ago (limit 0.5 h)"), "{}", stale[0].message);
    }

    #[test]
    fn scheduled_cooldown_is_healthy_but_refits_do_not_hide_an_overdue_search() {
        let mut entries = with(&[
            (crate::pacing::SETTINGS_KEY, r#"{"cooldown_minutes":1440}"#.into()),
            (
                "learner.pacing",
                format!(r#"{{"last_run":{},"last_end":{},"refit_run":{}}}"#, NOW - 25.0 * 3600.0, NOW - 23.0 * 3600.0, NOW - 60.0),
            ),
        ]);
        assert!(check(&entries).is_empty(), "cooldown starts at search completion");
        entries.retain(|(k, _)| *k != "learner.pacing");
        entries.push((
            "learner.pacing",
            format!(r#"{{"last_run":{},"last_end":{},"refit_run":{}}}"#, NOW - 29.0 * 3600.0, NOW - 27.0 * 3600.0, NOW - 60.0),
        ));
        assert_eq!(check(&entries).iter().map(|s| s.name).collect::<Vec<_>>(), ["learner"]);
    }

    #[test]
    fn healthy_loops_are_quiet_and_a_stopped_learner_is_named() {
        assert!(check(&healthy()).is_empty());
        let stale = check(&with(&[("learner.pacing", format!(r#"{{"last_run":{}}}"#, NOW - 9.0 * 3600.0))]));
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].name, "learner");
        assert!(stale[0].message.contains("9.0 h ago (limit 8.0 h)"), "{}", stale[0].message);
    }

    #[test]
    fn a_paused_learner_is_not_an_alarm_but_missing_or_malformed_status_is() {
        let paused = with(&[
            ("learner.pacing", format!(r#"{{"last_run":{}}}"#, NOW - 99.0 * 3600.0)),
            ("learner.status", r#"{"automatic":false}"#.to_string()),
        ]);
        assert!(check(&paused).is_empty());
        let mut missing = healthy();
        missing.retain(|(k, _)| *k != "analyst.status" && *k != crate::foldcal::FOLD_CAL_KEY);
        missing.push((crate::foldcal::FOLD_CAL_KEY, "not json".to_string()));
        let names: Vec<&str> = check(&missing).iter().map(|s| s.name).collect();
        assert_eq!(names, ["analyst", "fold calibration"]);
    }

    #[test]
    fn a_read_error_is_stale_and_transitions_log_once_each_way() {
        let failing = |_: &str| -> anyhow::Result<Option<String>> { anyhow::bail!("database is locked") };
        let stale = stale_loops(NOW, &loops(6.0 * 3600.0), failing);
        assert_eq!(stale.len(), 5, "every loop unprovable");
        assert!(stale[0].message.contains("unreadable (database is locked)"));
        let first = transitions(&[], &stale[..1]);
        assert_eq!(first, [("warn", format!("autonomy: {}", stale[0].message))]);
        assert!(transitions(&stale[..1], &stale[..1]).is_empty(), "still stale: no repeat");
        assert_eq!(transitions(&stale[..1], &[]), [("info", "autonomy: learner is reporting again".to_string())]);
    }
}

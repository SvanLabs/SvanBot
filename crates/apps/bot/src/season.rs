//! The season now being played, from `GET /season/current`: its clock and its identity.
//!
//! The clock's job is table moves: the last minutes of a season start no new hands (wind-down), so
//! leaving a table then — to top up, seek top bots or dodge a tough table — can only cost the
//! seat, never gain a hand.
//!
//! The identity's job is scoping. A season is a separate contest with its own leaderboard, so a
//! panel that presents *this season's* performance reads only hands played since
//! [`CurrentSeason::started_at`]. Accumulated learning — opponent models, calibration, the fitted
//! range model, the learner's champion — deliberately carries across seasons and is never scoped.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};

/// No voluntary table moves this close to the season end (wind-down itself is the last 5 minutes;
/// a re-queue needs time to seat and play enough hands to matter).
pub const FREEZE_BEFORE_END: Duration = Duration::from_secs(15 * 60);
/// The server's wind-down: the last 5 minutes start no new hands, so tables fall silent.
pub const WIND_DOWN: Duration = Duration::from_secs(5 * 60);
/// Silence is expected from wind-down until this long past the projected end (the season-12 end
/// took 30 s from `table_closed` to the next table; the margin covers a late clock).
const QUIET_AFTER_END: Duration = Duration::from_secs(10 * 60);
/// A clock reading older than this is ignored (moves allowed) rather than trusted.
const STALE_AFTER: Duration = Duration::from_secs(60 * 60);

#[derive(Clone, Debug, Default)]
pub struct SeasonClock {
    pub winding_down: bool,
    pub seconds_left: Option<f64>,
    pub fetched: Option<Instant>,
}

impl SeasonClock {
    pub fn from_current(v: &Value, now: Instant) -> Option<Self> {
        let winding_down = v.get("winding_down")?.as_bool().unwrap_or(false);
        Some(Self { winding_down, seconds_left: v["time_remaining_seconds"].as_f64(), fetched: Some(now) })
    }

    /// Whether a bot may leave its table voluntarily now.
    pub fn table_moves_allowed(&self, now: Instant) -> bool {
        let Some(fetched) = self.fetched else { return true };
        let age = now.saturating_duration_since(fetched);
        if age > STALE_AFTER {
            return true;
        }
        if self.winding_down {
            return false;
        }
        match self.seconds_left {
            Some(left) => left - age.as_secs_f64() > FREEZE_BEFORE_END.as_secs_f64(),
            None => true,
        }
    }
}

impl SeasonClock {
    /// Whether table silence is expected now (wind-down up to shortly after the end), so the
    /// no-traffic reconnect watchdog stays quiet (0143). A stale or unknown reading never
    /// suppresses it, and neither does a clock far past the end that has not been refreshed.
    pub fn silence_expected(&self, now: Instant) -> bool {
        let Some(fetched) = self.fetched else { return false };
        let age = now.saturating_duration_since(fetched);
        if age > STALE_AFTER {
            return false;
        }
        let projected = self.seconds_left.map(|left| left - age.as_secs_f64());
        let before_end_window = projected.is_some_and(|p| p <= WIND_DOWN.as_secs_f64() && p > -QUIET_AFTER_END.as_secs_f64());
        (self.winding_down && projected.is_none_or(|p| p > -QUIET_AFTER_END.as_secs_f64())) || before_end_window
    }
}

/// Identity and start of the season now being played.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CurrentSeason {
    /// Season number (13, …) when the server reported one.
    pub number: Option<i64>,
    /// Season id, as the leaderboard and history export use it.
    pub id: Option<String>,
    /// Season start in unix seconds: the boundary stored hands are scoped by.
    pub started_at: f64,
}

impl CurrentSeason {
    /// Read `GET /season/current`. `None` when the response carries no usable start time, so
    /// nothing ever scopes on a guessed boundary.
    pub fn from_current(v: &Value) -> Option<Self> {
        Some(Self {
            number: v["season_number"].as_i64(),
            id: v["season_id"].as_str().map(str::to_owned),
            started_at: rfc3339_secs(v["start_date"].as_str()?)?,
        })
    }

    /// Whether a hand that ended at `ts` (unix seconds) belongs to this season.
    pub fn contains(&self, ts: f64) -> bool {
        ts >= self.started_at
    }
}

/// An RFC 3339 timestamp as unix seconds.
pub fn rfc3339_secs(s: &str) -> Option<f64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.timestamp_millis() as f64 / 1000.0)
}

/// A stored hand's `ended_at` as unix seconds; an unreadable stamp reads as 0, which keeps it
/// out of the current season rather than inflating it.
pub fn ended_at_secs(s: &str) -> f64 {
    rfc3339_secs(s).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn table_moves_freeze_near_the_season_end() {
        let t0 = Instant::now();
        let clock = |v: Value| SeasonClock::from_current(&v, t0).unwrap();
        assert!(SeasonClock::default().table_moves_allowed(t0), "unknown clock never blocks");
        assert!(clock(json!({"winding_down": false, "time_remaining_seconds": 86400.0})).table_moves_allowed(t0));
        assert!(!clock(json!({"winding_down": true, "time_remaining_seconds": 200.0})).table_moves_allowed(t0));
        assert!(!clock(json!({"winding_down": false, "time_remaining_seconds": 600.0})).table_moves_allowed(t0));
        // The reading ages: 20 minutes left at fetch, 10 minutes later only 10 remain.
        let c = clock(json!({"winding_down": false, "time_remaining_seconds": 1200.0}));
        assert!(c.table_moves_allowed(t0));
        assert!(!c.table_moves_allowed(t0 + Duration::from_secs(600)));
        assert!(c.table_moves_allowed(t0 + Duration::from_secs(7200)), "stale readings are ignored");
        assert!(SeasonClock::from_current(&json!({"detail": "error"}), t0).is_none());
    }

    #[test]
    fn silence_is_expected_only_around_the_season_end() {
        let t0 = Instant::now();
        let clock = |v: Value| SeasonClock::from_current(&v, t0).unwrap();
        assert!(!SeasonClock::default().silence_expected(t0), "unknown clock keeps the watchdog");
        assert!(!clock(json!({"winding_down": false, "time_remaining_seconds": 86400.0})).silence_expected(t0));
        assert!(!clock(json!({"winding_down": false, "time_remaining_seconds": 600.0})).silence_expected(t0), "hands still run 10 min out");
        // Season 12's end: wind-down flagged with 568 s left.
        let end = clock(json!({"winding_down": true, "time_remaining_seconds": 568.0}));
        assert!(end.silence_expected(t0));
        assert!(end.silence_expected(t0 + Duration::from_secs(600)), "just past the end, before the next season is polled");
        assert!(!end.silence_expected(t0 + Duration::from_secs(568 + 11 * 60)), "long past the end the watchdog returns");
        // An unflagged clock that projects into the last 5 minutes also expects silence.
        assert!(clock(json!({"winding_down": false, "time_remaining_seconds": 400.0})).silence_expected(t0 + Duration::from_secs(120)));
        assert!(!end.silence_expected(t0 + Duration::from_secs(7200)), "stale readings never suppress");
    }

    #[test]
    fn current_season_parses_identity_and_scopes_hands_by_its_start() {
        let v = json!({
            "season_id": "ac9f1eb2", "season_number": 13,
            "start_date": "2026-09-20T11:46:23.998128+00:00",
            "end_date": "2026-10-04T11:46:23.998128+00:00", "status": "active",
        });
        let season = CurrentSeason::from_current(&v).expect("a start date makes a season");
        assert_eq!((season.number, season.id.as_deref()), (Some(13), Some("ac9f1eb2")));
        // Season 12's opening hand and its last day are outside; season 13's first hand is inside.
        assert!(!season.contains(ended_at_secs("2026-09-14T10:14:00+00:00")));
        assert!(!season.contains(ended_at_secs("2026-09-20T11:45:54.065607+00:00")));
        assert!(season.contains(ended_at_secs("2026-09-20T13:46:00.123456789+00:00")));
        assert!(season.contains(season.started_at), "the boundary itself belongs to the new season");
        assert!(!season.contains(ended_at_secs("not a timestamp")), "an unreadable stamp never counts as this season");

        assert!(CurrentSeason::from_current(&json!({"season_number": 13})).is_none(), "no start date, no scoping");
        assert!(CurrentSeason::from_current(&json!({"start_date": "yesterday"})).is_none());
    }
}

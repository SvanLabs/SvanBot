//! When a bot parks on an authentication failure (#744 layer 1, signed off on #721). One refused login
//! used to end the session as `Fatal`, parking the seat until a human restarted it. Now fewer than
//! [`LIMIT`] failures in a row within [`WINDOW`] seconds take the ordinary backoff path and the seat
//! retries; the [`LIMIT`]th parks, as before. A rotation marker — the unix time an operator set under
//! [`ROTATION_KEY`] when the key was replaced — makes a failure in a session that began after it fatal
//! at once: the new key is the one failing, so retrying cannot help.

/// Consecutive failures that park the bot.
pub const LIMIT: usize = 3;
/// Seconds the failures must fall within to count as consecutive.
pub const WINDOW: f64 = 600.0;
/// Kv key holding the unix time of the last key rotation. Set by hand at rekey time:
/// `sqlite3 artifacts/svanbot10.db "insert or replace into kv(key, value) values ('auth.rotation', strftime('%s','now'))"`.
pub const ROTATION_KEY: &str = "auth.rotation";

/// What to do with an authentication failure.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Back off and try again; carries how many failures are on the books.
    Retry(usize),
    /// Stop and wait for a human.
    Park,
}

/// Recent authentication failures of one bot, as unix seconds.
#[derive(Default)]
pub struct AuthPark(Vec<f64>);

impl AuthPark {
    /// Record a failure at `now` in a session that began at `session_started`; `rotated` is the
    /// rotation marker, when one is set.
    pub fn failed(&mut self, now: f64, session_started: f64, rotated: Option<f64>) -> Verdict {
        if rotated.is_some_and(|marker| session_started >= marker) {
            return Verdict::Park;
        }
        self.0.retain(|t| now - t <= WINDOW);
        self.0.push(now);
        if self.0.len() >= LIMIT { Verdict::Park } else { Verdict::Retry(self.0.len()) }
    }

    /// Any other way a session ends breaks the run of failures.
    pub fn reset(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_third_failure_in_ten_minutes_parks_and_anything_else_resets_the_count() {
        let mut park = AuthPark::default();
        assert_eq!(park.failed(0.0, 0.0, None), Verdict::Retry(1));
        assert_eq!(park.failed(60.0, 60.0, None), Verdict::Retry(2));
        assert_eq!(park.failed(120.0, 120.0, None), Verdict::Park);
        park.reset();
        assert_eq!(park.failed(130.0, 130.0, None), Verdict::Retry(1), "a session that ended otherwise broke the run");
        // Failures further apart than the window do not add up.
        let mut slow = AuthPark::default();
        for (i, t) in [0.0, 400.0, 800.0, 1200.0].into_iter().enumerate() {
            assert_eq!(slow.failed(t, t, None), Verdict::Retry(if i == 0 { 1 } else { 2 }), "at {t}");
        }
    }

    #[test]
    fn a_failure_after_a_key_rotation_parks_at_once() {
        let mut park = AuthPark::default();
        assert_eq!(park.failed(1000.0, 900.0, Some(500.0)), Verdict::Park, "the session began after the rotation");
        let mut older = AuthPark::default();
        assert_eq!(older.failed(1000.0, 400.0, Some(500.0)), Verdict::Retry(1), "a session from before it is the old key's");
    }
}

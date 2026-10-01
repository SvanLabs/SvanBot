//! Bounded ordinary/control hand inputs for frozen learner stack fixtures.
use super::*;

impl Store {
    /// Latest `limit` ordinary/control hand rows ending at `upto`: hero seat and full summary.
    /// The cutoff and provenance filter are applied before the limit.
    pub fn stack_samples(&self, upto: i64, limit: usize) -> Result<Vec<(Option<i64>, String)>> {
        let conn = self.read();
        let mut st = conn.prepare(&format!(
            "SELECT hero_seat, COALESCE(summary, '') FROM hands WHERE rowid <= ?1 AND {} ORDER BY rowid DESC LIMIT ?2",
            ordinary_hand("hands.bot", "hands.hand_id")
        ))?;
        Ok(st
            .query_map(params![upto, limit as i64], |r| Ok((r.get(0)?, self.codec.text(r.get_ref(1)?)?)))?
            .collect::<Result<Vec<_>, _>>()?)
    }
}

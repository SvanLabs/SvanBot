//! Predicted-vs-realized calibration samples and their summaries.
use super::*;
use anyhow::{Result, ensure};
use rusqlite::params;

/// (category, count, mean predicted bb, mean realized bb, residual variance).
pub type CalibrationRow = (String, i64, f64, f64, f64);

impl Store {
    /// Predicted vs realized chips (relative to folding) for one decision, in big blinds.
    /// `scale_bb` is the pot the decision was sized against (pot + amount to call), in big blinds.
    pub fn insert_calibration(
        &self,
        bot: &str,
        hand_id: &str,
        category: &str,
        predicted_bb: f64,
        realized_bb: f64,
        scale_bb: f64,
    ) -> Result<()> {
        ensure!(predicted_bb.is_finite() && realized_bb.is_finite(), "calibration values must be finite");
        ensure!(scale_bb.is_finite() && scale_bb > 0.0, "calibration scale must be positive and finite");
        self.write_lock().execute(
            "INSERT INTO calibration (bot, hand_id, ts, category, predicted, realized, scale) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![bot, hand_id, chrono::Utc::now().to_rfc3339(), category, predicted_bb, realized_bb, scale_bb],
        )?;
        Ok(())
    }
    /// Per category: (count, mean predicted, mean realized, variance of the residual).
    pub fn calibration_summary(&self) -> Result<Vec<CalibrationRow>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT category, COUNT(*), AVG(predicted), AVG(realized),
                    AVG((realized - predicted) * (realized - predicted)) - AVG(realized - predicted) * AVG(realized - predicted)
             FROM calibration GROUP BY category",
        )?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, Option<f64>>(4)?.unwrap_or(0.0))))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Every sample recorded at or after `since` (RFC 3339) as (category, predicted bb, realized bb),
    /// oldest first.
    pub fn calibration_samples_since(&self, since: &str) -> Result<Vec<(String, f64, f64)>> {
        let conn = self.read();
        let mut stmt = conn.prepare("SELECT category, predicted, realized FROM calibration WHERE ts >= ?1 ORDER BY id")?;
        let rows = stmt.query_map([since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Per category, over samples predicted in `[lo, hi)` bb (the decision margin): (count, mean
    /// residual in bb, residual variance).
    pub fn calibration_margin_summary(&self, lo: f64, hi: f64) -> Result<Vec<(String, i64, f64, f64)>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT category, COUNT(*), AVG(realized - predicted),
                    AVG((realized - predicted) * (realized - predicted)) - AVG(realized - predicted) * AVG(realized - predicted)
             FROM calibration WHERE predicted >= ?1 AND predicted < ?2 GROUP BY category",
        )?;
        let rows = stmt
            .query_map([lo, hi], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<f64>>(2)?.unwrap_or(0.0), r.get::<_, Option<f64>>(3)?.unwrap_or(0.0)))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Per category, residuals as a fraction of the pot: (count, mean, variance) over rows with a known pot.
    pub fn calibration_pot_summary(&self) -> Result<Vec<(String, i64, f64, f64)>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT category, COUNT(*), AVG((realized - predicted) / scale),
                    AVG(((realized - predicted) / scale) * ((realized - predicted) / scale)) - AVG((realized - predicted) / scale) * AVG((realized - predicted) / scale)
             FROM calibration WHERE scale > 0 GROUP BY category",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<f64>>(2)?.unwrap_or(0.0), r.get::<_, Option<f64>>(3)?.unwrap_or(0.0)))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pot_relative_calibration_rejects_invalid_scale() {
        let dir = std::env::temp_dir().join(format!("sv10-store-cal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        // Residuals of -10 bb in a 100 bb pot and +2 bb in a 20 bb pot: -10% and +10%.
        store.insert_calibration("A", "h1", "river:call", 40.0, 30.0, 100.0).unwrap();
        store.insert_calibration("A", "h2", "river:call", 10.0, 12.0, 20.0).unwrap();
        assert!(store.insert_calibration("A", "h3", "river:call", 10.0, 0.0, -1.0).is_err());
        assert!(store.insert_calibration("A", "h4", "river:call", f64::NAN, 0.0, 20.0).is_err());
        let rows = store.calibration_pot_summary().unwrap();
        assert_eq!(rows.len(), 1);
        let (cat, n, mean, var) = &rows[0];
        assert_eq!((cat.as_str(), *n), ("river:call", 2));
        assert!(mean.abs() < 1e-9 && (var - 0.01).abs() < 1e-9, "{mean} {var}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

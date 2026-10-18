//! Predicted-vs-realized calibration samples and their summaries.
use super::*;
use anyhow::{Result, ensure};
use rusqlite::{named_params, params};

/// (category, count, mean predicted bb, mean realized bb, residual variance).
pub type CalibrationRow = (String, i64, f64, f64, f64);

/// How the calibration reads group categories by pot depth (2026-10-18): a category named in
/// `categories` is reported as `category` + `suffix` on rows whose scale (pot + call, big blinds) is
/// at least `min_scale_bb`; every other row keeps its stored category. The store applies the
/// grouping and knows no poker: which categories split, and where, is the policy's rule
/// (`calibration_key` in `sv10-policy`), handed in by the bot.
#[derive(Clone, Copy, Debug)]
pub struct DepthSplit {
    /// Stored categories that split by depth.
    pub categories: &'static [&'static str],
    /// Scale (pot + call, big blinds) from which a row is in the deep class.
    pub min_scale_bb: f64,
    /// Appended to a deep row's category.
    pub suffix: &'static str,
}

impl DepthSplit {
    /// No split: every category pooled over all depths, the grouping before 2026-10-18.
    pub const NONE: DepthSplit = DepthSplit { categories: &[], min_scale_bb: 0.0, suffix: "" };

    /// The split categories as the JSON array `json_each` reads.
    fn categories_json(&self) -> String {
        serde_json::to_string(self.categories).unwrap_or_else(|_| "[]".into())
    }
}

/// A row's calibration class in SQL; binds `:split_cats`, `:split_min` and `:split_suffix`. A row
/// without a scale (before 0207) compares as NULL and stays in its stored category.
const CLASS_SQL: &str = "CASE WHEN scale >= :split_min AND category IN (SELECT value FROM json_each(:split_cats)) \
                         THEN category || :split_suffix ELSE category END";

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
    /// Per calibration class (`split`): (count, mean predicted, mean realized, variance of the residual).
    pub fn calibration_summary(&self, split: &DepthSplit) -> Result<Vec<CalibrationRow>> {
        let conn = self.read();
        let mut stmt = conn.prepare(&format!(
            "SELECT {CLASS_SQL} AS class, COUNT(*), AVG(predicted), AVG(realized),
                    AVG((realized - predicted) * (realized - predicted)) - AVG(realized - predicted) * AVG(realized - predicted)
             FROM calibration GROUP BY class"
        ))?;
        let cats = split.categories_json();
        let rows = stmt
            .query_map(named_params! {":split_cats": cats, ":split_min": split.min_scale_bb, ":split_suffix": split.suffix}, |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, Option<f64>>(4)?.unwrap_or(0.0)))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Every sample recorded at or after `since` (RFC 3339) as (calibration class under `split`,
    /// predicted bb, realized bb), oldest first.
    pub fn calibration_samples_since(&self, since: &str, split: &DepthSplit) -> Result<Vec<(String, f64, f64)>> {
        let conn = self.read();
        let mut stmt = conn.prepare(&format!("SELECT {CLASS_SQL}, predicted, realized FROM calibration WHERE ts >= :since ORDER BY id"))?;
        let cats = split.categories_json();
        let rows = stmt
            .query_map(
                named_params! {":since": since, ":split_cats": cats, ":split_min": split.min_scale_bb, ":split_suffix": split.suffix},
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Per calibration class (`split`), over samples predicted in `[lo, hi)` bb (the decision
    /// margin): (count, mean residual in bb, residual variance).
    pub fn calibration_margin_summary(&self, split: &DepthSplit, lo: f64, hi: f64) -> Result<Vec<(String, i64, f64, f64)>> {
        let conn = self.read();
        let mut stmt = conn.prepare(&format!(
            "SELECT {CLASS_SQL} AS class, COUNT(*), AVG(realized - predicted),
                    AVG((realized - predicted) * (realized - predicted)) - AVG(realized - predicted) * AVG(realized - predicted)
             FROM calibration WHERE predicted >= :lo AND predicted < :hi GROUP BY class"
        ))?;
        let cats = split.categories_json();
        let rows = stmt
            .query_map(
                named_params! {":lo": lo, ":hi": hi, ":split_cats": cats, ":split_min": split.min_scale_bb, ":split_suffix": split.suffix},
                |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<f64>>(2)?.unwrap_or(0.0), r.get::<_, Option<f64>>(3)?.unwrap_or(0.0))),
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Per calibration class (`split`), residuals as a fraction of the pot: (count, mean, variance)
    /// over rows with a known pot.
    pub fn calibration_pot_summary(&self, split: &DepthSplit) -> Result<Vec<(String, i64, f64, f64)>> {
        let conn = self.read();
        let mut stmt = conn.prepare(&format!(
            "SELECT {CLASS_SQL} AS class, COUNT(*), AVG((realized - predicted) / scale),
                    AVG(((realized - predicted) / scale) * ((realized - predicted) / scale)) - AVG((realized - predicted) / scale) * AVG((realized - predicted) / scale)
             FROM calibration WHERE scale > 0 GROUP BY class"
        ))?;
        let cats = split.categories_json();
        let rows = stmt
            .query_map(named_params! {":split_cats": cats, ":split_min": split.min_scale_bb, ":split_suffix": split.suffix}, |r| {
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
        let rows = store.calibration_pot_summary(&DepthSplit::NONE).unwrap();
        assert_eq!(rows.len(), 1);
        let (cat, n, mean, var) = &rows[0];
        assert_eq!((cat.as_str(), *n), ("river:call", 2));
        assert!(mean.abs() < 1e-9 && (var - 0.01).abs() < 1e-9, "{mean} {var}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_depth_split_groups_the_named_categories_by_scale_in_every_read() {
        let dir = std::env::temp_dir().join(format!("sv10-store-cal-split-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        for (cat, scale) in [("turn:call", 499.99), ("turn:call", 500.0), ("turn:call", 2_000.0), ("river:call", 2_000.0)] {
            store.insert_calibration("A", "h", cat, 1.0, 0.0, scale).unwrap();
        }
        // A row from before scales were recorded (0207) stays in its stored category.
        store
            .write_lock()
            .execute(
                "INSERT INTO calibration (bot, hand_id, ts, category, predicted, realized, scale)
                 VALUES ('A', 'h', '2026-01-01T00:00:00Z', 'turn:call', 1.0, 0.0, NULL)",
                [],
            )
            .unwrap();
        let split = DepthSplit { categories: &["turn:call"], min_scale_bb: 500.0, suffix: ":deep" };
        let sorted = |mut v: Vec<(String, i64)>| {
            v.sort();
            v
        };
        let summary = sorted(store.calibration_summary(&split).unwrap().into_iter().map(|r| (r.0, r.1)).collect());
        assert_eq!(summary, [("river:call".to_string(), 1), ("turn:call".into(), 2), ("turn:call:deep".into(), 2)]);
        let margin = sorted(store.calibration_margin_summary(&split, -5.0, 5.0).unwrap().into_iter().map(|r| (r.0, r.1)).collect());
        assert_eq!(margin, summary, "the margin read groups the same way");
        let pot = sorted(store.calibration_pot_summary(&split).unwrap().into_iter().map(|r| (r.0, r.1)).collect());
        assert_eq!(pot, [("river:call".to_string(), 1), ("turn:call".into(), 1), ("turn:call:deep".into(), 2)], "no scale, no pot row");
        let since: Vec<String> =
            store.calibration_samples_since("1970-01-01T00:00:00Z", &split).unwrap().into_iter().map(|s| s.0).collect();
        assert_eq!(since, ["turn:call", "turn:call:deep", "turn:call:deep", "river:call", "turn:call"]);
        let pooled = sorted(store.calibration_summary(&DepthSplit::NONE).unwrap().into_iter().map(|r| (r.0, r.1)).collect());
        assert_eq!(pooled, [("river:call".to_string(), 1), ("turn:call".into(), 4)], "no split: every category pooled, as before");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

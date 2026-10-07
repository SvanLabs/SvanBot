//! Imported training evidence must precede the live observation window (#907).
use super::*;
use chrono::{DateTime, FixedOffset};
use std::collections::HashSet;

fn timestamp(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value).ok()
}

impl HistoryDb {
    /// Completed exports before the earliest verified live start, excluding every live identity.
    /// Without a valid start for every live hand, no warm-up evidence is defensible. In particular,
    /// a completion time or first hero decision cannot stand in for an earlier opponent decision.
    pub fn prior_training_summaries(&self, live: &[(&str, &str)], limit: usize) -> Result<Vec<HandSummary>> {
        if live.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.reader.lock();
        let mut starts = conn.prepare("SELECT started_at FROM raw WHERE hand_id = ?1")?;
        let mut cutoff = None;
        for (id, ended) in live {
            let start: Option<String> = starts.query_row([id], |r| r.get(0)).optional()?.flatten();
            let (Some(start), Some(end)) = (start.as_deref().and_then(timestamp), timestamp(ended)) else {
                tracing::warn!(live_hands = live.len(), "history warm-up disabled: missing or invalid live boundary");
                return Ok(Vec::new());
            };
            if start >= end {
                tracing::warn!(live_hands = live.len(), "history warm-up disabled: live start does not precede completion");
                return Ok(Vec::new());
            }
            cutoff = Some(cutoff.map_or(start, |old: DateTime<FixedOffset>| old.min(start)));
        }
        let cutoff = cutoff.expect("nonempty live set has valid starts");
        let ids: HashSet<&str> = live.iter().map(|(id, _)| *id).collect();
        let mut query =
            conn.prepare("SELECT hand_id, started_at, json, summary FROM raw WHERE summary IS NOT NULL ORDER BY started_at DESC LIMIT ?1")?;
        let rows = query.query_map([limit as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, self.codec.text(r.get_ref(2)?)?, self.codec.text(r.get_ref(3)?)?))
        })?;
        let mut prior = Vec::new();
        let (mut unreadable, mut invalid_time, mut outside_window, mut duplicate) = (0, 0, 0, 0);
        for row in rows {
            let (id, started, raw, summary) = row?;
            if ids.contains(id.as_str()) {
                duplicate += 1;
                continue;
            }
            let Ok(export) = serde_json::from_str::<Value>(&raw) else {
                unreadable += 1;
                continue;
            };
            let (Some(start), Some(end)) = (started.as_deref().and_then(timestamp), export["ended_at"].as_str().and_then(timestamp)) else {
                invalid_time += 1;
                continue;
            };
            if start > end {
                invalid_time += 1;
                continue;
            }
            if end >= cutoff {
                outside_window += 1;
                continue;
            }
            if let Ok(hand) = serde_json::from_str(&summary) {
                prior.push((end, id, hand));
            } else {
                unreadable += 1;
            }
        }
        if unreadable + invalid_time > 0 {
            tracing::warn!(unreadable, invalid_time, "history warm-up rejected unreadable or invalid exports");
        }
        tracing::info!(accepted = prior.len(), outside_window, duplicate, "history warm-up chronology selection");
        prior.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        Ok(prior.into_iter().map(|(_, _, hand)| hand).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database(tag: &str) -> HistoryDb {
        let dir = std::env::temp_dir().join(format!("sv10-history-chronology-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        HistoryDb::open(&dir.join("history.db")).unwrap()
    }

    fn add(db: &HistoryDb, id: &str, start: &str, end: Option<&str>, tag: usize) {
        db.insert_page("Hero", &[serde_json::json!({"hand_id":id,"started_at":start,"ended_at":end})]).unwrap();
        let rowid: i64 = db.conn.lock().query_row("SELECT id FROM raw WHERE hand_id=?1", [id], |r| r.get(0)).unwrap();
        let summary = serde_json::json!({"players":[],"button":tag,"bb":20,"history":[],"board":[],"shown":[],"stacks":[]});
        db.set_summaries(&[(rowid, summary.to_string())]).unwrap();
    }

    #[test]
    fn history_warmup_uses_completion_before_start_not_before_end() {
        let db = database("bounds");
        add(&db, "live", "2026-01-02T10:00:00Z", Some("2026-01-02T10:10:00Z"), 0);
        // Both valid exports finish before the start, in the reverse order to their starts.
        add(&db, "first", "2026-01-01T09:00:00Z", Some("2026-01-01T09:05:00Z"), 1);
        add(&db, "second", "2026-01-01T08:00:00Z", Some("2026-01-01T09:06:00Z"), 2);
        add(&db, "overlap", "2026-01-02T09:59:00Z", Some("2026-01-02T10:05:00Z"), 3);
        add(&db, "boundary", "2026-01-02T09:59:00Z", Some("2026-01-02T11:00:00+01:00"), 4);
        add(&db, "future", "2026-01-03T00:00:00Z", Some("2026-01-03T00:01:00Z"), 5);
        add(&db, "missing-end", "2026-01-01T00:00:00Z", None, 6);
        add(&db, "invalid-end", "2026-01-01T00:00:00Z", Some("not a timestamp"), 7);
        add(&db, "backwards", "2026-01-01T10:00:00Z", Some("2026-01-01T09:00:00Z"), 8);
        // The summary and export exercise the packed-column path as well as plain timestamps.
        db.compact(&mut [Some(0), Some(0), None], 100).unwrap();
        let rows = db.prior_training_summaries(&[("live", "2026-01-02T10:10:00Z")], 100).unwrap();
        assert_eq!(rows.iter().map(|h| h.button).collect::<Vec<_>>(), [1, 2]);
    }

    #[test]
    fn history_warmup_requires_every_live_start_and_excludes_duplicate_ids() {
        let db = database("identity");
        add(&db, "live", "2026-01-02T10:00:00Z", Some("2026-01-02T10:10:00Z"), 0);
        add(&db, "old", "2026-01-01T09:00:00Z", Some("2026-01-01T09:05:00Z"), 1);
        let live = [("live", "2026-01-02T10:10:00Z"), ("old", "2026-01-01T09:05:00Z")];
        assert!(db.prior_training_summaries(&live, 100).unwrap().is_empty());
        let missing = [("live", "2026-01-02T10:10:00Z"), ("unexported", "2026-01-02T10:11:00Z")];
        assert!(db.prior_training_summaries(&missing, 100).unwrap().is_empty());
        assert!(db.prior_training_summaries(&[("live", "invalid")], 100).unwrap().is_empty());
        assert!(db.prior_training_summaries(&[], 100).unwrap().is_empty());
    }
}

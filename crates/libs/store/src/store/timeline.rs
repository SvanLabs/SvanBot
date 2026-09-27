//! Read-only, bounded slices for the dashboard's hourly fleet timeline.
use super::*;
use anyhow::Result;
use rusqlite::params;
use std::collections::BTreeMap;

/// One UTC hour of fleet hand results, aggregated without loading hand summaries.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct TimelineHour {
    /// Start of the UTC hour, in Unix seconds.
    pub ts: i64,
    /// Stored hand rows, including ones awaiting a net result.
    pub hands: i64,
    /// Rows with a recorded net result.
    pub priced: i64,
    /// Recorded net chips.
    pub net: i64,
    /// All-in EV adjusted net chips where available, otherwise recorded net.
    pub ev_net: f64,
    /// Sum of squared adjusted net chips, for a descriptive hand-level interval.
    pub ev_net_sq: f64,
}

/// The exact lightweight row behind a selected timeline hour.
#[derive(Clone, Debug, serde::Serialize)]
pub struct TimelineHand {
    /// Name under which the row is stored.
    pub bot: String,
    /// OpenPoker hand identifier.
    pub hand_id: String,
    /// Stored RFC 3339 end timestamp.
    pub ts: String,
    /// Recorded net chips, if known.
    pub net: Option<i64>,
    /// All-in EV adjusted net chips, if filled.
    pub ev_net: Option<f64>,
}

/// A stored warning or error placed on the timeline.
#[derive(Clone, Debug, serde::Serialize)]
pub struct TimelineEvent {
    /// Primary key of the source event row.
    pub id: i64,
    /// Stored RFC 3339 timestamp.
    pub ts: String,
    /// Bot or fleet name, if present.
    pub bot: Option<String>,
    /// Stored log severity.
    pub level: String,
    /// Exact logged message.
    pub message: String,
}

impl Store {
    /// Aggregate only the indexed bot/time ranges in this window. Reading the numeric columns
    /// avoids unpacking the large hand summary on the dashboard's blocking pool.
    pub fn timeline_hours(&self, names: &[String], start: &str, end: &str) -> Result<Vec<TimelineHour>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT CAST(strftime('%s', ended_at) AS INTEGER) / 3600 * 3600 AS hour,
                    COUNT(*), COUNT(net), COALESCE(SUM(net), 0),
                    COALESCE(SUM(COALESCE(ev_net, CAST(net AS REAL))), 0.0),
                    COALESCE(SUM(COALESCE(ev_net, CAST(net AS REAL)) * COALESCE(ev_net, CAST(net AS REAL))), 0.0)
             FROM hands WHERE bot = ?1 AND ended_at >= ?2 AND ended_at < ?3
             GROUP BY hour ORDER BY hour",
        )?;
        let mut grouped: BTreeMap<i64, TimelineHour> = BTreeMap::new();
        for name in names {
            let rows = stmt.query_map(params![name, start, end], |r| {
                Ok(TimelineHour {
                    ts: r.get(0)?,
                    hands: r.get(1)?,
                    priced: r.get(2)?,
                    net: r.get(3)?,
                    ev_net: r.get(4)?,
                    ev_net_sq: r.get(5)?,
                })
            })?;
            for row in rows {
                let row = row?;
                let hour = grouped.entry(row.ts).or_insert_with(|| TimelineHour { ts: row.ts, ..TimelineHour::default() });
                hour.hands += row.hands;
                hour.priced += row.priced;
                hour.net += row.net;
                hour.ev_net += row.ev_net;
                hour.ev_net_sq += row.ev_net_sq;
            }
        }
        Ok(grouped.into_values().collect())
    }

    /// Exact stored rows behind one hour of the timeline; the caller chooses the hour and fleet
    /// identities. This is an indexed one-hour lookup, not a full-history replay.
    pub fn timeline_hands(&self, names: &[String], start: &str, end: &str) -> Result<Vec<TimelineHand>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT bot, hand_id, ended_at, net, ev_net FROM hands
             WHERE bot = ?1 AND ended_at >= ?2 AND ended_at < ?3 ORDER BY ended_at",
        )?;
        let mut out = Vec::new();
        for name in names {
            let rows = stmt.query_map(params![name, start, end], |r| {
                Ok(TimelineHand { bot: r.get(0)?, hand_id: r.get(1)?, ts: r.get(2)?, net: r.get(3)?, ev_net: r.get(4)? })
            })?;
            out.extend(rows.collect::<Result<Vec<_>, _>>()?);
        }
        out.sort_by(|a, b| a.ts.cmp(&b.ts).then_with(|| a.bot.cmp(&b.bot)).then_with(|| a.hand_id.cmp(&b.hand_id)));
        Ok(out)
    }

    /// Warning/error records remain their exact stored rows in the timeline. The events table is
    /// small (about ten thousand rows on the 2026-09-27 fleet); filtering it is a read-only scan.
    pub fn timeline_events(&self, start: &str, end: &str) -> Result<Vec<TimelineEvent>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT id, ts, bot, level, message FROM events
             WHERE ts >= ?1 AND ts < ?2 AND level IN ('warn', 'error') ORDER BY ts, id",
        )?;
        Ok(stmt
            .query_map(params![start, end], |r| {
                Ok(TimelineEvent { id: r.get(0)?, ts: r.get(1)?, bot: r.get(2)?, level: r.get(3)?, message: r.get(4)? })
            })?
            .collect::<Result<Vec<_>, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hourly_axis_and_drill_use_the_same_stored_fleet_rows() {
        let dir = std::env::temp_dir().join(format!("sv10-timeline-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir.join("hands.db")).unwrap();
        {
            let conn = store.conn.lock();
            for (bot, hand, ts, net, ev) in [
                ("A", "a1", "2026-09-25T03:02:00+00:00", Some(30), Some(20.0)),
                ("B", "b1", "2026-09-25T03:04:00+00:00", Some(-10), Some(-5.0)),
                ("A", "a2", "2026-09-25T03:05:00+00:00", None, None),
                ("C", "c1", "2026-09-25T03:06:00+00:00", Some(100), Some(100.0)),
                ("A", "a3", "2026-09-25T04:01:00+00:00", Some(5), None),
            ] {
                conn.execute(
                    "INSERT INTO hands (bot, hand_id, ended_at, net, ev_net) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![bot, hand, ts, net, ev],
                )
                .unwrap();
            }
            conn.execute(
                "INSERT INTO events (ts, bot, level, message) VALUES ('2026-09-25T03:03:00+00:00', 'A', 'warn', 'connection lost')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO events (ts, bot, level, message) VALUES ('2026-09-25T03:03:10+00:00', 'A', 'info', 'ordinary action')",
                [],
            )
            .unwrap();
        }
        let names = vec!["A".to_string(), "B".to_string()];
        let from = "2026-09-25T03:00:00+00:00";
        let until = "2026-09-25T05:00:00+00:00";
        let axis = store.timeline_hours(&names, from, until).unwrap();
        assert_eq!(axis.len(), 2);
        assert_eq!((axis[0].hands, axis[0].priced, axis[0].net, axis[0].ev_net, axis[0].ev_net_sq), (3, 2, 20, 15.0, 425.0));
        assert_eq!((axis[1].hands, axis[1].net, axis[1].ev_net), (1, 5, 5.0));
        let hands = store.timeline_hands(&names, from, "2026-09-25T04:00:00+00:00").unwrap();
        assert_eq!(hands.iter().map(|h| h.hand_id.as_str()).collect::<Vec<_>>(), ["a1", "b1", "a2"]);
        let events = store.timeline_events(from, until).unwrap();
        assert_eq!(events.iter().map(|e| e.message.as_str()).collect::<Vec<_>>(), ["connection lost"]);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

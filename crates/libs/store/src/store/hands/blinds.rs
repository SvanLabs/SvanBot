//! Per-hand blind for rate calculations when tables change stakes.

use super::*;

/// A stored hand result with its own positive big blind, if the summary records one.
#[derive(Clone, Debug, PartialEq)]
pub struct EvResultWithBlind {
    /// Settled chip result, if known.
    pub net: Option<i64>,
    /// All-in EV net, falling back to the settled result until filled.
    pub ev_net: Option<f64>,
    /// Whether the hand reached showdown.
    pub showdown: bool,
    /// End timestamp in RFC 3339 form.
    pub ended_at: String,
    /// Positive blind from this hand's summary, or none if absent or invalid.
    pub big_blind: Option<i64>,
}

impl Store {
    /// Results in insertion order, with no current-table fallback for missing or invalid blinds.
    pub fn bot_ev_results_with_blinds(&self, bot: &str) -> Result<Vec<EvResultWithBlind>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT net, COALESCE(ev_net, CAST(net AS REAL)), COALESCE(showdown, 0), ended_at,
                    CASE WHEN json_valid(summary) THEN
                        CASE WHEN json_type(summary, '$.bb') = 'integer' AND json_extract(summary, '$.bb') > 0
                            THEN json_extract(summary, '$.bb') END
                    END
             FROM hands WHERE bot = ?1 ORDER BY rowid",
        )?;
        let rows = stmt
            .query_map(params![bot], |r| {
                Ok(EvResultWithBlind {
                    net: r.get(0)?,
                    ev_net: r.get(1)?,
                    showdown: r.get::<_, i64>(2)? != 0,
                    ended_at: r.get(3)?,
                    big_blind: r.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn result_blinds_come_from_each_hand_and_invalid_summaries_remain_unpriced() {
        let dir = std::env::temp_dir().join(format!("sv10-result-blinds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        for (id, net, summary) in [
            ("h1", 10, r#"{"bb":10}"#),
            ("h2", 20, r#"{"bb":20}"#),
            ("h3", 30, r#"{"bb":0}"#),
            ("h4", 40, r#"{"bb":"20"}"#),
            ("h5", 50, "bad json"),
            ("h6", 60, r#"{}"#),
        ] {
            store
                .insert_hand(&HandRow {
                    bot: "A".into(),
                    hand_id: id.into(),
                    ended_at: format!("2026-09-24T00:00:0{}Z", &id[1..]),
                    net: Some(net),
                    summary: summary.into(),
                    ..Default::default()
                })
                .unwrap();
        }
        let rows = store.bot_ev_results_with_blinds("A").unwrap();
        assert_eq!(rows.iter().map(|r| r.big_blind).collect::<Vec<_>>(), [Some(10), Some(20), None, None, None, None]);
        assert_eq!(rows.iter().map(|r| r.net).collect::<Vec<_>>(), [Some(10), Some(20), Some(30), Some(40), Some(50), Some(60)]);
        assert_eq!(rows[0].ev_net, Some(10.0), "missing filled EV falls back to actual net");
        assert_eq!(rows[0].ended_at, "2026-09-24T00:00:01Z");
        assert!(!rows[0].showdown);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! Our recorded decisions: inserts, per-hand reads and champion-version lookup.
use super::*;
use anyhow::Result;
use rusqlite::{OptionalExtension, params};
use std::collections::HashMap;

/// One of our recorded postflop bets into no bet (the fold-calibration sample source, 0156).
#[derive(Clone, Debug)]
pub struct PostflopBet {
    /// Decision time (RFC 3339).
    pub ts: String,
    /// Our bot.
    pub bot: String,
    /// Server hand id.
    pub hand_id: String,
    /// `flop`, `turn` or `river`.
    pub street: String,
    /// Raise-to amount sent.
    pub amount: Option<i64>,
    /// The decision's explanation JSON.
    pub detail: String,
}

/// One of our recorded preflop raises or all-ins (the preflop fold calibration, 2026-09-23).
#[derive(Clone, Debug)]
pub struct PreflopRaise {
    /// Decision time (RFC 3339).
    pub ts: String,
    /// Our bot.
    pub bot: String,
    /// Server hand id.
    pub hand_id: String,
    /// `raise` or `all_in`.
    pub action: String,
    /// Raise-to amount sent (none for an all-in).
    pub amount: Option<i64>,
    /// The decision's explanation JSON.
    pub detail: String,
}

/// One of our recorded postflop decisions, any action (the raise-war study source, 0158).
#[derive(Clone, Debug)]
pub struct PostflopDecision {
    /// Our bot.
    pub bot: String,
    /// Server hand id.
    pub hand_id: String,
    /// `flop`, `turn` or `river`.
    pub street: String,
    /// `fold`, `check`, `call`, `raise` or `all_in`.
    pub action: String,
    /// The model's equity estimate at the decision.
    pub equity: f64,
    /// Opponents still in the hand.
    pub opponents: Option<i64>,
    /// Pot before our action, chips.
    pub pot: Option<i64>,
    /// Chips we had to call.
    pub to_call: Option<i64>,
}

impl Store {
    /// Our postflop decisions in recorded order (oldest first), so the k-th decision of a bot on a
    /// street of a hand is its k-th action there.
    pub fn postflop_decisions(&self) -> Result<Vec<PostflopDecision>> {
        let conn = self.read();
        let mut stmt = conn.prepare(&format!(
            "SELECT bot, COALESCE(hand_id, ''), street, COALESCE(action, ''), COALESCE(equity, -1),
                    COALESCE(opponents, CASE WHEN typeof(detail) = 'text' AND json_valid(detail) THEN json_extract(detail, '$.opponents') END),
                    pot, to_call
             FROM decisions WHERE street IN ('flop', 'turn', 'river') AND {} ORDER BY id",
            ordinary_hand("decisions.bot", "decisions.hand_id")
        )
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PostflopDecision {
                    bot: r.get(0)?,
                    hand_id: r.get(1)?,
                    street: r.get(2)?,
                    action: r.get(3)?,
                    equity: r.get(4)?,
                    opponents: r.get(5)?,
                    pot: r.get(6)?,
                    to_call: r.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// (street, latency ms) of every decision recorded at or after `since` (RFC 3339).
    pub fn decision_latencies_since(&self, since: &str) -> Result<Vec<(String, f64)>> {
        let conn = self.read();
        let mut stmt = conn.prepare("SELECT COALESCE(street, ''), latency_ms FROM decisions WHERE ts >= ?1 AND latency_ms IS NOT NULL")?;
        let rows = stmt.query_map(params![since], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// (street, action family, decisions) recorded at or after `since` (RFC 3339): the whole
    /// population of each decision-loss class, audited or not (0355).
    ///
    /// This is the denominator of the coverage a decision-loss row names, so it is deliberately the
    /// plain one: every decision live play recorded, treatment-arm hands included — the analyst's
    /// verdicts are not arm-filtered either, and a ratio has to count one population against itself.
    /// The action is the family as live play records it (unsized: `raise`, not `raise:640`), which is
    /// the vocabulary the verdicts' `live_action` is grouped into. Decisions with no street or action
    /// are in no class and are not counted; a class the scan has no verdict for is simply absent.
    pub fn decision_counts_since(&self, since: &str) -> Result<Vec<(String, String, i64)>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT street, action, COUNT(*) FROM decisions
             WHERE ts >= ?1 AND COALESCE(street, '') <> '' AND COALESCE(action, '') <> '' GROUP BY street, action",
        )?;
        let rows = stmt.query_map(params![since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// (street, explanation JSON) of every decision recorded at or after `since` (RFC 3339), treatment-arm
    /// hands excluded, oldest first: how many choices self-calibration's bias decides is read off each
    /// decision's candidates (0332).
    pub fn decision_details_since(&self, since: &str) -> Result<Vec<(String, String)>> {
        let conn = self.read();
        let mut stmt = conn.prepare(&format!(
            "SELECT COALESCE(street, ''), COALESCE(detail, '') FROM decisions WHERE ts >= ?1 AND {} ORDER BY id",
            ordinary_hand("decisions.bot", "decisions.hand_id")
        ))?;
        let rows = stmt.query_map(params![since], |r| Ok((r.get(0)?, self.codec.text(r.get_ref(1)?)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// How often each action was our *first* preflop decision of a hand recorded in `[since, until)`
    /// (RFC 3339), treatment-arm hands excluded: (action, hands). The style-drift check (0332) compares two
    /// windows of it.
    pub fn first_preflop_actions(&self, since: &str, until: &str) -> Result<Vec<(String, i64)>> {
        let conn = self.read();
        let mut stmt = conn.prepare(&format!(
            "SELECT COALESCE(d.action, ''), COUNT(*) FROM decisions d JOIN (
                 SELECT MIN(id) AS id FROM decisions WHERE street = 'preflop' AND ts >= ?1 AND ts < ?2 GROUP BY bot, hand_id
             ) f ON f.id = d.id WHERE {} GROUP BY d.action",
            ordinary_hand("d.bot", "d.hand_id")
        ))?;
        let rows = stmt.query_map(params![since, until], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Our preflop raises and all-ins, newest first, for the preflop fold calibration.
    pub fn preflop_raises(&self, limit: usize) -> Result<Vec<PreflopRaise>> {
        let conn = self.read();
        let mut stmt = conn.prepare(&format!(
            "SELECT ts, bot, COALESCE(hand_id, ''), action, amount, COALESCE(detail, '') FROM decisions
             WHERE street = 'preflop' AND action IN ('raise', 'all_in')
             AND {} ORDER BY id DESC LIMIT ?1",
            ordinary_hand("decisions.bot", "decisions.hand_id")
        ))?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok(PreflopRaise {
                    ts: r.get(0)?,
                    bot: r.get(1)?,
                    hand_id: r.get(2)?,
                    action: r.get(3)?,
                    amount: r.get(4)?,
                    detail: self.codec.text(r.get_ref(5)?)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Our most recent postflop bets or raises made with nothing to call, newest first.
    pub fn heads_up_postflop_bets(&self, limit: usize) -> Result<Vec<PostflopBet>> {
        let conn = self.read();
        let mut stmt = conn.prepare(&format!(
            "SELECT ts, bot, COALESCE(hand_id, ''), street, amount, COALESCE(detail, '') FROM decisions
             WHERE street IN ('flop', 'turn', 'river') AND action IN ('raise', 'all_in') AND to_call = 0
             AND {} ORDER BY id DESC LIMIT ?1",
            ordinary_hand("decisions.bot", "decisions.hand_id")
        ))?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok(PostflopBet {
                    ts: r.get(0)?,
                    bot: r.get(1)?,
                    hand_id: r.get(2)?,
                    street: r.get(3)?,
                    amount: r.get(4)?,
                    detail: self.codec.text(r.get_ref(5)?)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Record one of our decisions with its explanation JSON.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_decision(
        &self,
        bot: &str,
        hand_id: &str,
        street: &str,
        action: &str,
        amount: Option<i64>,
        // The equity the decision was priced on, or `None` when the draw could not measure one and
        // the decision was refused (#424): stored as NULL, which the readers already read as "no
        // measurement" (`COALESCE(equity, -1)` in `postflop_decisions`).
        equity: Option<f64>,
        pot: i64,
        to_call: i64,
        latency_ms: f64,
        detail: &str,
    ) -> Result<()> {
        // The fields SQL selects on get real columns; the detail itself is stored packed (0229).
        let parsed = serde_json::from_str::<serde_json::Value>(detail).ok();
        let field = |k: &str| parsed.as_ref().and_then(|v| v.get(k));
        let opponents = field("opponents").and_then(|v| v.as_i64());
        let version = field("version").and_then(|v| v.as_str());
        let candidates = field("candidates").and_then(|v| v.as_array()).map(|a| a.len() as i64);
        // Compressed before the write lock is taken, so the lock covers the write alone: every other
        // process's writer waits on it (0322). Only a due dictionary reload packs under the lock.
        let cached = self.codec.pack_cached(crate::packed::DECISION_DETAIL, detail);
        let conn = self.write_lock();
        let packed = cached.unwrap_or_else(|| self.codec.pack(&conn, crate::packed::DECISION_DETAIL, detail));
        conn.execute(
            "INSERT INTO decisions (bot, hand_id, ts, street, action, amount, equity, pot, to_call, latency_ms, detail, opponents, version, candidates)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                bot,
                hand_id,
                chrono::Utc::now().to_rfc3339(),
                street,
                action,
                amount,
                equity,
                pot,
                to_call,
                latency_ms,
                packed,
                opponents,
                version,
                candidates
            ],
        )?;
        Ok(())
    }
    /// Champion version recorded with each hand's last decision (decisions since 2026-09-15 carry it).
    pub fn decision_versions(&self, bot: &str, hand_ids: &[String]) -> Result<HashMap<String, String>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT v FROM (SELECT id, COALESCE(version, CASE WHEN typeof(detail) = 'text' AND json_valid(detail) THEN json_extract(detail, '$.version') END) AS v
               FROM decisions WHERE bot = ?1 AND hand_id = ?2) WHERE v IS NOT NULL ORDER BY id DESC LIMIT 1",
        )?;
        let mut out = HashMap::new();
        for id in hand_ids {
            if let Some(v) = stmt.query_row(params![bot, id], |r| r.get::<_, String>(0)).optional()? {
                out.insert(id.clone(), v);
            }
        }
        Ok(out)
    }
    /// Our decisions in one hand as JSON objects, in order.
    pub fn decisions_for_hand(&self, bot: &str, hand_id: &str) -> Result<Vec<serde_json::Value>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT ts, street, action, amount, equity, pot, to_call, latency_ms, detail FROM decisions
             WHERE bot = ?1 AND hand_id = ?2 ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![bot, hand_id], |r| {
                Ok(serde_json::json!({
                    "ts": r.get::<_, String>(0)?,
                    "street": r.get::<_, Option<String>>(1)?,
                    "action": r.get::<_, Option<String>>(2)?,
                    "amount": r.get::<_, Option<i64>>(3)?,
                    "equity": r.get::<_, Option<f64>>(4)?,
                    "pot": r.get::<_, Option<i64>>(5)?,
                    "to_call": r.get::<_, Option<i64>>(6)?,
                    "latency_ms": r.get::<_, Option<f64>>(7)?,
                    "detail": self.codec.opt_text(r.get_ref(8)?)?.and_then(|d| serde_json::from_str::<serde_json::Value>(&d).ok()),
                }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

/// A recent decision for the "What would Svanbot do?" quiz (0220).
#[derive(Clone, Debug, PartialEq)]
pub struct QuizSpot {
    /// Decision row id.
    pub id: i64,
    /// Our bot.
    pub bot: String,
    /// Server hand id.
    pub hand_id: String,
    /// Street name.
    pub street: String,
    /// Action the bot took.
    pub action: String,
    /// Its amount (raise-to), when sized.
    pub amount: Option<i64>,
    /// Pot before the action, chips.
    pub pot: i64,
    /// Chips to call.
    pub to_call: i64,
    /// The decision detail JSON (board, hole, candidates with EVs, opponents).
    pub detail: String,
}

impl Store {
    /// A random decision among the newest `window` with at least `min_options` candidate actions
    /// and a pot of at least `min_pot` chips.
    pub fn random_quiz_spot(&self, window: i64, min_options: i64, min_pot: i64) -> Result<Option<QuizSpot>> {
        let conn = self.read();
        let spot = conn
            .query_row(
                "SELECT id, bot, COALESCE(hand_id, ''), COALESCE(street, ''), COALESCE(action, ''), COALESCE(pot, 0), COALESCE(to_call, 0), detail, amount
                 FROM decisions WHERE id > (SELECT COALESCE(MAX(id), 0) - ?1 FROM decisions) AND COALESCE(pot, 0) >= ?3
                   AND COALESCE(candidates, CASE WHEN typeof(detail) = 'text' AND json_valid(detail)
                                             THEN json_array_length(json_extract(detail, '$.candidates')) END) >= ?2
                 ORDER BY random() LIMIT 1",
                params![window, min_options, min_pot],
                |r| {
                    Ok(QuizSpot {
                        id: r.get(0)?,
                        bot: r.get(1)?,
                        hand_id: r.get(2)?,
                        street: r.get(3)?,
                        action: r.get(4)?,
                        pot: r.get(5)?,
                        to_call: r.get(6)?,
                        detail: self.codec.text(r.get_ref(7)?)?,
                        amount: r.get(8)?,
                    })
                },
            )
            .optional()?;
        Ok(spot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0355: the coverage denominator. A class's population is its decisions inside the window, per
    /// street and action family, and a decision recorded outside it — or with no street to belong to —
    /// is in none of them.
    #[test]
    fn decision_counts_are_the_class_population_in_the_window() {
        let dir = std::env::temp_dir().join(format!("sv10-store-decision-counts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        // `insert_decision` stamps `now`, which is inside every window; the rows go in straight so the
        // test can choose their time.
        let conn = Connection::open(dir.join("svanbot10.db")).unwrap();
        let put = |days_ago: i64, street: Option<&str>, action: &str| {
            conn.execute(
                "INSERT INTO decisions (bot, hand_id, ts, street, action) VALUES ('A', 'h', ?1, ?2, ?3)",
                params![(chrono::Utc::now() - chrono::Duration::days(days_ago)).to_rfc3339(), street, action],
            )
            .unwrap();
        };
        put(1, Some("preflop"), "raise");
        put(1, Some("preflop"), "raise");
        put(1, Some("river"), "call");
        put(20, Some("preflop"), "raise");
        put(40, Some("turn"), "raise");
        put(1, None, "call");

        let counts = |days: i64| -> std::collections::BTreeMap<(String, String), i64> {
            store
                .decision_counts_since(&(chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339())
                .unwrap()
                .into_iter()
                .map(|(street, action, n)| ((street, action), n))
                .collect()
        };
        let short = counts(8);
        assert_eq!(short[&("preflop".to_string(), "raise".to_string())], 2);
        assert_eq!(short[&("river".to_string(), "call".to_string())], 1);
        assert_eq!(short.len(), 2, "the 20-day-old raise is outside the short window: {short:?}");
        let long = counts(30);
        assert_eq!(long[&("preflop".to_string(), "raise".to_string())], 3, "inside the long window the 20-day-old raise counts");
        assert_eq!(long.len(), 2, "and the 40-day-old turn raise is outside the retention the store keeps: {long:?}");
        assert!(!long.keys().any(|(street, _)| street.is_empty()), "a decision with no street is in no class");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

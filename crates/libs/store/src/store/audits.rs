//! The analyst queue: queued deep re-solves, their verdicts and summaries.
use super::*;
use anyhow::Result;
use rusqlite::params;

/// How long a stored deep re-solve verdict (`decision_audit`) is kept: [`Store::prune_audits`] drops
/// results older than this, and nothing else deletes them. Readers of the table measure over a window
/// that must fit inside it — the findings scan's is
/// `sv10_bot::findings::DECISION_LOSS_DAYS`, which a test asserts is not longer than this (0345).
pub const AUDIT_RESULT_DAYS: i64 = 30;

/// A queued deep re-solve of one live decision.
#[derive(Clone, Debug)]
pub struct AuditJob {
    /// Queue row id.
    pub id: i64,
    /// Bot that decided.
    pub bot: String,
    /// Hand id ('' when unknown).
    pub hand_id: String,
    /// Digest of the response network the decision used, if any (`replay_nets`).
    pub net_digest: Option<String>,
    /// `ReplayRecord` JSON.
    pub record: String,
}

/// The analyst's verdict on one live decision.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AuditResult {
    /// Bot that decided.
    pub bot: String,
    /// Hand id.
    pub hand_id: String,
    /// Street name.
    pub street: String,
    /// Action the live search chose (`raise:640` style for sized actions).
    pub live_action: String,
    /// Action the deep search prefers.
    pub deep_action: String,
    /// Deep-search EV of its best action minus that of the live choice, in big blinds (>= 0).
    pub gap_bb: f64,
    /// Pot in big blinds at the decision.
    pub pot_bb: f64,
    /// Deep search wall time, milliseconds.
    pub deep_ms: f64,
    /// Deep search Monte Carlo samples.
    pub samples: usize,
    /// `ReplayRecord::version` of the record this verdict was computed from (0316).
    ///
    /// The record carries the inputs live play had (the per-opponent corrections, replay v3), and a
    /// record written before that lacks them: a gap measured on one of those compares the live choice
    /// with a model that saw less than the live choice did. `None` means the version was not recorded
    /// (rows written before this column existed, or an unreadable record) and no measurement may mix
    /// it with a versioned one.
    pub replay_version: Option<u32>,
}

/// Totals of [`Store::audit_summary`].
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct AuditSummary {
    /// Decisions audited.
    pub decisions: i64,
    /// Of those, where the live search chose the deep search's action.
    pub same_action: i64,
    /// Mean EV gap, big blinds per decision.
    pub mean_gap_bb: f64,
    /// Largest EV gap, big blinds.
    pub max_gap_bb: f64,
    /// Mean deep-search time, milliseconds.
    pub mean_deep_ms: f64,
    /// Decisions still waiting in the queue.
    pub queued: i64,
}

impl Store {
    /// Queue a live decision's full inputs (a `ReplayRecord`) for the analyst's deep re-solve.
    pub fn insert_audit(&self, bot: &str, hand_id: &str, record: &str, net: Option<(&str, &str)>) -> Result<()> {
        // Compressed before the write lock, which then covers only the write (0322); only a due
        // dictionary reload packs under it.
        let cached = self.codec.pack_cached(crate::packed::AUDIT_RECORD, record);
        let conn = self.write_lock();
        let packed = cached.unwrap_or_else(|| self.codec.pack(&conn, crate::packed::AUDIT_RECORD, record));
        let tx = conn.unchecked_transaction()?;
        if let Some((digest, json)) = net {
            tx.execute("INSERT OR IGNORE INTO replay_nets (digest, json) VALUES (?1, ?2)", params![digest, json])?;
        }
        tx.execute(
            "INSERT INTO audit_queue (ts, bot, hand_id, net_digest, record) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![chrono::Utc::now().to_rfc3339(), bot, hand_id, net.map(|n| n.0), packed],
        )?;
        tx.commit()?;
        Ok(())
    }
    /// Oldest queued audits: (id, bot, hand_id, net digest, record JSON).
    pub fn audit_batch(&self, limit: usize) -> Result<Vec<AuditJob>> {
        let conn = self.read();
        let mut stmt = conn.prepare("SELECT id, bot, COALESCE(hand_id, ''), net_digest, record FROM audit_queue ORDER BY id LIMIT ?1")?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok(AuditJob {
                    id: r.get(0)?,
                    bot: r.get(1)?,
                    hand_id: r.get(2)?,
                    net_digest: r.get(3)?,
                    record: self.codec.text(r.get_ref(4)?)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Store an audit result (or none for an unreadable record) and remove the job from the queue, atomically.
    pub fn finish_audit(&self, job_id: i64, result: Option<&AuditResult>) -> Result<()> {
        let conn = self.write_lock();
        let tx = conn.unchecked_transaction()?;
        if let Some(r) = result {
            tx.execute(
                "INSERT INTO decision_audit (ts, bot, hand_id, street, live_action, deep_action, gap_bb, pot_bb, deep_ms, samples, replay_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    chrono::Utc::now().to_rfc3339(),
                    r.bot,
                    r.hand_id,
                    r.street,
                    r.live_action,
                    r.deep_action,
                    r.gap_bb,
                    r.pot_bb,
                    r.deep_ms,
                    r.samples as i64,
                    r.replay_version.map(i64::from)
                ],
            )?;
        }
        tx.execute("DELETE FROM audit_queue WHERE id = ?1", params![job_id])?;
        tx.commit()?;
        Ok(())
    }
    /// Audit totals since `since` (RFC 3339): decisions, same action as the deep search, mean and
    /// largest gap in big blinds, mean deep-search milliseconds; plus the current queue length.
    pub fn audit_summary(&self, since: &str) -> Result<AuditSummary> {
        let conn = self.read();
        let (decisions, same, mean_gap, max_gap, mean_ms): (i64, Option<i64>, Option<f64>, Option<f64>, Option<f64>) = conn.query_row(
            "SELECT COUNT(*), SUM(live_action = deep_action), AVG(gap_bb), MAX(gap_bb), AVG(deep_ms) FROM decision_audit WHERE ts >= ?1",
            params![since],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )?;
        let queued: i64 = conn.query_row("SELECT COUNT(*) FROM audit_queue", [], |r| r.get(0))?;
        Ok(AuditSummary {
            decisions,
            same_action: same.unwrap_or(0),
            mean_gap_bb: mean_gap.unwrap_or(0.0),
            max_gap_bb: max_gap.unwrap_or(0.0),
            mean_deep_ms: mean_ms.unwrap_or(0.0),
            queued,
        })
    }
    /// Every audit result since `since` (RFC 3339), oldest first, with its time (decision grades, 0220).
    pub fn audit_results_since(&self, since: &str) -> Result<Vec<(String, AuditResult)>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT ts, bot, COALESCE(hand_id, ''), COALESCE(street, ''), COALESCE(live_action, ''), COALESCE(deep_action, ''), gap_bb,
                    COALESCE(pot_bb, 0), COALESCE(deep_ms, 0), COALESCE(samples, 0), replay_version
             FROM decision_audit WHERE ts >= ?1 ORDER BY id",
        )?;
        let rows = stmt
            .query_map(params![since], |r| {
                Ok((
                    r.get(0)?,
                    AuditResult {
                        bot: r.get(1)?,
                        hand_id: r.get(2)?,
                        street: r.get(3)?,
                        live_action: r.get(4)?,
                        deep_action: r.get(5)?,
                        gap_bb: r.get(6)?,
                        pot_bb: r.get(7)?,
                        deep_ms: r.get(8)?,
                        samples: r.get::<_, i64>(9)? as usize,
                        replay_version: r.get::<_, Option<i64>>(10)?.map(|v| v as u32),
                    },
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Drop queued audits older than `queue_days` (the analyst was not running) and results older than
    /// `result_days`; returns rows removed. Callers keep results for [`AUDIT_RESULT_DAYS`]: a shorter
    /// window silently deletes evidence a reader is still measuring over.
    pub fn prune_audits(&self, queue_days: i64, result_days: i64) -> Result<usize> {
        let now = chrono::Utc::now();
        let conn = self.write_lock();
        let q = conn.execute("DELETE FROM audit_queue WHERE ts < ?1", params![(now - chrono::Duration::days(queue_days)).to_rfc3339()])?;
        let r =
            conn.execute("DELETE FROM decision_audit WHERE ts < ?1", params![(now - chrono::Duration::days(result_days)).to_rfc3339()])?;
        Ok(q + r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audit_jobs_move_from_the_queue_to_results_atomically() {
        let dir = std::env::temp_dir().join(format!("sv10-store-audit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        store.insert_audit("A", "h1", "{}", Some(("d1", "[1]"))).unwrap();
        store.insert_audit("A", "h2", "{}", None).unwrap();
        let jobs = store.audit_batch(10).unwrap();
        assert_eq!(
            jobs.iter().map(|j| (j.hand_id.as_str(), j.net_digest.as_deref())).collect::<Vec<_>>(),
            [("h1", Some("d1")), ("h2", None)]
        );
        assert_eq!(store.replay_net("d1").unwrap().as_deref(), Some("[1]"));
        let result = |live: &str, gap: f64| AuditResult {
            bot: "A".into(),
            hand_id: "h1".into(),
            street: "flop".into(),
            live_action: live.into(),
            deep_action: "call".into(),
            gap_bb: gap,
            pot_bb: 10.0,
            deep_ms: 200.0,
            samples: 1_000_000,
            // Deliberately left unset here: the version is the analyst's to state, and a verdict that
            // does not name one must read back as "not recorded", not as version 0.
            replay_version: None,
        };
        store.finish_audit(jobs[0].id, Some(&result("call", 0.0))).unwrap();
        store.finish_audit(jobs[1].id, Some(&result("fold", 3.0))).unwrap();
        let s = store.audit_summary("2000-01-01T00:00:00Z").unwrap();
        assert_eq!((s.decisions, s.same_action, s.queued), (2, 1, 0));
        assert!((s.mean_gap_bb - 1.5).abs() < 1e-9 && (s.max_gap_bb - 3.0).abs() < 1e-9);
        // An unreadable job leaves the queue without a result.
        store.insert_audit("A", "h3", "not json", None).unwrap();
        let bad = store.audit_batch(1).unwrap();
        store.finish_audit(bad[0].id, None).unwrap();
        assert_eq!(store.audit_summary("2000-01-01T00:00:00Z").unwrap().decisions, 2);
        assert_eq!(store.prune_audits(1, 30).unwrap(), 0);
        // A network a queued audit still needs survives replay pruning.
        store.insert_audit("A", "h4", "{}", Some(("d2", "[2]"))).unwrap();
        store.prune_replays(14).unwrap();
        assert_eq!(store.replay_net("d2").unwrap().as_deref(), Some("[2]"));
        assert_eq!(store.replay_net("d1").unwrap(), None);
        // 0316: the graded record's replay version survives the round trip, and an unstated one reads
        // back as `None` — never as a version a filter would then match.
        let mut versioned = result("raise", 1.5);
        versioned.replay_version = Some(3);
        store.insert_audit("A", "h5", "{}", None).unwrap();
        let job = store.audit_batch(1).unwrap().pop().unwrap();
        store.finish_audit(job.id, Some(&versioned)).unwrap();
        let graded = store.audit_results_since("2000-01-01T00:00:00Z").unwrap();
        assert_eq!(graded.iter().map(|(_, r)| r.replay_version).collect::<Vec<_>>(), [None, None, Some(3)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// If the queue half of `insert_audit` fails, the network half must roll back with it. This
    /// catches a frontier-like partial commit where later work appears durable but cannot run.
    #[test]
    fn failed_audit_queue_insert_does_not_commit_its_network() {
        let dir = std::env::temp_dir().join(format!("sv10-store-audit-rollback-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        store
            .conn
            .lock()
            .execute_batch(
                "CREATE TRIGGER reject_audit BEFORE INSERT ON audit_queue BEGIN SELECT RAISE(ABORT, 'injected queue failure'); END;",
            )
            .unwrap();

        let error = store.insert_audit("A", "h1", "{}", Some(("candidate-net", "[1]"))).expect_err("trigger rejects queue insert");

        assert!(error.to_string().contains("injected queue failure"));
        assert_eq!(store.replay_net("candidate-net").unwrap(), None, "network insert rolled back with queue insert");
        assert!(store.audit_batch(10).unwrap().is_empty(), "no partial queue row");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

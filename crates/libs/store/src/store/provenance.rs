//! Distribution provenance of hands played under experiment mode (0291): which arm of which
//! evidence target a hand belongs to.
//!
//! A hand with no row here is ordinary play. A `treatment` hand was played by a learner challenger,
//! so the production fits (opponent models, neural and range data, the decision-based live fits)
//! skip it through [`ordinary_hand`]; a `control` hand is champion play and stays in them. The
//! same condition gates [`Store::ordinary_results_after`], the head-to-head ledger's read (0361):
//! that number steers table selection, and a challenger's hands are not evidence about the policy
//! that plays. The dashboard's results and the season race read every hand: treatment chips are
//! real chips.
use super::*;
use anyhow::Result;
use rusqlite::params;

/// The arm that played a learner challenger.
pub const TREATMENT_ARM: &str = "treatment";
/// The arm that played the champion as the experiment's control.
pub const CONTROL_ARM: &str = "control";

/// SQL condition that holds unless hand (`bot`, `hand_id`) was a treatment-arm hand. Both
/// arguments are SQL expressions from the enclosing query (for example `hands.bot`).
pub fn ordinary_hand(bot: &str, hand_id: &str) -> String {
    format!("NOT EXISTS (SELECT 1 FROM hand_provenance p WHERE p.bot = {bot} AND p.hand_id = {hand_id} AND p.arm = '{TREATMENT_ARM}')")
}

/// The provenance a hand carries into the store.
#[derive(Clone, Debug, PartialEq)]
pub struct HandTag {
    /// Evidence target id.
    pub target: String,
    /// [`TREATMENT_ARM`] or [`CONTROL_ARM`].
    pub arm: String,
    /// The full provenance record as JSON (versions, digests, assignment).
    pub record: String,
}

/// Hands of one arm of one target: (target, arm, hands, first ts, last ts).
pub type TargetArmSpan = (String, String, i64, String, String);

/// One experiment hand with its result, for the live estimate and the audit listing.
#[derive(Clone, Debug, PartialEq)]
pub struct ArmHand {
    /// Our bot.
    pub bot: String,
    /// Server hand id.
    pub hand_id: String,
    /// [`TREATMENT_ARM`] or [`CONTROL_ARM`].
    pub arm: String,
    /// When the provenance was stored (RFC 3339).
    pub ts: String,
    /// Net chips, when the hand is stored with one.
    pub net: Option<i64>,
    /// All-in luck-adjusted net, when filled.
    pub ev_net: Option<f64>,
    /// Big blind from the hand summary.
    pub bb: Option<i64>,
}

impl Store {
    /// Store a hand and, for an experiment hand, its provenance in one transaction, so no reader
    /// ever sees a treatment hand without its tag. Returns the hand's rowid.
    pub fn insert_hand_tagged(&self, h: &HandRow, tag: Option<&HandTag>) -> Result<i64> {
        let conn = self.write_lock();
        let tx = conn.unchecked_transaction()?;
        if let Some(tag) = tag {
            insert_tag(&tx, &h.bot, &h.hand_id, tag)?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO hands (bot, hand_id, table_id, ended_at, hero_seat, hole, board, pot, net, winners, summary, showdown, digest)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![h.bot, h.hand_id, h.table_id, h.ended_at, h.hero_seat, h.hole, h.board, h.pot, h.net, h.winners, h.summary, h.showdown as i64,
                crate::integrity::hand_digest(&h.bot, &h.hand_id, &h.ended_at, &h.hole, &h.board, &h.summary)],
        )?;
        let rowid = tx.last_insert_rowid();
        tx.commit()?;
        Ok(rowid)
    }

    /// Store only the provenance of a hand whose row is still queued for a retry.
    pub fn tag_hand(&self, bot: &str, hand_id: &str, tag: &HandTag) -> Result<()> {
        insert_tag(&self.write_lock(), bot, hand_id, tag)
    }

    /// Every hand of evidence target `target`, oldest first, with its result.
    pub fn target_hands(&self, target: &str) -> Result<Vec<ArmHand>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT p.bot, p.hand_id, p.arm, p.ts, h.net, h.ev_net,
                    CASE WHEN json_valid(h.summary) THEN json_extract(h.summary, '$.bb') END
             FROM hand_provenance p LEFT JOIN hands h ON h.bot = p.bot AND h.hand_id = p.hand_id
             WHERE p.target = ?1 ORDER BY p.ts, p.bot, p.hand_id",
        )?;
        let rows = stmt
            .query_map(params![target], |r| {
                Ok(ArmHand {
                    bot: r.get(0)?,
                    hand_id: r.get(1)?,
                    arm: r.get(2)?,
                    ts: r.get(3)?,
                    net: r.get(4)?,
                    ev_net: r.get(5)?,
                    bb: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The stored provenance record of one hand, if it was an experiment hand.
    pub fn hand_tag(&self, bot: &str, hand_id: &str) -> Result<Option<HandTag>> {
        use rusqlite::OptionalExtension;
        Ok(self
            .read()
            .query_row("SELECT target, arm, record FROM hand_provenance WHERE bot = ?1 AND hand_id = ?2", params![bot, hand_id], |r| {
                Ok(HandTag { target: r.get(0)?, arm: r.get(1)?, record: r.get(2)? })
            })
            .optional()?)
    }

    /// Every target with experiment hands: (target, arm, hands, first ts, last ts).
    pub fn experiment_targets(&self) -> Result<Vec<TargetArmSpan>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT target, arm, COUNT(*), MIN(ts), MAX(ts) FROM hand_provenance GROUP BY target, arm ORDER BY MIN(ts), target, arm",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Decision row ids and replay record ids of one hand, for the experiment audit.
    pub fn hand_audit_ids(&self, bot: &str, hand_id: &str) -> Result<(Vec<i64>, Vec<i64>)> {
        let conn = self.read();
        let ids = |sql: &str| -> Result<Vec<i64>> {
            let mut stmt = conn.prepare(sql)?;
            Ok(stmt.query_map(params![bot, hand_id], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?)
        };
        Ok((
            ids("SELECT id FROM decisions WHERE bot = ?1 AND hand_id = ?2 ORDER BY id")?,
            ids("SELECT id FROM replays WHERE bot = ?1 AND hand_id = ?2 ORDER BY id")?,
        ))
    }

    /// (bot, hand_id) of every treatment-arm hand: the other side of [`ordinary_hand`], for callers
    /// that hold whole hand rows instead of SQL expressions. Keyed by the pair, since one hand two
    /// of our seats shared has a row per seat and only the arm's seat is a treatment hand.
    pub fn treatment_hands(&self) -> Result<std::collections::HashSet<(String, String)>> {
        let conn = self.read();
        let mut stmt = conn.prepare("SELECT bot, hand_id FROM hand_provenance WHERE arm = ?1")?;
        Ok(stmt.query_map(params![TREATMENT_ARM], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?)
    }

    /// Newest `limit` hands of `bot` that the production fits may learn from: every hand except a
    /// treatment-arm one, newest first.
    pub fn recent_fit_hands(&self, bot: &str, limit: usize) -> Result<Vec<HandRow>> {
        let mut rows = self.recent_hands(bot, limit)?;
        let treated = self.treatment_hands()?;
        let arm = |h: &HandRow| (h.bot.clone(), h.hand_id.clone());
        rows.retain(|h| !treated.contains(&arm(h)));
        Ok(rows)
    }
}

fn insert_tag(conn: &Connection, bot: &str, hand_id: &str, tag: &HandTag) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO hand_provenance (bot, hand_id, target, arm, record, ts) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![bot, hand_id, tag.target, tag.arm, tag.record, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(tag: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("sv10-provenance-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store::open(&dir.join("t.db")).unwrap()
    }

    fn hand(id: &str, net: i64) -> HandRow {
        HandRow {
            bot: "A".into(),
            hand_id: id.into(),
            ended_at: format!("2026-09-27T00:00:0{}Z", id.len()),
            net: Some(net),
            summary: r#"{"bb":20}"#.into(),
            ..Default::default()
        }
    }

    fn tag(arm: &str) -> HandTag {
        HandTag { target: "t1".into(), arm: arm.into(), record: "{}".into() }
    }

    /// 0361: every read that steers behaviour leaves a treatment hand out — the fits, and the
    /// head-to-head ledger, whose verdict moves the fleet between tables. The result panels are the
    /// other side: there the chips *are* the answer, so they count every hand.
    #[test]
    fn treatment_hands_are_kept_out_of_the_reads_that_steer_play_but_not_out_of_results() {
        let s = store("fits");
        s.insert_hand_tagged(&hand("h1", 10), None).unwrap();
        s.insert_hand_tagged(&hand("h2", -40), Some(&tag(TREATMENT_ARM))).unwrap();
        s.insert_hand_tagged(&hand("h3", 20), Some(&tag(CONTROL_ARM))).unwrap();
        let fit: Vec<String> = s.recent_fit_hands("A", 10).unwrap().into_iter().map(|h| h.hand_id).collect();
        assert!(!fit.contains(&"h2".to_string()) && fit.len() == 2, "{fit:?}");
        assert_eq!(s.hands_after(0).unwrap().len(), 2, "the model tailer skips the treatment hand");
        assert_eq!(s.hands_page(0, i64::MAX, 10).unwrap().len(), 2, "the learner's population skips it");
        let ledger: Vec<Option<i64>> = s.ordinary_results_after(0).unwrap().into_iter().map(|(_, _, net, _, _, _)| net).collect();
        assert_eq!(ledger, [Some(10), Some(20)], "the ledger skips the treatment hand and keeps the control one, which is champion play");
        assert_eq!(s.recent_hands("A", 10).unwrap().len(), 3, "results still count every chip");
        assert_eq!(s.bot_results("A").unwrap().len(), 3);
        let arms: Vec<(String, Option<i64>, Option<i64>)> =
            s.target_hands("t1").unwrap().into_iter().map(|h| (h.arm, h.net, h.bb)).collect();
        assert_eq!(arms, vec![(TREATMENT_ARM.into(), Some(-40), Some(20)), (CONTROL_ARM.into(), Some(20), Some(20))]);
        assert_eq!(s.hand_tag("A", "h2").unwrap().map(|t| t.arm), Some(TREATMENT_ARM.to_string()));
        assert_eq!(s.hand_tag("A", "h1").unwrap(), None);
    }

    #[test]
    fn a_tag_stored_ahead_of_a_queued_hand_still_excludes_it() {
        let s = store("queued");
        s.tag_hand("A", "h9", &tag(TREATMENT_ARM)).unwrap();
        s.insert_hand(&hand("h9", 5)).unwrap();
        assert!(s.hands_after(0).unwrap().is_empty());
    }
}

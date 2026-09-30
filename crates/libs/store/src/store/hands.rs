//! Stored hands: rows, inserts, result backfills and hand queries.
use super::*;
use anyhow::Result;
use rusqlite::{OptionalExtension, params};

/// A stored hand as the API and tools read it.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct HandRow {
    /// Our bot that played it.
    pub bot: String,
    /// Server hand id.
    pub hand_id: String,
    /// Server table id.
    pub table_id: String,
    /// End time (RFC 3339).
    pub ended_at: String,
    /// Our seat.
    pub hero_seat: Option<i64>,
    /// Our hole cards as concatenated text (`"AhKd"`).
    pub hole: String,
    /// Board as concatenated text.
    pub board: String,
    /// Final pot.
    pub pot: i64,
    /// Our net chips, when known.
    pub net: Option<i64>,
    /// Winner names, comma-separated.
    pub winners: String,
    /// The `HandSummary` as JSON.
    pub summary: String,
    /// Whether it went to showdown (filled by callers; list queries leave it false).
    #[serde(default)]
    pub showdown: bool,
}

/// One of our hands a named player was dealt into (player cards, 0217).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerHand {
    /// Our bot that played it.
    pub bot: String,
    /// Server hand id.
    pub hand_id: String,
    /// End time (RFC 3339).
    pub ended_at: String,
    /// Our net chips, when known.
    pub net: Option<i64>,
    /// Our all-in EV net once filled (0213).
    pub ev_net: Option<f64>,
    /// Final pot.
    pub pot: i64,
    /// Winner names, comma-separated.
    pub winners: String,
}

/// (net, all-in EV net, showdown, ended_at) of one hand (0213).
pub type EvResult = (Option<i64>, Option<f64>, bool, String);

/// (rowid, bot, net chips, summary json, settled pot, comma-separated winner names).
pub type ResultRow = (i64, String, Option<i64>, String, i64, String);

/// Store one hand, updating the row in place when `(bot, hand_id)` is already there, and return the
/// rowid that identifies it to a watermark reader.
///
/// Shared by `insert_hand` and `insert_hand_tagged` so the two cannot drift, and an upsert rather
/// than `INSERT OR REPLACE` so a re-store keeps the rowid (#600).
pub(crate) fn store_hand(conn: &Connection, h: &HandRow) -> Result<i64> {
    Ok(conn.query_row(
        "INSERT INTO hands (bot, hand_id, table_id, ended_at, hero_seat, hole, board, pot, net, winners, summary, showdown, digest)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
         ON CONFLICT(bot, hand_id) DO UPDATE SET
             table_id = excluded.table_id, ended_at = excluded.ended_at, hero_seat = excluded.hero_seat,
             hole = excluded.hole, board = excluded.board, pot = excluded.pot, net = excluded.net,
             winners = excluded.winners, summary = excluded.summary, showdown = excluded.showdown,
             digest = excluded.digest
         RETURNING rowid",
        params![
            h.bot,
            h.hand_id,
            h.table_id,
            h.ended_at,
            h.hero_seat,
            h.hole,
            h.board,
            h.pot,
            h.net,
            h.winners,
            h.summary,
            h.showdown as i64,
            crate::integrity::hand_digest(&h.bot, &h.hand_id, &h.ended_at, &h.hole, &h.board, &h.summary)
        ],
        |r| r.get(0),
    )?)
}

impl Store {
    /// Recompute every hand's digest: (hands checked, (bot, hand_id) whose content changed).
    pub fn verify_hand_digests(&self) -> Result<(usize, Vec<(String, String)>)> {
        let conn = self.read();
        let mut stmt = conn.prepare("SELECT bot, hand_id, ended_at, COALESCE(hole, ''), COALESCE(board, ''), COALESCE(summary, ''), COALESCE(digest, '') FROM hands")?;
        let mut checked = 0;
        let mut bad = Vec::new();
        let rows =
            stmt.query_map([], |r| Ok([r.get::<_, String>(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?]))?;
        for row in rows {
            let [bot, id, ended, hole, board, summary, digest] = row?;
            checked += 1;
            if crate::integrity::hand_digest(&bot, &id, &ended, &hole, &board, &summary) != digest {
                bad.push((bot, id));
            }
        }
        Ok((checked, bad))
    }
    /// (net, showdown, ended_at) for every hand of a bot, oldest first, without summaries.
    pub fn bot_results(&self, bot: &str) -> Result<Vec<(Option<i64>, bool, String)>> {
        Ok(self.bot_ev_results(bot)?.into_iter().map(|(net, _, showdown, ended)| (net, showdown, ended)).collect())
    }
    /// (net, all-in EV net, showdown, ended_at) for every hand of a bot, oldest first; the EV net
    /// is the net until the background fill has reached the hand (0213).
    pub fn bot_ev_results(&self, bot: &str) -> Result<Vec<EvResult>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT net, COALESCE(ev_net, CAST(net AS REAL)), COALESCE(showdown, 0), ended_at FROM hands WHERE bot = ?1 ORDER BY rowid",
        )?;
        let rows = stmt
            .query_map(params![bot], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? != 0, r.get(3)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Every hand of any of our bots that `player` was dealt into, oldest first, without summaries.
    pub fn hands_with_player(&self, player: &str) -> Result<Vec<PlayerHand>> {
        // The summary lists players as `[seat,"name"]`, so `"name"]` matches that seat and no prefix of another name.
        let needle = format!("{}]", serde_json::Value::String(player.to_string()));
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT bot, hand_id, ended_at, net, ev_net, COALESCE(pot, 0), COALESCE(winners, '') FROM hands
             WHERE instr(summary, ?1) > 0 ORDER BY ended_at",
        )?;
        let rows = stmt
            .query_map(params![needle], |r| {
                Ok(PlayerHand {
                    bot: r.get(0)?,
                    hand_id: r.get(1)?,
                    ended_at: r.get(2)?,
                    net: r.get(3)?,
                    ev_net: r.get(4)?,
                    pot: r.get(5)?,
                    winners: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Up to `limit` hands with a net but no all-in EV net yet, oldest first, with summaries.
    pub fn hands_missing_ev(&self, limit: usize) -> Result<Vec<HandRow>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT bot, hand_id, table_id, ended_at, hero_seat, hole, board, pot, net, winners, summary
             FROM hands WHERE ev_net IS NULL AND net IS NOT NULL ORDER BY rowid LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                Ok(HandRow {
                    bot: r.get(0)?,
                    hand_id: r.get(1)?,
                    table_id: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    ended_at: r.get(3)?,
                    hero_seat: r.get(4)?,
                    hole: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    board: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
                    pot: r.get::<_, Option<i64>>(7)?.unwrap_or(0),
                    net: r.get(8)?,
                    winners: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
                    summary: r.get::<_, Option<String>>(10)?.unwrap_or_default(),
                    showdown: false,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Store all-in EV nets as (bot, hand_id, ev_net) in one transaction.
    pub fn set_ev_nets(&self, rows: &[(String, String, f64)]) -> Result<()> {
        let mut conn = self.write_lock();
        let tx = conn.transaction()?;
        {
            let mut stmt = tx.prepare("UPDATE hands SET ev_net = ?3 WHERE bot = ?1 AND hand_id = ?2")?;
            for (bot, hand_id, ev) in rows {
                stmt.execute(params![bot, hand_id, ev])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    /// Big blind of the most recently stored hand (from its summary), if any hand is stored.
    pub fn latest_big_blind(&self) -> Result<Option<i64>> {
        let conn = self.read();
        let bb = conn
            .query_row("SELECT json_extract(summary, '$.bb') FROM hands WHERE json_valid(summary) ORDER BY rowid DESC LIMIT 1", [], |r| {
                r.get::<_, Option<i64>>(0)
            })
            .optional()?
            .flatten();
        Ok(bb.filter(|b| *b > 0))
    }
    /// Recent hands without the (large) summary column.
    pub fn recent_hands_light(&self, bot: &str, limit: usize) -> Result<Vec<HandRow>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT bot, hand_id, table_id, ended_at, hero_seat, hole, board, pot, net, winners, COALESCE(showdown, 0)
             FROM hands WHERE bot = ?1 ORDER BY rowid DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![bot, limit as i64], |r| {
                Ok(HandRow {
                    bot: r.get(0)?,
                    hand_id: r.get(1)?,
                    table_id: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    ended_at: r.get(3)?,
                    hero_seat: r.get(4)?,
                    hole: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    board: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
                    pot: r.get::<_, Option<i64>>(7)?.unwrap_or(0),
                    net: r.get(8)?,
                    winners: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
                    summary: String::new(),
                    showdown: r.get::<_, i64>(10)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Insert a completed hand; returns its rowid (the model watermark unit).
    ///
    /// A hand that is already stored is *updated*, never replaced (#600): `INSERT OR REPLACE` deletes
    /// the conflicting row and inserts another, so a re-store — the retry path — moved the hand to the
    /// highest rowid and every reader that persisted a watermark saw it a second time.
    pub fn insert_hand(&self, h: &HandRow) -> Result<i64> {
        let conn = self.write_lock();
        store_hand(&conn, h)
    }
    /// (bot, hand_id) of hands whose net the tracker could not compute (joined mid-hand on a resync).
    pub fn hands_missing_net(&self) -> Result<Vec<(String, String)>> {
        let conn = self.read();
        let mut stmt = conn.prepare("SELECT bot, hand_id FROM hands WHERE net IS NULL")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Set a missing net in place (rowid kept); a tracked net is never overwritten.
    pub fn fill_net(&self, bot: &str, hand_id: &str, net: i64) -> Result<bool> {
        let n = self
            .conn
            .lock()
            .execute("UPDATE hands SET net = ?1 WHERE bot = ?2 AND hand_id = ?3 AND net IS NULL", params![net, bot, hand_id])?;
        Ok(n > 0)
    }
    /// Highest hand rowid (0 when empty), for model watermarks.
    pub fn max_hand_rowid(&self) -> Result<i64> {
        Ok(self.read().query_row("SELECT COALESCE(MAX(rowid), 0) FROM hands", [], |r| r.get(0))?)
    }
    /// (rowid, bot, summary json) for hands stored after `rowid`, oldest first; treatment-arm hands
    /// are left out (they are not the policy the models describe).
    pub fn hands_after(&self, rowid: i64) -> Result<Vec<(i64, String, String)>> {
        let conn = self.read();
        // Treatment-arm hands (0291) never reach the opponent models.
        let mut stmt = conn.prepare(&format!(
            "SELECT rowid, bot, summary FROM hands WHERE rowid > ?1 AND {} ORDER BY rowid",
            ordinary_hand("hands.bot", "hands.hand_id")
        ))?;
        let rows = stmt
            .query_map(params![rowid], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, Option<String>>(2)?.unwrap_or_default())))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// (rowid, hand_id, summary json) for at most `limit` hands with `after < rowid <= upto`,
    /// oldest first: a long replay reads it page by page (a hand two of our seats shared has a row per seat).
    /// Treatment-arm hands are left out, as in [`Store::hands_after`].
    pub fn hands_page(&self, after: i64, upto: i64, limit: usize) -> Result<Vec<(i64, String, String)>> {
        let conn = self.read();
        let mut stmt = conn.prepare(&format!(
            "SELECT rowid, hand_id, COALESCE(summary, '') FROM hands WHERE rowid > ?1 AND rowid <= ?2 AND {} ORDER BY rowid LIMIT ?3",
            ordinary_hand("hands.bot", "hands.hand_id")
        ))?;
        let rows = stmt
            .query_map(params![after, upto, limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// The first hand row that ended at or after `ts` (RFC 3339): where a season starts.
    pub fn first_hand_since(&self, ts: &str) -> Result<Option<i64>> {
        Ok(self.read().query_row("SELECT MIN(rowid) FROM hands WHERE ended_at >= ?1", params![ts], |r| r.get(0))?)
    }
    /// When hand row `rowid` ended.
    pub fn hand_time(&self, rowid: i64) -> Result<Option<String>> {
        Ok(self.read().query_row("SELECT ended_at FROM hands WHERE rowid = ?1", params![rowid], |r| r.get(0)).optional()?)
    }
    /// Hands stored after `rowid` with their result: (rowid, bot, net, summary, pot, winners),
    /// oldest first, treatment-arm hands left out (0361).
    ///
    /// The head-to-head ledger is the only caller, and its claim — this opponent beats us — is
    /// about the policy that plays, so a learner challenger's hands are not evidence for it; a
    /// control hand is champion play and stays, as it does in every other population. This is the
    /// one results read that filters: [`Store::bot_results`], [`Store::recent_hands`] and
    /// [`Store::bot_ev_results`] count every chip, treatment arms included, because for those the
    /// chips *are* the answer.
    pub fn ordinary_results_after(&self, rowid: i64) -> Result<Vec<ResultRow>> {
        let conn = self.read();
        let mut stmt = conn.prepare(&format!(
            "SELECT rowid, bot, net, COALESCE(summary, ''), COALESCE(pot, 0), COALESCE(winners, '') FROM hands
             WHERE rowid > ?1 AND {} ORDER BY rowid",
            ordinary_hand("hands.bot", "hands.hand_id")
        ))?;
        let rows = stmt
            .query_map(params![rowid], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// Every bot that has a stored hand: our own seats, derived from data rather than a list
    /// that can drift from the fleet configuration.
    pub fn bot_names(&self) -> Result<Vec<String>> {
        let conn = self.read();
        let mut stmt = conn.prepare("SELECT DISTINCT bot FROM hands ORDER BY bot")?;
        let names = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(names)
    }
    /// Newest `limit` hands of `bot`, newest first.
    pub fn recent_hands(&self, bot: &str, limit: usize) -> Result<Vec<HandRow>> {
        let conn = self.read();
        let mut stmt = conn.prepare(
            "SELECT bot, hand_id, table_id, ended_at, hero_seat, hole, board, pot, net, winners, summary
             FROM hands WHERE bot = ?1 ORDER BY ended_at DESC LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![bot, limit as i64], |r| {
                Ok(HandRow {
                    bot: r.get(0)?,
                    hand_id: r.get(1)?,
                    table_id: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    ended_at: r.get(3)?,
                    hero_seat: r.get(4)?,
                    hole: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                    board: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
                    pot: r.get::<_, Option<i64>>(7)?.unwrap_or(0),
                    net: r.get(8)?,
                    winners: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
                    summary: r.get::<_, Option<String>>(10)?.unwrap_or_default(),
                    showdown: false,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    /// One hand of `bot` by id.
    pub fn hand(&self, bot: &str, hand_id: &str) -> Result<Option<HandRow>> {
        self.recent_hands_where(bot, hand_id)
    }
    fn recent_hands_where(&self, bot: &str, hand_id: &str) -> Result<Option<HandRow>> {
        let conn = self.read();
        Ok(conn
            .query_row(
                "SELECT bot, hand_id, table_id, ended_at, hero_seat, hole, board, pot, net, winners, summary
                 FROM hands WHERE bot = ?1 AND hand_id = ?2",
                params![bot, hand_id],
                |r| {
                    Ok(HandRow {
                        bot: r.get(0)?,
                        hand_id: r.get(1)?,
                        table_id: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                        ended_at: r.get(3)?,
                        hero_seat: r.get(4)?,
                        hole: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                        board: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
                        pot: r.get::<_, Option<i64>>(7)?.unwrap_or(0),
                        net: r.get(8)?,
                        winners: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
                        summary: r.get::<_, Option<String>>(10)?.unwrap_or_default(),
                        showdown: false,
                    })
                },
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn latest_big_blind_comes_from_the_newest_hand_summary() {
        let dir = std::env::temp_dir().join(format!("sv10-store-bb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        assert_eq!(store.latest_big_blind().unwrap(), None);
        let hand = |id: &str, summary: &str| HandRow {
            bot: "A".into(),
            hand_id: id.into(),
            table_id: "t".into(),
            ended_at: "2026-09-16T00:00:00Z".into(),
            hero_seat: Some(0),
            hole: String::new(),
            board: String::new(),
            pot: 0,
            net: Some(0),
            winners: String::new(),
            summary: summary.into(),
            showdown: false,
        };
        store.insert_hand(&hand("h1", r#"{"bb":20}"#)).unwrap();
        store.insert_hand(&hand("h2", r#"{"bb":50}"#)).unwrap();
        assert_eq!(store.latest_big_blind().unwrap(), Some(50));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hands_page_reads_a_bounded_window_in_row_order() {
        // 0244: the pacing study replays ~110k summaries; pages keep that out of memory at once.
        let dir = std::env::temp_dir().join(format!("sv10-store-page-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        for i in 1..=5 {
            let h = HandRow {
                bot: "A".into(),
                hand_id: format!("h{i}"),
                ended_at: format!("2026-09-2{i}T00:00:00Z"),
                summary: format!("s{i}"),
                ..Default::default()
            };
            store.insert_hand(&h).unwrap();
        }
        let page = store.hands_page(1, 4, 2).unwrap();
        assert_eq!(page.iter().map(|(r, _, s)| (*r, s.as_str())).collect::<Vec<_>>(), [(2, "s2"), (3, "s3")]);
        assert_eq!(store.hands_page(3, 4, 10).unwrap().len(), 1, "stops at the upper row");
        assert!(store.hands_page(5, 99, 10).unwrap().is_empty());
        assert_eq!(store.first_hand_since("2026-09-23T12:00:00Z").unwrap(), Some(4));
        assert_eq!(store.first_hand_since("2026-09-30T00:00:00Z").unwrap(), None);
        assert_eq!(store.hand_time(2).unwrap().as_deref(), Some("2026-09-22T00:00:00Z"));
        assert_eq!(store.hand_time(9).unwrap(), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ev_nets_are_filled_in_place_and_read_back_with_the_results() {
        // 0213: an EV net stands in for the net only once stored; a hand without a net is never listed.
        let dir = std::env::temp_dir().join(format!("sv10-store-ev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        for (id, net) in [("h1", Some(-400)), ("h2", Some(120)), ("h3", None)] {
            store
                .insert_hand(&HandRow {
                    bot: "A".into(),
                    hand_id: id.into(),
                    ended_at: format!("2026-09-24T00:00:0{}Z", &id[1..]),
                    net,
                    ..Default::default()
                })
                .unwrap();
        }
        let missing: Vec<String> = store.hands_missing_ev(10).unwrap().into_iter().map(|h| h.hand_id).collect();
        assert_eq!(missing, ["h1", "h2"]);
        assert_eq!(store.bot_ev_results("A").unwrap()[0].1, Some(-400.0), "the net until filled");
        store.set_ev_nets(&[("A".into(), "h1".into(), 180.5)]).unwrap();
        let rows = store.bot_ev_results("A").unwrap();
        assert_eq!((rows[0].0, rows[0].1), (Some(-400), Some(180.5)));
        assert_eq!(rows[2].1, None);
        assert_eq!(store.hands_missing_ev(10).unwrap().len(), 1);
        assert_eq!(store.bot_results("A").unwrap()[0].0, Some(-400), "plain results unchanged");
        // Player lookups match whole names only.
        let with = |id: &str, players: &str| HandRow {
            bot: "B".into(),
            hand_id: id.into(),
            ended_at: format!("2026-09-25T00:00:0{}Z", &id[1..]),
            net: Some(5),
            summary: format!(r#"{{"players":{players}}}"#),
            ..Default::default()
        };
        store.insert_hand(&with("p1", r#"[[0,"B"],[3,"one"]]"#)).unwrap();
        store.insert_hand(&with("p2", r#"[[0,"B"],[2,"someone"]]"#)).unwrap();
        let hands = store.hands_with_player("one").unwrap();
        assert_eq!(hands.iter().map(|h| h.hand_id.as_str()).collect::<Vec<_>>(), ["p1"]);
        assert_eq!((hands[0].bot.as_str(), hands[0].net), ("B", Some(5)));
        // A name with a quote is matched as its JSON escape, and no prefix of it matches.
        store.insert_hand(&with("p3", r#"[[0,"B"],[1,"O\"Brien"]]"#)).unwrap();
        assert_eq!(store.hands_with_player("O\"Brien").unwrap().iter().map(|h| h.hand_id.as_str()).collect::<Vec<_>>(), ["p3"]);
        assert!(store.hands_with_player("O").unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

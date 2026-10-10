//! Past-season data: download every hand our bots ever played from the server's hand-history
//! export into `artifacts/history.db`, replay them, and fold them into the opponent models.
//!
//! The history database is re-downloadable, so it lives outside the backed-up main database.
//! Imports are crash-safe the same way live hands are: `ModelStore::history_watermark` is
//! checkpointed together with the stats, and anything above it is re-imported on restart.

use std::time::Duration;

// Database access, import and download, split by stage (0260); the names below are
// re-exported so every caller keeps its path (`sv10_bot::history::HistoryDb` and friends
// are unchanged).
pub mod db;
pub mod download;
pub mod import;

pub use db::{CorpusRow, HistoryDb, STATUS_KEY, open};
pub use download::run;
pub use import::{ImportReport, fill_missing_nets, import};

pub(super) const PAGE: i64 = 200;
/// Minimum spacing between export requests (the endpoint allows 30/minute).
pub(crate) const REQUEST_GAP: Duration = Duration::from_millis(2600);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::Shared;
    use serde_json::json;
    use sv10_core::model::HandSummary;
    use sv10_store::store::{HandRow, Store};

    #[test]
    fn capped_backfills_resume_once_the_cap_is_gone() {
        assert!(download::resume_capped_backfill(true, 20_017, 123_693, 0), "pro key below its total resumes");
        assert!(!download::resume_capped_backfill(true, 123_693, 123_693, 0), "a truly complete backfill stays done");
        assert!(!download::resume_capped_backfill(true, 20_017, 123_693, 20_000), "a capped key stays stopped");
        assert!(!download::resume_capped_backfill(false, 5_000, 123_693, 0), "no marker, nothing to clear");
        assert!(!download::resume_capped_backfill(true, 5_000, 0, 0), "unknown total never resumes");
    }

    #[test]
    fn backfill_stops_at_the_export_cap() {
        assert!(!download::beyond_export_cap(19_900, 20_000));
        assert!(download::beyond_export_cap(20_000, 20_000));
        assert!(download::beyond_export_cap(20_137, 20_000));
        assert!(!download::beyond_export_cap(1_000_000, 0), "0 means unlimited (Pro)");
    }

    fn live(bot: &str, hand_id: &str, net: Option<i64>) -> HandRow {
        HandRow {
            bot: bot.into(),
            hand_id: hand_id.into(),
            table_id: "t".into(),
            ended_at: "2026-09-14T20:05:00Z".into(),
            hero_seat: Some(1),
            hole: String::new(),
            board: String::new(),
            pot: 100,
            net,
            winners: String::new(),
            summary: String::new(),
            showdown: false,
        }
    }

    #[test]
    fn training_summaries_prefer_full_corpus_hands() {
        let dir = std::env::temp_dir().join(format!("sv10-corpus-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = HistoryDb::open(&dir.join("history.db")).unwrap();
        let summary = |bb: i64| {
            serde_json::to_string(&HandSummary {
                players: vec![(0, "A".into())],
                button: 0,
                bb,
                history: vec![],
                board: vec![],
                shown: vec![],
                stacks: vec![(0, 2000)],
            })
            .unwrap()
        };
        let export = |id: &str, t: &str| json!({"hand_id": id, "table_id": "t", "hand_number": 1, "started_at": t});
        db.insert_page("A", &[export("h1", "2026-09-10T00:00:00Z"), export("h2", "2026-09-10T00:01:00Z")]).unwrap();
        db.set_summaries(&[(1, summary(20)), (2, summary(20))]).unwrap();
        let row = |id: &str, t: &str, bb| CorpusRow {
            hand_id: id.into(),
            bot: "A".into(),
            table_id: String::new(),
            started_at: t.into(),
            summary: summary(bb),
        };
        // h1 gains a full-detail version; h3 exists only in the corpus; h4 is another venue.
        db.insert_corpus(
            "openpoker-archive-frames",
            &[row("h1", "2026-09-10T00:00:00Z", 40), row("h3", "2026-09-10T00:02:00Z", 40)],
            ("wm", "1"),
        )
        .unwrap();
        db.insert_corpus("phh-handhq", &[row("h4", "2026-09-10T00:03:00Z", 100)], ("wm2", "1")).unwrap();
        let got: Vec<i64> = db.recent_summaries(10, true).unwrap().into_iter().map(|(_, h)| h.bb).collect();
        assert_eq!(got, vec![40, 20, 40]);
        let exports: Vec<i64> = db.recent_summaries(10, false).unwrap().into_iter().map(|(_, h)| h.bb).collect();
        assert_eq!(exports, vec![20, 20]);
        // Re-inserting is a no-op and every row still matches its digest.
        assert_eq!(db.insert_corpus("openpoker-archive-frames", &[row("h1", "2026-09-10T00:00:00Z", 40)], ("wm", "1")).unwrap(), (0, 1));
        assert_eq!(db.verify_corpus().unwrap(), (3, vec![]));
    }

    #[test]
    fn missing_live_nets_are_filled_from_server_profit() {
        let dir = std::env::temp_dir().join(format!("sv10-fillnet-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir.join("live.db")).unwrap();
        let db = HistoryDb::open(&dir.join("history.db")).unwrap();
        store.insert_hand(&live("A", "h1", None)).unwrap();
        store.insert_hand(&live("A", "h2", Some(-40))).unwrap();
        store.insert_hand(&live("A", "h3", None)).unwrap();
        store.insert_hand(&live("B", "h1", None)).unwrap();
        let rowid_before = store.max_hand_rowid().unwrap();
        db.insert_page("A", &[json!({"hand_id": "h1", "profit": 250}), json!({"hand_id": "h2", "profit": 999})]).unwrap();

        assert_eq!(fill_missing_nets(&store, &db).unwrap(), 1);
        let nets: Vec<Option<i64>> = ["h1", "h2", "h3"].iter().map(|h| store.hand("A", h).unwrap().unwrap().net).collect();
        // Filled from the server; a tracked net is never overwritten; no server row stays missing;
        // another bot's hand with the same id is untouched.
        assert_eq!(nets, vec![Some(250), Some(-40), None]);
        assert_eq!(store.hand("B", "h1").unwrap().unwrap().net, None);
        // Updated in place: model watermarks (rowids) are unaffected.
        assert_eq!(store.max_hand_rowid().unwrap(), rowid_before);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Regression test for 0087: when `insert_page` fails, `store_page` must propagate the
    /// error so the download stages in `run()` leave the frontier untouched and retry on the
    /// next pass. A failed page must never be silently skipped (which would permanently drop
    /// up to 200 hands per failure on a Pro key with ~100k hands).
    #[tokio::test]
    async fn store_page_failure_does_not_advance_frontier() {
        let dir = std::env::temp_dir().join(format!("sv10-history-fail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = HistoryDb::open(&dir.join("history.db")).unwrap();
        let db_arc = std::sync::Arc::new(db);

        // Simulate a saved backfill frontier.
        db_arc.set_meta("offset", "100");
        db_arc.set_meta("total", "100");
        assert_eq!(db_arc.meta("offset"), Some("100".to_string()));

        // Force insert_page to fail by dropping the raw table.
        db_arc.conn.lock().execute_batch("DROP TABLE raw;").unwrap();

        // store_page must fail rather than swallow the error.
        let page = json!({"hands": [{"hand_id": "h1", "table_id": "t", "hand_number": 1, "started_at": "2026-09-10T00:00:00Z"}]});
        let result = download::store_page(&db_arc, "A", &page).await;
        assert!(result.is_err(), "store_page should propagate insert_page failure");

        // The frontier must not have moved.
        assert_eq!(db_arc.meta("offset"), Some("100".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_counts_unparseable_rows_and_reports_discarded_passes() {
        let shared = Shared::for_test("import", &["A"]);
        let dir = std::env::temp_dir().join(format!("sv10-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = HistoryDb::open(&dir.join("history.db")).unwrap();
        // One well-formed export row and two missing the required `seat` (a format change).
        let good = json!({"hand_id": "h1", "table_id": "t", "hand_number": 1, "seat": 0, "dealer_seat": 0, "started_at": "2026-09-10T00:00:00Z",
                          "all_actions": [], "board": []});
        let bad = |id: &str, n: i64| json!({"hand_id": id, "table_id": "t", "hand_number": n, "started_at": "2026-09-10T00:00:00Z"});
        db.insert_page("A", &[good, bad("h2", 2), bad("h3", 3)]).unwrap();
        let r = import(&shared, &db).unwrap();
        assert_eq!(r.unparseable, 2, "{r:?}");
        assert!(r.merged);
        assert!(!r.format_alarm(), "two bad rows are noise, not an alarm");
        assert_eq!(shared.models.read().history_watermark, Some(db.max_id()));
        // Nothing new: nothing counted again.
        let again = import(&shared, &db).unwrap();
        assert_eq!((again.unparseable, again.hands), (0, 0));
        let alarm = ImportReport { unparseable: 30, hands: 5, ..Default::default() };
        assert!(alarm.format_alarm());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corpus_hand_is_counted_once_when_its_raw_row_arrives_later() {
        // #923: the corpus pass counted a hand whose raw row had not arrived; the raw pass counted the
        // same hand again when that row arrived. The bad row only makes the first pass run.
        let shared = Shared::for_test("corpus-once", &["A"]);
        let dir = std::env::temp_dir().join(format!("sv10-corpus-once-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = HistoryDb::open(&dir.join("history.db")).unwrap();
        let summary = serde_json::to_string(&HandSummary {
            players: vec![(0, "A".into())],
            button: 0,
            bb: 20,
            history: vec![],
            board: vec![],
            shown: vec![],
            stacks: vec![(0, 2000)],
        })
        .unwrap();
        let corpus = CorpusRow {
            hand_id: "h1".into(),
            bot: "A".into(),
            table_id: String::new(),
            started_at: "2026-09-10T00:00:00Z".into(),
            summary,
        };
        db.insert_corpus("openpoker-archive-frames", &[corpus], ("wm", "1")).unwrap();
        db.insert_page("A", &[json!({"hand_id": "h9", "table_id": "t", "hand_number": 9, "started_at": "2026-09-10T00:00:00Z"})]).unwrap();
        let first = import(&shared, &db).unwrap();
        assert_eq!(first.hands, 1, "the corpus pass counts the hand once: {first:?}");
        let good = json!({"hand_id": "h1", "table_id": "t", "hand_number": 1, "seat": 0, "dealer_seat": 0, "started_at": "2026-09-10T00:00:00Z",
                          "all_actions": [], "board": []});
        db.insert_page("A", &[good]).unwrap();
        let second = import(&shared, &db).unwrap();
        assert_eq!(first.hands + second.hands, 1, "the raw row must not count the hand again: {second:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_page_without_a_hands_list_is_not_an_empty_page() {
        // #937: a 200 body that is not a page read as zero hands, which ends the download as done for
        // good. An honest empty page is still an empty page.
        let dir = std::env::temp_dir().join(format!("sv10-history-page-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = std::sync::Arc::new(HistoryDb::open(&dir.join("history.db")).unwrap());
        assert!(download::store_page(&db, "A", &json!({"detail": "maintenance"})).await.is_err());
        assert_eq!(download::store_page(&db, "A", &json!({"hands": []})).await.unwrap().2, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

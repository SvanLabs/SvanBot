use super::*;
use std::sync::{Arc, mpsc};

#[test]
fn terminal_history_compaction_scan_does_not_wait_for_the_importer_mutex() {
    let dir = std::env::temp_dir().join(format!("sv10-history-reader-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = Arc::new(HistoryDb::open(&dir.join("history.db")).unwrap());
    let rows: Vec<Value> = (0..200).map(|i| serde_json::json!({"hand_id":format!("h{i}"),"profit":10})).collect();
    assert_eq!(db.insert_page("A", &rows).unwrap().0, 200);
    assert_eq!(db.compact(&mut [Some(0), None, None], 300).unwrap(), 200);
    assert!(db.codec.has_dictionary(sv10_store::packed::RAW_JSON));
    assert_eq!(serde_json::from_str::<Value>(&db.export_json("h199").unwrap().unwrap()).unwrap(), rows[199]);
    assert_eq!(db.profit("A", "h199"), Some(10));
    let held = db.conn.lock();
    let (send, receive) = mpsc::channel();
    std::thread::scope(|scope| {
        let worker = db.clone();
        scope.spawn(move || {
            let mut cursor = [Some(0), None, None];
            send.send((worker.compact(&mut cursor, 300), cursor)).unwrap();
        });
        let result = receive.recv_timeout(Duration::from_secs(1));
        drop(held);
        let (packed, cursor) = result.expect("terminal history scan waited for the importer mutex");
        assert_eq!(packed.unwrap(), 0);
        assert_eq!(cursor, [None; 3]);
    });
}

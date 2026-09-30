use super::*;
use std::sync::{Arc, mpsc};
use std::time::Duration;

#[test]
fn terminal_compaction_scan_does_not_wait_for_the_writer_mutex() {
    let dir = std::env::temp_dir().join(format!("sv10-compaction-reader-{}", std::process::id()));
    let store = Arc::new(Store::open(&dir.join("svanbot10.db")).unwrap());
    for i in 0..200 {
        store.insert_decision("A", &format!("h{i}"), "flop", "call", None, None, 100, 20, 1.0, "{\"opponents\":1}").unwrap();
    }
    let mut cursor = [Some(0), None, None];
    assert_eq!(store.compact(&mut cursor, 300).unwrap(), 200);
    assert!(store.codec.has_dictionary(crate::packed::DECISION_DETAIL));
    let held = store.write_lock();
    let (send, receive) = mpsc::channel();
    std::thread::scope(|scope| {
        let worker = store.clone();
        scope.spawn(move || {
            let mut cursor = [Some(0), None, None];
            send.send((worker.compact(&mut cursor, 300), cursor)).unwrap();
        });
        let result = receive.recv_timeout(Duration::from_secs(1));
        drop(held);
        let (packed, cursor) = result.expect("a terminal reader scan waited for the writer mutex");
        assert_eq!(packed.unwrap(), 0);
        assert_eq!(cursor, [None; 3]);
    });
}

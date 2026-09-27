//! Tests for the packed columns (0229, 0322), split out of `packed.rs` (the 500-line rule).

use super::*;

fn db(name: &str) -> (Connection, PathBuf) {
    let dir = std::env::temp_dir().join(format!("sv10-packed-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TABLE replays (id INTEGER PRIMARY KEY, record TEXT)").unwrap();
    (conn, path)
}

fn record(i: usize) -> String {
    format!(
        r#"{{"bot":"svan{}","hand_id":"h{i}","street":"flop","candidates":[{{"action":"call","ev":{:.3}}},{{"action":"fold","ev":0}}],"pot":{}}}"#,
        i % 5,
        i as f64 * 0.37,
        100 + i
    )
}

/// 0322: a writer packs before it takes the write lock. Without loaded dictionaries there is nothing
/// to pack with (`None`: pack under the lock, which loads them); once loaded, the cached pack is the
/// same frame the connected one makes, and it reads back.
#[test]
fn a_cached_pack_is_the_connected_pack_once_dictionaries_are_loaded() {
    let (conn, path) = db("cached");
    let codec = Codec::open(&conn, &path).unwrap();
    let text = record(3);
    let connected = codec.pack(&conn, REPLAY_RECORD, &text);
    assert_eq!(codec.pack_cached(REPLAY_RECORD, &text).as_deref(), Some(connected.as_slice()));
    assert_eq!(codec.text(ValueRef::Blob(&connected)).unwrap(), text);
    let fresh = Codec { dicts: Default::default(), ..codec };
    assert_eq!(fresh.pack_cached(REPLAY_RECORD, &text), None, "nothing loaded: the caller packs under its connection");
}

#[test]
fn both_forms_read_back_and_compaction_trains_then_packs_everything() {
    let (conn, path) = db("compact");
    let codec = Codec::open(&conn, &path).unwrap();
    for i in 0..400 {
        conn.execute("INSERT INTO replays (record) VALUES (?1)", [record(i)]).unwrap();
    }
    // A frame without a dictionary, written before one existed, reads back too.
    let early = codec.pack(&conn, REPLAY_RECORD, &record(1000));
    conn.execute("INSERT INTO replays (record) VALUES (?1)", [&early]).unwrap();
    assert_eq!(sv10_pack::dictionary_id(&early), Some(0));
    assert_eq!(text_rows(&conn, REPLAY_RECORD).unwrap(), 400);

    assert_eq!(codec.train(&conn, REPLAY_RECORD).unwrap(), Some(1));
    assert_eq!(codec.train(&conn, REPLAY_RECORD).unwrap(), None, "one dictionary per family");
    let (mut after, mut packed) = (0, 0);
    while let (n, Some(last)) = compact_batch(&conn, &codec, REPLAY_RECORD, after, 64).unwrap() {
        packed += n;
        after = last;
    }
    assert_eq!(packed, 401, "every text row and the dictionary-less frame");
    assert_eq!(text_rows(&conn, REPLAY_RECORD).unwrap(), 0);
    let ids: Vec<Option<u8>> = conn
        .prepare("SELECT record FROM replays ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok(sv10_pack::dictionary_id(r.get_ref(0)?.as_blob()?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(ids.iter().all(|id| *id == Some(1)), "all packed against the trained dictionary");

    // Every row reads back to its text, through a second codec as another process would.
    let other = Codec::open(&Connection::open(&path).unwrap(), &path).unwrap();
    let texts: Vec<String> = conn
        .prepare("SELECT record FROM replays ORDER BY id")
        .unwrap()
        .query_map([], |r| other.text(r.get_ref(0)?))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert_eq!(texts.len(), 401);
    for (i, t) in texts.iter().take(400).enumerate() {
        assert_eq!(t, &record(i));
    }
    assert_eq!(texts[400], record(1000));
    let stored: i64 = conn.query_row("SELECT SUM(length(record)) FROM replays", [], |r| r.get(0)).unwrap();
    let text: usize = texts.iter().map(|t| t.len()).sum();
    assert!((stored as f64) < 0.5 * text as f64, "packed {stored} of {text} bytes");

    // And back to text for an older build.
    assert_eq!(unpack_column(&conn, &codec, REPLAY_RECORD).unwrap(), 401);
    assert_eq!(text_rows(&conn, REPLAY_RECORD).unwrap(), 401);
    let first: String = conn.query_row("SELECT record FROM replays ORDER BY id LIMIT 1", [], |r| r.get(0)).unwrap();
    assert_eq!(first, record(0));
}

#[test]
fn a_damaged_frame_is_an_error_not_other_text() {
    let (conn, path) = db("damaged");
    let codec = Codec::open(&conn, &path).unwrap();
    let mut frame = codec.pack(&conn, REPLAY_RECORD, &record(7));
    let n = frame.len();
    frame[n - 3] ^= 0x40;
    assert!(codec.text(ValueRef::Blob(&frame)).is_err());
    assert!(codec.text(ValueRef::Blob(b"not a frame")).is_err());
    assert_eq!(codec.opt_text(ValueRef::Null).unwrap(), None);
    // A frame naming a dictionary this database never had is an error, after a reload.
    let foreign = sv10_pack::pack_with(record(7).as_bytes(), 9, b"some other dictionary");
    assert!(codec.text(ValueRef::Blob(&foreign)).is_err());
}

#[test]
fn a_dictionary_trained_elsewhere_is_picked_up_by_a_reader() {
    let (conn, path) = db("elsewhere");
    let reader = Codec::open(&conn, &path).unwrap();
    let writer = Codec::open(&Connection::open(&path).unwrap(), &path).unwrap();
    for i in 0..TRAIN_MIN_ROWS as usize {
        conn.execute("INSERT INTO replays (record) VALUES (?1)", [record(i)]).unwrap();
    }
    assert_eq!(writer.train(&conn, REPLAY_RECORD).unwrap(), Some(1));
    let frame = writer.pack(&conn, REPLAY_RECORD, &record(3));
    assert_eq!(sv10_pack::dictionary_id(&frame), Some(1));
    assert_eq!(reader.text(ValueRef::Blob(&frame)).unwrap(), record(3));
}

#[test]
fn a_dictionary_id_one_reload_could_not_find_is_remembered_then_forgiven() {
    // 0253: reading many frames that name an absent dictionary used to open a connection and
    // rescan pack_dicts for every row. The id is remembered instead — and forgotten as soon as
    // a dictionary with that id really exists, so a process that trained one late still reads.
    let (conn, path) = db("missing-id");
    let codec = Codec::open(&conn, &path).unwrap();
    let frame = sv10_pack::pack_with(record(7).as_bytes(), 9, b"some other dictionary");
    for _ in 0..3 {
        assert!(codec.text(ValueRef::Blob(&frame)).is_err());
    }
    assert!(codec.dicts.read().missing.contains(&9), "one reload, then remembered");
    conn.execute("INSERT INTO pack_dicts (id, family, created, bytes) VALUES (9, 'other', 'now', ?1)", [&b"some other dictionary"[..]])
        .unwrap();
    assert!(codec.text(ValueRef::Blob(&frame)).is_err(), "no second reload until one is due");
    codec.reload(&conn).unwrap();
    assert!(!codec.dicts.read().missing.contains(&9), "a reload forgets the verdict");
    assert_eq!(codec.text(ValueRef::Blob(&frame)).unwrap(), record(7));
}

#[test]
fn data_format_marker_only_rises_unless_lowered() {
    let dir = std::env::temp_dir().join(format!("sv10-packed-format-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert_eq!(data_format(&dir), 1);
    mark_data_format(&dir, 2, false).unwrap();
    assert_eq!(data_format(&dir), 2);
    mark_data_format(&dir, 1, false).unwrap();
    assert_eq!(data_format(&dir), 2);
    mark_data_format(&dir, 1, true).unwrap();
    assert_eq!(data_format(&dir), 1);
}

#[test]
fn training_bytes_fill_the_window_with_whole_rows_newest_last() {
    let rows: Vec<String> = (0..600).map(record).collect();
    let bytes = train_bytes(&rows);
    assert!(bytes.len() <= 32 * 1024 && bytes.len() > 30 * 1024, "{}", bytes.len());
    assert!(bytes.ends_with(rows[0].as_bytes()), "the newest sample ends the dictionary");
    assert!(train_bytes(&[]).is_empty());
}

#[test]
fn a_column_another_process_added_first_is_not_an_error() {
    // Hot swaps open the store from the bot, learner and analyst at once: both can see the
    // column missing, and the slower ALTER then fails with "duplicate column name" (learner
    // exit at the 2026-09-26 06:55 swap).
    let (conn, path) = db("race");
    let other = Connection::open(&path).unwrap();
    add_column(&other, "replays", "version", "TEXT").unwrap();
    add_column(&conn, "replays", "version", "TEXT").unwrap();
    ensure_column(&conn, "replays", "version", "TEXT").unwrap();
    assert!(add_column(&conn, "missing_table", "version", "TEXT").is_err(), "other errors still fail");
}

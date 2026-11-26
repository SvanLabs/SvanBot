//! The season download (V11-13, V11-38) against a stand-in venue: the season's export into the
//! store as history, duplicates kept once, and a second run reading the season whole again.
// The stand-in venue is a socket server, and each store is a file in a temporary directory.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use svanbot_runtime::download::{DownloadError, MAX_BODY, PAGE, TIMEOUT, download};
use svanbot_runtime::history::observed_for_fit;
use svanbot_runtime::limit::Limiter;
use svanbot_runtime::record::Hand;
use svanbot_runtime::rest::Rest;
use svanbot_runtime::store::{Source, Store};

fn venue(pages: Vec<Value>) -> (String, thread::JoinHandle<Vec<String>>) {
    venue_with_totals(pages.into_iter().map(|page| (page, None)).collect())
}

fn venue_with_totals(
    pages: Vec<(Value, Option<u64>)>,
) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/api", listener.local_addr().unwrap());
    let served = thread::spawn(move || {
        let mut paths = Vec::new();
        for (page, total) in pages {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            let head = String::from_utf8(head).unwrap();
            paths.push(head.split(' ').nth(1).unwrap().to_owned());
            let body = page.to_string();
            let total =
                total.map_or_else(String::new, |total| format!("x-total-hands: {total}\r\n"));
            let reply = format!(
                "HTTP/1.1 200 OK\r\nconnection: close\r\ncontent-type: application/json\r\n{total}content-length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
        }
        paths
    });
    (base, served)
}

#[test]
fn a_short_export_that_declares_more_hands_keeps_progress_and_fails() {
    let (store, _) = store("incomplete");
    let (base, served) = venue_with_totals(vec![
        (json!({"agent_id": "a-1"}), None),
        (json!([{"hand_id": "h-1"}, {"hand_id": "h-1"}]), Some(2)),
    ]);
    assert!(matches!(
        download(&mut rest(&base), &store, || 5),
        Err(DownloadError::Incomplete {
            expected: 2,
            received: 1
        })
    ));
    served.join().unwrap();
    assert_eq!(store.hands().unwrap().len(), 1);
}

fn store(name: &str) -> (Store, PathBuf) {
    let dir = std::env::temp_dir().join(format!("svanbot-download-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("SurSvan.db");
    (Store::open(&path, "SurSvan").unwrap(), path)
}

fn rest(base: &str) -> Rest {
    Rest::new(base, "k", Limiter::new(100, Duration::from_secs(60))).with_sleep(|_| {})
}

fn hands(ids: std::ops::Range<u32>) -> Value {
    Value::Array(
        ids.map(|id| json!({"hand_id": format!("h-{id}"), "table_id": "t-1"}))
            .collect(),
    )
}

#[test]
fn it_reads_the_season_export_into_the_store_and_reads_it_whole_again() {
    let (store, _) = store("export");
    // Live play already kept one of these hands: the download keeps that first copy.
    let live = Hand {
        hand_id: "h-3".into(),
        table_id: "t-1".into(),
        frames: vec![json!({"type": "hand_start"}).to_string()],
        traced: Vec::new(),
    };
    store.append_hand(&live, Source::Live, 1).unwrap();
    let full = PAGE;
    let export = "/api/portfolio/bots/a-1/hand-history/export?format=json";
    // The bot's id first, then the export: a list a page, newest first, this season only.
    let (base, served) = venue(vec![
        json!({ "agent_id": "a-1", "name": "SurSvan" }),
        hands(0..full),
        json!([{"id": 9999}, {"no_id": true}]),
    ]);
    let summary = download(&mut rest(&base), &store, || 5).unwrap();
    let paths = served.join().unwrap();
    assert_eq!(paths[0], "/api/me");
    assert_eq!(paths[1], format!("{export}&limit={full}&offset=0"));
    assert_eq!(paths[2], format!("{export}&limit={full}&offset={full}"));
    assert_eq!(summary.pages, 2);
    assert_eq!(
        (summary.added, summary.known, summary.skipped),
        (full, 1, 1)
    );
    assert_eq!(summary.offset, u64::from(full) + 2);
    assert_eq!(store.download_offset().unwrap(), summary.offset);
    let kept = store.hands().unwrap();
    assert_eq!(kept.len() as u32, full + 1);
    assert!(kept.iter().any(|hand| hand.hand_id == "9999"));
    // A second run reads the season from its newest hand again, since every new hand moves the
    // offsets, and keeps nothing twice.
    let (base, served) = venue(vec![json!({ "agent_id": "a-1" }), hands(0..2)]);
    let again = download(&mut rest(&base), &store, || 6).unwrap();
    let paths = served.join().unwrap();
    assert_eq!(paths[1], format!("{export}&limit={full}&offset=0"));
    assert_eq!((again.pages, again.added, again.known), (1, 0, 2));
}

#[test]
fn a_season_larger_than_ten_mebibytes_is_read_whole() {
    // A season's export is 30 MB; the client's usual limit is 10 MiB.
    let pad = "x".repeat(1024);
    let page = Value::Array(
        (0..11_000)
            .map(|id| json!({"hand_id": format!("h-{id}"), "pad": pad}))
            .collect(),
    );
    let (store, _) = store("large");
    let (base, served) = venue(vec![json!({ "agent_id": "a-1" }), page.clone()]);
    assert!(download(&mut rest(&base), &store, || 5).is_err());
    served.join().unwrap();
    let (base, served) = venue(vec![json!({ "agent_id": "a-1" }), page]);
    let mut large = rest(&base).with_large_replies(TIMEOUT, MAX_BODY);
    let summary = download(&mut large, &store, || 6).unwrap();
    served.join().unwrap();
    assert_eq!(summary.added, 11_000);
}

#[test]
fn a_profile_without_an_id_stops_the_download() {
    let (store, _) = store("no-id");
    let (base, served) = venue(vec![json!({ "name": "SurSvan" })]);
    assert!(download(&mut rest(&base), &store, || 5).is_err());
    served.join().unwrap();
    assert!(store.hands().unwrap().is_empty());
}

#[test]
fn a_newest_first_export_is_read_chronologically_for_the_fit() {
    let (store, _) = store("training");
    let live = Hand {
        hand_id: "live".into(),
        table_id: "t-1".into(),
        frames: vec![],
        traced: vec![],
    };
    store.append_hand(&live, Source::Live, 2_000).unwrap();
    let (base, served) = venue(vec![
        json!({"agent_id": "a-1"}),
        json!([
            {"hand_id": "new", "ended_at": "1970-01-01T00:00:03+00:00"},
            {"hand_id": "old", "ended_at": "1970-01-01T00:00:01+00:00"}
        ]),
    ]);
    download(&mut rest(&base), &store, || 10_000).unwrap();
    served.join().unwrap();
    let order: Vec<_> = store
        .hands_for_fit()
        .unwrap()
        .into_iter()
        .map(|hand| hand.kept.hand_id)
        .collect();
    assert_eq!(order, ["old", "live", "new"]);
}

#[test]
fn an_export_fills_a_partial_live_hand_for_the_fit_without_replacing_it() {
    let (store, _) = store("partial-live");
    let live = Hand {
        hand_id: "known".into(),
        table_id: "t-1".into(),
        frames: vec![json!({"type": "hand_start"}).to_string()],
        traced: vec![],
    };
    store.append_hand(&live, Source::Live, 2_000).unwrap();
    let (base, served) = venue(vec![
        json!({"agent_id": "a-1"}),
        json!([{
            "hand_id": "known", "table_id": "t-1", "seat": 0, "dealer_seat": 0,
            "small_blind": 10, "big_blind": 20, "num_players": 2,
            "hole_cards": ["3c", "3d"], "board": [], "shown_cards": {}, "pot_total": 40,
            "all_actions": [
                {"seat": 0, "sequence_in_hand": 1, "street": "preflop", "action": "call", "amount": 0},
                {"seat": 1, "sequence_in_hand": 2, "street": "preflop", "action": "check", "amount": 0}
            ]
        }]),
    ]);
    let summary = download(&mut rest(&base), &store, || 5).unwrap();
    served.join().unwrap();
    assert_eq!((summary.added, summary.known), (0, 1));
    assert_eq!(store.hands().unwrap()[0].frames, live.frames);
    let hands = store.hands_for_fit().unwrap();
    assert_eq!(hands.len(), 1);
    let (sides, export) = observed_for_fit(&hands[0], 0);
    assert_eq!(sides.len(), 1);
    assert_eq!(export, Some(true));
}

#[test]
fn an_unusable_export_keeps_a_partial_live_hands_usable_actions() {
    let (store, _) = store("export-fallback");
    let live = Hand {
        hand_id: "known".into(),
        table_id: "t-1".into(),
        frames: [
            json!({"type": "hand_start", "hand_id": "known", "seat": 0, "dealer_seat": 0,
                   "blinds": {"small_blind": 10, "big_blind": 20}}),
            json!({"type": "player_action", "seat": 1, "action": "fold", "amount": null, "stack": 1000}),
        ].map(|frame| frame.to_string()).to_vec(),
        traced: vec![],
    };
    store.append_hand(&live, Source::Live, 2_000).unwrap();
    let (base, served) = venue(vec![
        json!({"agent_id": "a-1"}),
        json!([{"hand_id": "known"}]),
    ]);
    download(&mut rest(&base), &store, || 5).unwrap();
    served.join().unwrap();
    let hands = store.hands_for_fit().unwrap();
    let (sides, export) = observed_for_fit(&hands[0], 0);
    assert_eq!(sides.len(), 1);
    assert_eq!(export, Some(false));
}

#[test]
fn all_seasons_verifies_closed_exports_and_skips_them_on_retry() {
    use svanbot_runtime::download::download_all;

    let (store, _) = store("all-seasons");
    let catalog = json!([
        {"season_number": 4, "season_id": "active", "status": "active"},
        {"season_number": 3, "season_id": "ended", "status": "ended",
         "start_date": "2026-09-01T00:00:00Z", "end_date": "2026-09-15T00:00:00Z"}
    ]);
    let stats = json!({"seasons": [{"season_number": 3, "hands_played": 1}]});
    let old = json!([{
        "hand_id": "old", "started_at": "2026-09-02T01:00:00+01:00"
    }]);
    let (base, served) = venue_with_totals(vec![
        (json!({"agent_id": "bot"}), None),
        (stats.clone(), None),
        (catalog.clone(), None),
        (json!([{"hand_id": "active"}]), Some(1)),
        (old, Some(1)),
    ]);
    download_all(&mut rest(&base), &store, || 5, |_, _| {}).unwrap();
    let paths = served.join().unwrap();
    assert!(!paths[3].contains("season_id="));
    assert!(paths[4].contains("season_id=ended"));
    assert_eq!(store.hands().unwrap().len(), 2);

    let (base, served) = venue(vec![
        json!({"agent_id": "bot"}),
        stats,
        catalog,
        json!([{"hand_id": "active"}]),
    ]);
    let mut skipped = false;
    download_all(
        &mut rest(&base),
        &store,
        || 6,
        |season, result| {
            if season == Some(3) {
                skipped = matches!(result, Ok(None));
            }
        },
    )
    .unwrap();
    assert!(skipped);
    assert_eq!(served.join().unwrap().len(), 4);
    assert_eq!(store.hands().unwrap().len(), 2);
}

#[test]
fn a_wrong_season_is_never_imported_or_marked_complete() {
    use svanbot_runtime::download::download_all;

    let (store, _) = store("wrong-season");
    let stats = json!({"seasons": [{"season_number": 3, "hands_played": 1}]});
    let catalog = json!([{
        "season_number": 3, "season_id": "ended", "status": "ended",
        "start_date": "2026-09-01T00:00:00Z", "end_date": "2026-09-15T00:00:00Z"
    }]);
    for _ in 0..2 {
        let (base, served) = venue(vec![
            json!({"agent_id": "bot"}),
            stats.clone(),
            catalog.clone(),
            json!([{"hand_id": "active"}]),
            json!([{"hand_id": "wrong", "started_at": "2026-09-16T00:00:00Z"}]),
        ]);
        let mut rejected = false;
        let result = download_all(
            &mut rest(&base),
            &store,
            || 5,
            |season, result| {
                if season == Some(3) {
                    rejected = result.is_err();
                }
            },
        );
        assert!(result.is_err());
        assert!(rejected);
        assert_eq!(served.join().unwrap().len(), 5);
        assert_eq!(store.hands().unwrap().len(), 1);
    }
}

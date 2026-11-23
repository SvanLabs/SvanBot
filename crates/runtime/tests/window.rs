//! The canary's window, as the ship bar reads it from the stores (V11-22): it opens when the
//! canary took the release, and holds what the canary won since, and what the rest of the fleet
//! won over the same hours by its export.
// A test store is a file in a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

mod support;

use serde_json::json;
use support::scratch;
use svanbot_runtime::ledger::Played;
use svanbot_runtime::record::Hand;
use svanbot_runtime::ship::window::{self, started_ms};
use svanbot_runtime::store::{Source, Store};
use svanbot_strategy::params::Params;

fn played(id: &str, net: i64) -> Played {
    Played {
        hand_id: id.into(),
        table_id: "t-1".into(),
        digest: Params::DEFAULT.digest(),
        net,
        luck: 0.0,
        big_blind: 20,
        orbit: None,
    }
}

/// An export hand as the venue gives it: one item, kept whole.
fn exported(item: serde_json::Value) -> Hand {
    Hand {
        hand_id: item["hand_id"].as_str().unwrap_or("none").into(),
        table_id: "t-9".into(),
        frames: vec![item.to_string()],
        traced: Vec::new(),
    }
}

#[test]
fn a_store_keeps_when_it_first_took_a_release() {
    let dir = scratch("window-release");
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    assert_eq!(store.released("11.0.0").unwrap(), None);
    store.mark_release("11.0.0", 1_000).unwrap();
    // A restart on the same release marks it again: the first mark stands.
    store.mark_release("11.0.0", 9_000).unwrap();
    store.mark_release("11.0.1", 5_000).unwrap();
    assert_eq!(store.released("11.0.0").unwrap(), Some(1_000));
    assert_eq!(store.released("11.0.1").unwrap(), Some(5_000));
}

#[test]
fn the_canarys_side_is_what_it_won_on_each_hand_since_the_release_in_big_blinds() {
    let dir = scratch("window-canary");
    let store = Store::open(&dir.join("SurSvan.db"), "SurSvan").unwrap();
    store.append_played(&played("h-1", 400), 500).unwrap();
    store.append_played(&played("h-2", -10), 1_500).unwrap();
    store.append_played(&played("h-3", 30), 2_500).unwrap();
    assert_eq!(
        store.results_since(1_000).unwrap(),
        [(1_500, -0.5), (2_500, 1.5)]
    );
}

#[test]
fn the_fleets_side_is_its_export_hands_started_in_the_window_in_big_blinds() {
    let dir = scratch("window-fleet");
    let store = Store::open(&dir.join("Svanar.db"), "Svanar").unwrap();
    let items = [
        json!({"hand_id": "x-1", "profit": 40, "big_blind": 20, "started_at": "2026-11-23T10:00:00Z"}),
        // Another offset, and a fraction of a second: the same instant plus 0.25 s.
        json!({"hand_id": "x-2", "profit": -15, "big_blind": 10, "started_at": "2026-11-23T11:00:00.250+01:00"}),
        // Before the window.
        json!({"hand_id": "x-3", "profit": 99, "big_blind": 20, "started_at": "2026-11-23T09:59:59Z"}),
        // No result: not counted, and counted as unread.
        json!({"hand_id": "x-4", "big_blind": 20, "started_at": "2026-11-23T10:30:00Z"}),
    ];
    for item in items {
        store
            .append_hand(&exported(item), Source::History, 0)
            .unwrap();
    }
    // A hand recorded at the table is not the export.
    let live = exported(
        json!({"hand_id": "x-5", "profit": 7, "big_blind": 1, "started_at": "2026-11-23T10:00:00Z"}),
    );
    store.append_hand(&live, Source::Live, 0).unwrap();

    let from = started_ms("2026-11-23T10:00:00Z").unwrap();
    let until = from + 3_600_000;
    let fleet = window::fleet(&store.export_lines().unwrap(), from, until);
    assert_eq!(fleet.results, [2.0, -1.5]);
    assert_eq!(fleet.unread, 1);
}

#[test]
fn a_start_is_read_as_rfc_3339() {
    assert_eq!(started_ms("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(started_ms("2026-11-23T10:00:00Z"), Some(1_795_428_000_000));
    assert_eq!(
        started_ms("2026-11-23T10:00:00+00:00"),
        Some(1_795_428_000_000)
    );
    assert_eq!(
        started_ms("2026-11-23T05:30:00-04:30"),
        Some(1_795_428_000_000)
    );
    assert_eq!(
        started_ms("2026-11-23 10:00:00.5z"),
        Some(1_795_428_000_500)
    );
    // The venue's naive stamps are its UTC.
    assert_eq!(started_ms("2024-02-29T00:00:00"), Some(1_709_164_800_000));
    for refused in [
        "",
        "2026-11-23",
        "2026-13-01T00:00:00Z",
        "2026-11-23T10:00Z",
        "yesterday",
    ] {
        assert_eq!(started_ms(refused), None, "{refused}");
    }
}

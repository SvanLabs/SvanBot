//! `/api/reads/<bot>`: one bot's own opponent stats, rebuilt from its store — counts beside every
//! rate, a display interval, and the evidence floor said out loud.
// A test server is a socket and files in a fresh temporary directory.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

mod support;

use std::path::PathBuf;
use std::thread;

use serde_json::{Value, json};
use support::{get, scratch};
use svanbot::dashboard::{Config, Season, Server};
use svanbot_runtime::fleet;
use svanbot_runtime::record::Hand;
use svanbot_runtime::store::{Source, Store};

/// A store for `Alpha` holding `count` hands in which Villain (seat 0) calls preflop and Hero
/// (seat 2, the bot) folds to it.
fn seeded(name: &str, count: usize) -> (PathBuf, std::net::SocketAddr) {
    let dir = scratch(name);
    let data = dir.join("data");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    let store = Store::open(&data.join("Alpha.db"), "Alpha").unwrap();
    for number in 0..count {
        let id = format!("h-{number}");
        let hand = Hand {
            hand_id: id.clone(),
            table_id: "t-1".into(),
            frames: vec![
                json!({"type": "hand_start", "hand_id": id, "seat": 2, "dealer_seat": 0,
                       "blinds": {"small_blind": 10, "big_blind": 20}})
                .to_string(),
                json!({"type": "your_turn", "hand_id": id, "turn_token": "tok", "pot": 30,
                       "community_cards": [],
                       "players": [{"seat": 0, "name": "Villain", "stack": 1980},
                                   {"seat": 2, "name": "Hero", "stack": 1990}],
                       "valid_actions": [{"action": "fold"}]})
                .to_string(),
                json!({"type": "player_action", "hand_id": id, "seat": 0, "name": "Villain",
                       "action": "call", "amount": 20, "stack": 1980})
                .to_string(),
                json!({"type": "player_action", "hand_id": id, "seat": 2, "name": "Hero",
                       "action": "fold", "amount": null, "stack": 1990})
                .to_string(),
            ],
            traced: vec![],
        };
        store
            .append_hand(&hand, Source::Live, number as i64)
            .unwrap();
    }
    let fleet_dir = fleet::beside_store(data.join("Alpha.db"));
    let server = Server::bind(Config {
        addr: "127.0.0.1:0".into(),
        data_dir: data,
        fleet_dir,
        lib_dir: lib,
        season: Season::default(),
    })
    .unwrap();
    let addr = server.addr();
    thread::spawn(move || server.run());
    (dir, addr)
}

#[test]
fn a_rates_counts_and_its_floor_are_all_said() {
    let (_dir, addr) = seeded("reads-floor", 30);
    let (status, body) = get(addr, "/api/reads/Alpha");
    assert_eq!(status, 200);
    let reads: Value = serde_json::from_str(&body).unwrap();
    let villains = reads["opponents"].as_array().unwrap();
    assert_eq!(villains.len(), 1);
    let villain = &villains[0];
    assert_eq!(villain["name"], "Villain");
    assert_eq!(villain["hands"], 30);
    // 30 calls from 30 hands: past the floor, so a rate and an interval are shown.
    let vpip = &villain["stats"]["vpip"];
    assert_eq!(vpip["hits"], 30);
    assert_eq!(vpip["opportunities"], 30);
    assert_eq!(vpip["rate"], 1.0);
    assert_eq!(vpip["floor"], false);
    let interval = vpip["interval"].as_array().unwrap();
    assert_eq!(interval.len(), 2);
    let (lo, hi) = (interval[0].as_f64().unwrap(), interval[1].as_f64().unwrap());
    assert!(0.0 <= lo && lo <= hi && hi <= 1.0);
    // Never faced a bet: the counts are said, and the floor message instead of a rate.
    let fold = &villain["stats"]["fold_to_bet"];
    assert_eq!(fold["opportunities"], 0);
    assert_eq!(fold["rate"], Value::Null);
    assert_eq!(fold["floor"], true);
    assert_eq!(fold["interval"], Value::Null);
    // The situations carry their own counts, and a thin one is flagged as thin.
    let situations = villain["situations"].as_array().unwrap();
    assert_eq!(situations.len(), 8, "every street, facing a bet or not");
    assert!(situations.iter().all(|cell| cell["sparse"]
        == json!(cell["n"].as_u64().unwrap() > 0 && cell["n"].as_u64().unwrap() < 30)));
    // The hands behind the read, so the stat can be traced back to what produced it.
    let hands = villain["hand_ids"].as_array().unwrap();
    assert_eq!(hands.len(), 30);
    assert_eq!(hands[0], "h-0");
}

#[test]
fn under_the_floor_the_counts_shown_but_no_rate() {
    let (_dir, addr) = seeded("reads-thin", 1);
    let (_, body) = get(addr, "/api/reads/Alpha");
    let reads: Value = serde_json::from_str(&body).unwrap();
    let vpip = &reads["opponents"][0]["stats"]["vpip"];
    assert_eq!(vpip["hits"], 1);
    assert_eq!(vpip["opportunities"], 1);
    assert_eq!(vpip["rate"], Value::Null);
    assert_eq!(vpip["floor"], true);
    assert_eq!(vpip["interval"], Value::Null);
}

#[test]
fn a_bot_that_has_played_nothing_reads_nobody() {
    let dir = scratch("reads-empty");
    let data = dir.join("data");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    let server = Server::bind(Config {
        addr: "127.0.0.1:0".into(),
        data_dir: data.clone(),
        fleet_dir: fleet::beside_store(data.join("Alpha.db")),
        lib_dir: lib,
        season: Season::default(),
    })
    .unwrap();
    let addr = server.addr();
    thread::spawn(move || server.run());
    let (status, body) = get(addr, "/api/reads/Alpha");
    assert_eq!(status, 200);
    let reads: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(reads["opponents"], json!([]));
    // A name no bot can have is not an empty list.
    let (status, _) = get(addr, "/api/reads/bad%20name");
    assert_eq!(status, 404);
}

#[test]
fn the_pages_stats_script_is_served() {
    let (_dir, addr) = seeded("reads-script", 1);
    let (status, body) = get(addr, "/reads.js");
    assert_eq!(status, 200);
    assert!(
        body.contains("readsPanel"),
        "{}",
        &body[..body.len().min(80)]
    );
}

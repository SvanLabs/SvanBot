//! The dashboard hands list carries each hand's provenance beside it (V11-122), so a stat can
//! be traced to hands and a hand to how it was recorded.
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

mod support;

use std::thread;

use serde_json::Value;
use support::{get, scratch};
use svanbot::dashboard::{Config, Season, Server};
use svanbot_runtime::fleet;

#[test]
fn the_hands_list_carries_each_hands_provenance() {
    let dir = scratch("provenance");
    let data = dir.join("data");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::create_dir_all(&lib).unwrap();
    support::traced_store(&data, "Alpha");
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
    let (status, body) = get(addr, "/api/hands/Alpha");
    assert_eq!(status, 200);
    let hands: Value = serde_json::from_str(&body).unwrap();
    let first = hands["hands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|hand| hand["hand_id"] == "h-1")
        .unwrap();
    let quality = &first["quality"];
    assert_eq!(quality["parser"], "2026-09-02");
    assert_eq!(quality["complete"], true);
    assert_eq!(quality["unparsed"], 0);
}

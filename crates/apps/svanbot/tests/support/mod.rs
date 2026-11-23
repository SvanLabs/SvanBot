//! Fixtures shared by the dashboard's tests: a scratch directory, a report from a bot, a server
//! on an ephemeral port, and a raw request against it.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use svanbot::dashboard::{Config, Server};
use svanbot_runtime::fleet::{self, ArenaRow, ArenaRun, Fleet, League};
use svanbot_runtime::status::{Play, Status, status_path};

pub fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("svanbot-dashboard-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn status(bot: &str, updated_ms: i64) -> Status {
    Status {
        bot: bot.into(),
        pid: 1,
        version: "11.0.0-dev".into(),
        started_ms: 1_000,
        updated_ms,
        connected: true,
        table: None,
        hands: 12,
        incidents: 0,
        run_incidents: 0,
        last_hand_ms: Some(900),
        last_error: None,
        play: Play::default(),
    }
}

/// A server on an ephemeral port, with `Alpha` already reporting, two builds installed, and one
/// recorded Arena run in the fleet beside the stores.
pub fn serve(name: &str) -> (SocketAddr, PathBuf) {
    let dir = scratch(name);
    let data = dir.join("data");
    let lib = dir.join("lib");
    std::fs::create_dir_all(&data).unwrap();
    for version in ["1.9.0", "1.10.0", "staged"] {
        std::fs::create_dir_all(lib.join(version)).unwrap();
    }
    std::os::unix::fs::symlink(lib.join("1.10.0"), lib.join("current")).unwrap();
    status("Alpha", 2_000)
        .write(&status_path(&data.join("Alpha.db")))
        .unwrap();
    let fleet_dir = fleet::beside_store(data.join("Alpha.db"));
    let arena = Fleet::in_dir(&fleet_dir);
    // The run is of the champion in play: the default until a promotion. A scoreboard of any
    // other champion is a problem the page raises, which most tests here are not about.
    let champion = arena.champion().unwrap().digest().to_string();
    arena
        .record_arena(&ArenaRun {
            commit: "abc1234".into(),
            champion,
            seed: 7,
            league: League::HeadsUp,
            iterations: None,
            rows: vec![ArenaRow {
                opponent: "nit".into(),
                deals: 400,
                mbb_per_100: 12_345.6,
                ci95: 2_000.4,
                bouts: Vec::new(),
            }],
        })
        .unwrap();
    let server = Server::bind(Config {
        addr: "127.0.0.1:0".into(),
        data_dir: data,
        fleet_dir,
        lib_dir: lib,
    })
    .unwrap();
    let addr = server.addr();
    thread::spawn(move || server.run());
    (addr, dir)
}

/// Sends `request` as written and returns the status line and a reader at the body.
pub fn send(addr: SocketAddr, request: &str) -> (String, BufReader<TcpStream>) {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut reader = BufReader::new(stream);
    let mut status = String::new();
    reader.read_line(&mut status).unwrap();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" || line.is_empty() {
            break;
        }
    }
    (status, reader)
}

/// A whole answer to `GET path`: the status code and the body.
pub fn get(addr: SocketAddr, path: &str) -> (u16, String) {
    let (status, mut reader) = send(addr, &format!("GET {path} HTTP/1.1\r\nHost: x\r\n\r\n"));
    let mut body = String::new();
    reader.read_to_string(&mut body).unwrap();
    (status.split(' ').nth(1).unwrap().parse().unwrap(), body)
}

/// A store whose one traced hand has a decision the codec cannot read: valid JSON and a street and
/// branch the schema allows, but no `hole`, as the row shape before `hole` and `board` were kept
/// wrote it. The hand is one `/api/hands` counts a decision in, so `/api/trace` cannot call it
/// untraced.
pub fn undecodable_trace(data: &std::path::Path, bot: &str) {
    use svanbot_runtime::record::Hand;
    use svanbot_runtime::store::{Source, Store};

    let store = Store::open(&data.join(format!("{bot}.db")), bot).unwrap();
    store
        .append_hand(
            &Hand {
                hand_id: "h-3".into(),
                table_id: "t-1".into(),
                frames: vec![r#"{"type":"hand_start","hand_id":"h-3"}"#.into()],
                traced: Vec::new(),
            },
            Source::Live,
            3_000,
        )
        .unwrap();
    drop(store);
    // A decision written straight, since the writer refuses one the reader could not read: the
    // point is a store this code reads that an older writer left. Append-only allows the insert,
    // and the `json_valid` CHECK takes this JSON.
    let raw = rusqlite::Connection::open(data.join(format!("{bot}.db"))).unwrap();
    raw.execute(
        "INSERT INTO decisions (hand_id, seq, street, branch, trace) VALUES (?1, 0, ?2, ?3, ?4)",
        rusqlite::params![
            "h-3",
            "preflop",
            "priced",
            r#"{"branch":"priced","equity":0.5,"price":null,"ranges":[]}"#
        ],
    )
    .unwrap();
}

/// A status with the bot's store beside it, holding one traced hand and one untraced one.
pub fn traced_store(data: &std::path::Path, bot: &str) {
    use svanbot_core::rules::Street;
    use svanbot_runtime::record::Hand;
    use svanbot_runtime::store::{Source, Store};
    use svanbot_strategy::policy::{Branch, PricedTrace, Ranged, Trace};

    let path = data.join(format!("{bot}.db"));
    let store = Store::open(&path, bot).unwrap();
    for (id, ms, traced) in [("h-1", 1_000, true), ("h-2", 2_000, false)] {
        store
            .append_hand(
                &Hand {
                    hand_id: id.into(),
                    table_id: "t-1".into(),
                    frames: vec![
                        format!(r#"{{"type":"hand_start","hand_id":"{id}"}}"#),
                        format!(r#"{{"type":"hand_result","hand_id":"{id}"}}"#),
                    ],
                    traced: if traced {
                        vec![Trace {
                            branch: Branch::Priced,
                            street: Street::Preflop,
                            hole: "AhAs".parse().expect("a hole"),
                            board: svanbot_core::card::CardSet::EMPTY,
                            resolved: None,
                            priced: Some(PricedTrace {
                                equity: 0.61,
                                price: Some(0.25),
                                exploit: Some(0.4),
                                ranges: vec![Ranged {
                                    seat: Some(3),
                                    masses: vec![0.0, 0.1, 0.2, 0.3, 0.2, 0.1, 0.1, 0.0],
                                }],
                            }),
                        }]
                    } else {
                        Vec::new()
                    },
                },
                Source::Live,
                ms,
            )
            .unwrap();
    }
}

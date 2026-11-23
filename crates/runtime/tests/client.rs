//! The client against a stand-in venue on localhost: it authenticates, joins, and answers a turn.
// The stand-in venue is a socket server, and tungstenite's handshake callback returns its own
// large error type.
#![allow(clippy::disallowed_types, clippy::result_large_err)]

use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use serde_json::{Value, json};
use svanbot_core::rules::Action;
use svanbot_runtime::client::{Bot, Config, Event, PlayOutcome, play, play_with_stop};
use svanbot_runtime::status::Table;
use tungstenite::Message;
use tungstenite::handshake::server::{Request, Response};

fn send(socket: &mut tungstenite::WebSocket<std::net::TcpStream>, frame: Value) {
    socket.send(Message::text(frame.to_string())).unwrap();
}

fn read(socket: &mut tungstenite::WebSocket<std::net::TcpStream>) -> Value {
    loop {
        if let Message::Text(text) = socket.read().unwrap() {
            return serde_json::from_str(text.as_str()).unwrap();
        }
    }
}

#[test]
fn it_authenticates_joins_and_answers_its_turn() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/ws", listener.local_addr().unwrap());
    let venue = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut bearer = String::new();
        let mut socket =
            tungstenite::accept_hdr(stream, |request: &Request, response: Response| {
                bearer = request.headers()["authorization"]
                    .to_str()
                    .unwrap()
                    .to_owned();
                Ok(response)
            })
            .unwrap();
        send(
            &mut socket,
            json!({"type": "connected", "agent_id": "a-1", "name": "SurSvan"}),
        );
        let joined = [read(&mut socket), read(&mut socket)];
        send(
            &mut socket,
            json!({"type": "table_joined", "table_id": "t-1", "seat": 2, "players": [
            {"seat": 0, "name": "alpha_bot", "stack": 2000}, {"seat": 2, "name": "SurSvan", "stack": 2000}]}),
        );
        send(
            &mut socket,
            json!({"type": "hand_start", "hand_id": "h-1", "seat": 2, "dealer_seat": 0,
            "blinds": {"small_blind": 10, "big_blind": 20}}),
        );
        send(
            &mut socket,
            json!({"type": "hole_cards", "hand_id": "h-1", "cards": ["Ah", "As"]}),
        );
        send(
            &mut socket,
            json!({"type": "community_cards", "hand_id": "h-1", "cards": ["Ah", "Kd", "7c"]}),
        );
        send(
            &mut socket,
            json!({"type": "your_turn", "hand_id": "h-1", "turn_token": "tok-1", "pot": 30,
            "community_cards": ["Ah", "Kd", "7c"], "valid_actions": [{"action": "fold"}, {"action": "call", "amount": 20},
            {"action": "raise", "min": 40, "max": 2000}],
            "players": [{"seat": 0, "name": "alpha_bot", "stack": 1980}, {"seat": 2, "name": "SurSvan", "stack": 1990}]}),
        );
        let action = read(&mut socket);
        socket.close(None).unwrap();
        let _ = socket.read();
        (bearer, joined, action)
    });
    let config = Config {
        url,
        key: "test-key".into(),
        me: "SurSvan".into(),
        roster: vec!["Svanar".into(), "SurSvan".into()],
    };
    let mut bot = Bot::new(&config);
    let mut connected = 0;
    let mut tables: Vec<Option<Table>> = Vec::new();
    let ended = play(
        &config,
        &mut bot,
        |_| Action::RaiseTo(60),
        |event| match event {
            Event::Connected => connected += 1,
            Event::Table(table) => tables.push(table),
            Event::Incident(incident) => panic!("{incident:?}"),
            Event::Hand(hand) => panic!("the hand never ended: {hand:?}"),
        },
    );
    let (bearer, joined, action) = venue.join().unwrap();
    assert!(ended.is_ok(), "{ended:?}");
    assert_eq!(connected, 1);
    let table = tables
        .last()
        .and_then(Option::as_ref)
        .expect("the client reports the table it sits at");
    assert_eq!(
        (
            table.id.as_str(),
            table.seat,
            table.hand_id.as_deref(),
            table.street.as_deref(),
            table.pot,
            table.big_blind,
        ),
        ("t-1", Some(2), Some("h-1"), Some("flop"), 30, 20)
    );
    assert_eq!(table.board, ["Ah", "Kd", "7c"]);
    let seats: Vec<(&str, u64)> = table
        .players
        .iter()
        .map(|seat| (seat.name.as_str(), seat.stack))
        .collect();
    assert_eq!(seats, [("alpha_bot", 1_980), ("SurSvan", 1_990)]);
    assert_eq!(bearer, "Bearer test-key");
    assert_eq!(joined[0], json!({"type": "join_lobby", "buy_in": 2000}));
    assert_eq!(
        (
            &action["hand_id"],
            &action["turn_token"],
            &action["action"],
            &action["amount"]
        ),
        (&json!("h-1"), &json!("tok-1"), &json!("raise"), &json!(60))
    );
}

#[test]
fn a_stop_between_hands_keeps_the_finished_hand_and_ends_the_connection() {
    // The venue deals one hand and then goes quiet: after its result, nothing arrives. The updater
    // sets the stop once that hand is kept, and `play` must leave the read it is blocked in, say
    // why, and have kept the hand.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("ws://{}/ws", listener.local_addr().unwrap());
    let venue = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut socket = tungstenite::accept(stream).unwrap();
        for frame in [
            json!({"type": "connected", "agent_id": "a-1", "name": "SurSvan"}),
            json!({"type": "table_joined", "table_id": "t-1", "seat": 2, "players": [
                {"seat": 0, "name": "alpha_bot", "stack": 2000},
                {"seat": 2, "name": "SurSvan", "stack": 2000}]}),
            json!({"type": "hand_start", "hand_id": "h-1", "seat": 2, "dealer_seat": 0,
                "blinds": {"small_blind": 10, "big_blind": 20}}),
            json!({"type": "hand_result", "hand_id": "h-1", "final_stacks": {"0": 1970, "2": 2030}}),
        ] {
            send(&mut socket, frame);
        }
        // Never close: the client is left blocked in a read, as it is between hands.
        let _ = socket.read();
    });
    let config = Config {
        url,
        key: "test-key".into(),
        me: "SurSvan".into(),
        roster: vec!["SurSvan".into()],
    };
    let mut bot = Bot::new(&config);
    let stop = Arc::new(AtomicBool::new(false));
    let setter = Arc::clone(&stop);
    let handed = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&handed);
    let ended = play_with_stop(
        &config,
        &mut bot,
        |_| Action::Fold,
        |event| {
            if let Event::Hand(hand) = event {
                assert_eq!(hand.hand_id, "h-1");
                seen.store(true, Ordering::SeqCst);
                setter.store(true, Ordering::SeqCst);
            }
        },
        &move || stop.load(Ordering::SeqCst),
    );
    venue.join().unwrap();
    assert!(
        handed.load(Ordering::SeqCst),
        "the finished hand was not kept"
    );
    assert!(
        matches!(ended, Ok(PlayOutcome::Stopped)),
        "a stop between hands must end the connection: {ended:?}"
    );
}

#[test]
fn play_without_a_stop_never_stops_itself() {
    let ended = play_with_stop(
        &Config {
            url: "ws://127.0.0.1:1/ws".into(),
            key: "k".into(),
            me: "SurSvan".into(),
            roster: vec!["SurSvan".into()],
        },
        &mut Bot::new(&Config {
            url: "ws://127.0.0.1:1/ws".into(),
            key: "k".into(),
            me: "SurSvan".into(),
            roster: vec!["SurSvan".into()],
        }),
        |_| Action::Fold,
        |_| {},
        &|| false,
    );
    // The stub venue is not there: the connection fails, and the failure is not a stop.
    assert!(
        !matches!(ended, Ok(PlayOutcome::Stopped)),
        "no stop was asked for, yet play stopped: {ended:?}"
    );
}

#[test]
fn a_config_never_shows_its_key() {
    let config = Config {
        url: "wss://x".into(),
        key: "secret-key".into(),
        me: "a".into(),
        roster: vec![],
    };
    assert!(!format!("{config:?}").contains("secret-key"));
}

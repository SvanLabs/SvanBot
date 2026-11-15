//! The client against a stand-in venue on localhost: it authenticates, joins, and answers a turn.
// The stand-in venue is a socket server, and tungstenite's handshake callback returns its own
// large error type.
#![allow(clippy::disallowed_types, clippy::result_large_err)]

use std::net::TcpListener;
use std::thread;

use serde_json::{Value, json};
use svanbot_core::rules::Action;
use svanbot_runtime::client::{Config, play};
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
            json!({"type": "your_turn", "hand_id": "h-1", "turn_token": "tok-1", "pot": 30,
            "community_cards": [], "valid_actions": [{"action": "fold"}, {"action": "call", "amount": 20},
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
    let ended = play(
        &config,
        |_| Action::RaiseTo(60),
        |incident| panic!("{incident:?}"),
    );
    let (bearer, joined, action) = venue.join().unwrap();
    assert!(ended.is_ok(), "{ended:?}");
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
fn a_config_never_shows_its_key() {
    let config = Config {
        url: "wss://x".into(),
        key: "secret-key".into(),
        me: "a".into(),
        roster: vec![],
    };
    assert!(!format!("{config:?}").contains("secret-key"));
}

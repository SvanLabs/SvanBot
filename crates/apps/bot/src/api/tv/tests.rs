//! Tests for the public TV: what the projection keeps, and what the listener answers.
//!
//! Two things can make this listener wrong, and they are tested separately. The projection can leak a
//! field the dashboard only ever sends to an operator, and the router can grow a route nobody meant to
//! publish — a route mounted here is served with no token at all, so [`never_public`] and the socket
//! test below are the whole security story. Both fail loudly when the dashboard's payload or its route
//! list moves, rather than quietly ceasing to prove anything.

use super::*;
use crate::live::Shared;

/// Everything the public payload must not carry, wherever it appears: a card of a live hand, the
/// policy's working, the think clock, and the operator's own figures.
const NEVER_PUBLIC: [&str; 9] = ["hole", "decision", "turn", "turn_started", "last_error", "version", "season", "read", "street"];

/// Every key at every depth, so a forbidden one is caught inside `seats` and `decision` alike.
fn key_paths(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (key, child) in map {
                out.push(key.clone());
                key_paths(child, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| key_paths(item, out)),
        _ => {}
    }
}

/// A table payload in the shape [`state::table_json`] builds it, carrying a field for every name in
/// [`PUBLIC_BOT_KEYS`] and every name in [`NEVER_PUBLIC`].
fn dashboard_table() -> Value {
    json!({
        "slot": 0, "name": "A", "mode": "playing", "status": "playing at table deadbeef",
        "connected": true, "last_error": null, "table_id": "deadbeef-0000", "hand_id": "h1",
        "board": ["As", "Kd", "7c"], "hole": ["2h", "3h"],
        "seats": [
            {"seat": 0, "name": "A", "stack": 5000, "bet": 20, "folded": false, "status": "in hand",
             "last_action": "raise", "avatar_url": "https://example.test/a.png",
             "read": {"archetype": "tag", "vpip": 0.31, "hands": 900}},
            {"seat": 1, "name": "Villain", "stack": 4800, "bet": 40, "folded": false, "status": "in hand",
             "last_action": "call", "avatar_url": null, "read": {"archetype": "lag", "vpip": 0.44}},
        ],
        "hero_seat": 0, "dealer_seat": 5, "actor_seat": 1, "pot": 60, "big_blind": 20,
        "street": "flop",
        "decision": {"action": "call", "amount": 40, "equity": {"value": 0.41}, "version": "sv10-ev-1", "reason": "…"},
        "version": "sv10-ev-7", "turn_started": 1234.5, "turn": true,
        "season": {"number": 4, "hands": 12},
    })
}

#[test]
fn the_public_projection_keeps_the_table_and_drops_everything_else() {
    let built = dashboard_table();
    let spot = Shared::for_test("tv-shape", &["A"]);
    let seated = |seat: usize, name: &str| sv10_venue::tracker::SeatView {
        seat,
        name: name.into(),
        stack: 5_000,
        bet: 20,
        in_hand: true,
        status: "in hand".into(),
        ..Default::default()
    };
    spot.update(0, |b| b.seats.extend([seated(0, "A"), seated(1, "Villain")]));
    let real = state::fleet_table_json(&spot, &spot.bots[0].read());
    // The guard is only worth anything while the payload still has these: a rename or a restructure
    // in `table_json` fails here instead of leaving the blocklist silently out of date.
    for key in NEVER_PUBLIC {
        if key == "read" {
            assert!(built["seats"][0].get(key).is_some(), "the sample seat lost its read");
            assert!(real["seats"][0].get(key).is_some(), "the dashboard's own seats no longer carry a read");
            continue;
        }
        assert!(built.get(key).is_some(), "the sample payload has no `{key}`: the check below no longer proves anything");
        assert!(real.get(key).is_some(), "the dashboard's own payload no longer carries `{key}`");
    }
    let public = public_table(&built);
    // Nothing the dashboard sends for an operator survives, at any depth.
    let mut seen = Vec::new();
    key_paths(&public, &mut seen);
    for key in NEVER_PUBLIC {
        assert!(!seen.iter().any(|k| k == key), "`{key}` reached the public payload: {}", public);
    }
    // And the table a spectator came for is intact.
    for key in PUBLIC_BOT_KEYS {
        assert!(public.get(key).is_some(), "the projection dropped `{key}`: {}", public);
    }
    assert_eq!(public["name"], "A");
    assert_eq!(public["board"], json!(["As", "Kd", "7c"]));
    assert_eq!(public["pot"], 60);
    assert_eq!(public["seats"][1]["name"], "Villain");
    assert_eq!(public["seats"][1]["last_action"], "call");
}

/// One HTTP/1.1 request over a real socket: the status, the header block lowercased, and the body.
async fn ask(addr: std::net::SocketAddr, method: &str, path: &str) -> (u16, String, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(addr).await.expect("connect to the TV listener");
    let request = format!("{method} {path} HTTP/1.1\r\nHost: tv.test\r\nConnection: close\r\nContent-Length: 0\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("send the request");
    let mut raw = Vec::new();
    tokio::time::timeout(Duration::from_secs(20), stream.read_to_end(&mut raw)).await.expect("the TV answered").expect("read the answer");
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
    (head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0), head.to_lowercase(), body.to_string())
}

#[tokio::test]
async fn the_tv_listener_serves_the_table_view_and_404s_every_other_api_route() {
    let s = Shared::for_test("tv-surface", &["A"]);
    std::fs::write(s.config.web_dist.join("index.html"), "<!doctype html><title>tv</title>").unwrap();
    let app = router(s.clone(), &s.config.web_dist);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    // The positive control: an extra listener whose every route 404s would pass the rest of this.
    let (status, _, body) = ask(addr, "GET", "/api/tv").await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"public\":true"), "the TV did not name itself public: {body}");
    // The route applies the projection, not merely the function beside it: `table_json` carries
    // `version`, `street`, `hole`, `season` and `last_error` unconditionally, so a `/api/tv` wired
    // to the dashboard's own payload would be caught here and not only in the shape test.
    let served: Value = serde_json::from_str(&body).expect("the TV serves JSON");
    assert!(served["bots"].as_array().is_some_and(|b| !b.is_empty()), "the TV served no bots: {body}");
    let mut offered = Vec::new();
    key_paths(&served, &mut offered);
    for key in NEVER_PUBLIC {
        assert!(!offered.iter().any(|k| k == key), "`{key}` was served on the public TV: {body}");
    }
    // The discriminator the dashboard branches on before it asks for a session: the TV must keep
    // answering its health this way, or a spectator gets a login form that cannot be satisfied.
    let (status, _, body) = ask(addr, "GET", "/api/health").await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"public\":true"), "health no longer names the TV public: {body}");
    // The page itself: the URL a spectator is handed boots the table view.
    assert_eq!(ask(addr, "GET", "/").await.0, 200, "the SPA is not served on the TV port");
    // Every dashboard route — reads and writes alike — is absent, not merely unauthorized. The
    // detail line is how a route that leaked in behind the auth layer would announce itself.
    for (method, path) in [
        ("GET", "/api/state"),
        ("GET", "/api/events"),
        ("GET", "/api/raw"),
        ("GET", "/api/releases"),
        ("GET", "/api/host"),
        ("POST", "/api/session"),
        ("POST", "/api/releases/update"),
        ("POST", "/api/training/command"),
        ("POST", "/api/bots/0/command"),
    ] {
        let (status, head, body) = ask(addr, method, path).await;
        assert_eq!(status, 404, "{method} {path} answered {status}: {body}");
        assert!(!head.contains("operator token required"), "{method} {path} reached the dashboard's auth layer");
    }
    // Framing: the dashboard refuses it; a public table view is meant to be embedded.
    let (_, head, _) = ask(addr, "GET", "/api/tv").await;
    assert!(head.contains("frame-ancestors *"), "the TV does not allow framing: {head}");
    assert!(!head.contains("x-frame-options: deny"), "the TV kept the dashboard's framing refusal: {head}");
}

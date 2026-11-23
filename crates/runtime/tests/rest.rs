//! The REST client against a stand-in venue on localhost: one limiter per key, `Retry-After`
//! honoured, and a backoff on empty pages (V11-01; <https://docs.openpoker.ai/llms-full.txt>,
//! rev 2026-09-02, "Authentication").
// The stand-in venue is a socket server.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]

use std::cell::RefCell;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::rc::Rc;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use svanbot_runtime::limit::Limiter;
use svanbot_runtime::rest::{EMPTY_PAGE_TRIES, MAX_ATTEMPTS, Rest, RestError, retry_wait};

/// Serves each scripted reply to one request, then stops. Returns each request's head.
fn venue(replies: Vec<String>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/api", listener.local_addr().unwrap());
    let served = thread::spawn(move || {
        let mut heads = Vec::new();
        for reply in replies {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            heads.push(String::from_utf8(head).unwrap().to_lowercase());
            stream.write_all(reply.as_bytes()).unwrap();
        }
        heads
    });
    (base, served)
}

fn reply(status: &str, headers: &[&str], body: &Value) -> String {
    let body = body.to_string();
    let mut text = format!(
        "HTTP/1.1 {status}\r\nconnection: close\r\ncontent-type: application/json\r\ncontent-length: {}\r\n",
        body.len()
    );
    for header in headers {
        text.push_str(header);
        text.push_str("\r\n");
    }
    text.push_str("\r\n");
    text.push_str(&body);
    text
}

/// A client that records every wait instead of sleeping.
fn client(base: &str, limiter: Limiter) -> (Rest, Rc<RefCell<Vec<Duration>>>) {
    let waits = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&waits);
    let rest = Rest::new(base, "test-key", limiter).with_sleep(move |wait| {
        if !wait.is_zero() {
            seen.borrow_mut().push(wait);
        }
    });
    (rest, waits)
}

fn roomy() -> Limiter {
    Limiter::new(100, Duration::from_secs(60))
}

#[test]
fn a_public_client_carries_no_key_at_all() {
    // The season's leaderboard and its dates need no auth ("Season"), and the dashboard that reads
    // them holds no bot's key: a public client sends no Authorization header.
    let (base, served) = venue(vec![reply("200 OK", &[], &json!([]))]);
    let mut rest = Rest::public(&base, roomy());
    assert_eq!(
        rest.get("/season/leaderboard?min_hands=10").unwrap(),
        json!([])
    );
    let heads = served.join().unwrap();
    assert!(
        heads[0].starts_with("get /api/season/leaderboard?min_hands=10 "),
        "{}",
        heads[0]
    );
    assert!(!heads[0].contains("authorization"), "{}", heads[0]);
}

#[test]
fn retry_after_is_honoured_and_server_errors_back_off_but_a_refusal_is_final() {
    assert_eq!(retry_wait(429, Some("7"), 0), Some(Duration::from_secs(7)));
    assert_eq!(
        retry_wait(429, Some(" 2 "), 3),
        Some(Duration::from_secs(2))
    );
    // No usable header: back off instead.
    let unknown = retry_wait(429, Some("Wed, 21 Oct 2026 07:28:00 GMT"), 0).unwrap();
    assert!(unknown >= Duration::from_secs(1));
    let (first, later) = (retry_wait(503, None, 0), retry_wait(503, None, 3));
    assert!(first.unwrap() < later.unwrap());
    assert!(retry_wait(503, None, MAX_ATTEMPTS - 2).unwrap() <= Duration::from_secs(60));
    // The last try is the last: no wait after it.
    assert_eq!(retry_wait(503, None, MAX_ATTEMPTS - 1), None);
    assert_eq!(retry_wait(429, Some("1"), MAX_ATTEMPTS - 1), None);
    for final_status in [400, 401, 403, 404, 422] {
        assert_eq!(retry_wait(final_status, None, 0), None, "{final_status}");
    }
}

#[test]
fn a_429_waits_what_retry_after_says_then_the_request_goes_again_with_the_key() {
    let (base, served) = venue(vec![
        reply(
            "429 Too Many Requests",
            &["retry-after: 3"],
            &json!({"detail": "slow down"}),
        ),
        reply("200 OK", &[], &json!({"playing": false})),
    ]);
    let (mut rest, waits) = client(&base, roomy());
    assert_eq!(
        rest.get("/me/active-game").unwrap(),
        json!({"playing": false})
    );
    assert_eq!(*waits.borrow(), [Duration::from_secs(3)]);
    let heads = served.join().unwrap();
    assert_eq!(heads.len(), 2);
    for head in heads {
        assert!(head.starts_with("get /api/me/active-game "), "{head}");
        assert!(head.contains("authorization: bearer test-key"), "{head}");
    }
}

#[test]
fn a_refused_key_is_not_retried() {
    let (base, served) = venue(vec![reply(
        "401 Unauthorized",
        &[],
        &json!({"detail": "bad key"}),
    )]);
    let (mut rest, waits) = client(&base, roomy());
    assert!(matches!(rest.get("/me"), Err(RestError::AuthFailed)));
    assert_eq!(served.join().unwrap().len(), 1);
    assert!(waits.borrow().is_empty());
}

#[test]
fn every_request_on_one_key_shares_its_limiter() {
    let (base, served) = venue(vec![
        reply("200 OK", &[], &json!({})),
        reply("200 OK", &[], &json!({})),
    ]);
    let (mut rest, waits) = client(&base, Limiter::new(1, Duration::from_secs(60)));
    rest.get("/me").unwrap();
    rest.get("/me").unwrap();
    served.join().unwrap();
    let waited = waits.borrow();
    assert_eq!(waited.len(), 1, "{waited:?}");
    assert!(waited[0] > Duration::from_secs(55));
}

#[test]
fn pages_run_until_a_short_page_and_an_empty_page_backs_off_before_it_ends() {
    let page = |hands: Value| reply("200 OK", &[], &json!({ "hands": hands }));
    let mut replies = vec![page(json!([1, 2])), page(json!([3, 4]))];
    replies.extend((0..EMPTY_PAGE_TRIES).map(|_| page(json!([]))));
    let (base, served) = venue(replies);
    let (mut rest, waits) = client(&base, roomy());
    let mut pager = rest.pages("/me/hand-history", 2, |page| {
        page["hands"].as_array().cloned()
    });
    assert_eq!(pager.next_page().unwrap(), Some(vec![json!(1), json!(2)]));
    assert_eq!(pager.next_page().unwrap(), Some(vec![json!(3), json!(4)]));
    assert_eq!(pager.next_page().unwrap(), None);
    assert_eq!(pager.next_page().unwrap(), None, "the end stays the end");
    let heads = served.join().unwrap();
    assert!(
        heads[0].contains("/api/me/hand-history?limit=2&offset=0 "),
        "{}",
        heads[0]
    );
    assert!(heads[1].contains("offset=2 "));
    assert!(heads[2].contains("offset=4 "));
    assert_eq!(heads.len(), 2 + EMPTY_PAGE_TRIES as usize);
    // A wait before each retry of the empty page, each longer than the last.
    let waited = waits.borrow();
    assert_eq!(waited.len(), EMPTY_PAGE_TRIES as usize - 1);
    assert!(
        waited.windows(2).all(|pair| pair[0] < pair[1]),
        "{waited:?}"
    );
}

#[test]
fn a_short_page_is_the_end_without_another_request() {
    let (base, served) = venue(vec![reply("200 OK", &[], &json!({"hands": [1]}))]);
    let (mut rest, _) = client(&base, roomy());
    let mut pager = rest.pages("/me/hand-history", 2, |page| {
        page["hands"].as_array().cloned()
    });
    assert_eq!(pager.next_page().unwrap(), Some(vec![json!(1)]));
    assert_eq!(pager.next_page().unwrap(), None);
    assert_eq!(served.join().unwrap().len(), 1);
}

use super::*;
use axum::{Json, Router, extract::State as AxState, http::StatusCode, routing::post};
use parking_lot::Mutex;
use std::collections::VecDeque;

const ON: Settings = Settings { enabled: true, max_seasons: 3 };

fn free() -> Tier {
    Tier::Free
}

#[test]
fn off_means_no_purchase_whatever_the_tier() {
    for tier in [Tier::Free, Tier::Pro, Tier::Unknown] {
        assert_eq!(plan(Settings { enabled: false, ..ON }, &State::default(), tier, 1_000.0), Plan::Off);
    }
}

#[test]
fn only_a_measured_free_tier_buys_and_the_ladder_stops_at_the_ceiling() {
    assert_eq!(plan(ON, &State::default(), Tier::Pro, 1.0), Plan::Nothing("Pro is active"));
    assert_eq!(plan(ON, &State::default(), Tier::Unknown, 1.0), Plan::Nothing("tier not read"));
    assert_eq!(plan(ON, &State::default(), free(), 1.0), Plan::Buy { ladder: vec![3, 1] }, "default ceiling is 3 seasons");
    assert_eq!(plan(Settings { max_seasons: 6, ..ON }, &State::default(), free(), 1.0), Plan::Buy { ladder: vec![6, 3, 1] });
    assert_eq!(plan(Settings { max_seasons: 1, ..ON }, &State::default(), free(), 1.0), Plan::Buy { ladder: vec![1] });
}

#[test]
fn a_request_of_unknown_fate_is_retried_with_its_own_id_and_then_gives_up() {
    let pending = Pending { request_id: "rid-1".into(), seasons: 3, at: 1_000.0 };
    let mut s = State { pending: Some(pending), last_attempt_at: Some(1_000.0), ..State::default() };
    assert_eq!(plan(ON, &s, free(), 1_100.0), Plan::Nothing("retrying shortly"));
    assert_eq!(plan(ON, &s, free(), 1_400.0), Plan::Retry { seasons: 3, request_id: "rid-1".into() }, "same id, same bundle");
    assert_eq!(plan(ON, &s, Tier::Pro, 1_400.0), Plan::Nothing("Pro is active"), "a renewal that did land needs no retry");
    s.pending.as_mut().unwrap().at = 1_000.0 - 25.0 * 3600.0;
    s.last_attempt_at = None;
    assert!(matches!(plan(ON, &s, free(), 1_000.0), Plan::Buy { .. }), "after a day the old id is dropped for a fresh decision");
}

#[test]
fn a_purchase_that_did_not_turn_the_tier_to_pro_is_never_repeated_for_a_week() {
    let bought = Receipt { seasons_purchased: 3, seasons_remaining: 3, amount_charged_cents: 1200, request_id: "r".into(), at: 1_000.0 };
    let s = State { receipt: Some(bought), ..State::default() };
    let within = 1_000.0 + 6.0 * 24.0 * 3600.0;
    assert!(matches!(plan(ON, &s, free(), within), Plan::Nothing(why) if why.contains("still reads Free")));
    assert!(matches!(plan(ON, &s, free(), 1_000.0 + 8.0 * 24.0 * 3600.0), Plan::Buy { .. }), "a real lapse a week and more later renews");
}

#[test]
fn nothing_more_is_bought_for_six_hours_after_a_purchase() {
    let s = State { quiet_until: 10_000.0, ..State::default() };
    assert_eq!(plan(ON, &s, free(), 9_999.0), Plan::Nothing("a renewal was tried recently"));
    assert!(matches!(plan(ON, &s, free(), 10_000.0), Plan::Buy { .. }));
}

#[test]
fn the_tier_comes_from_pro_tier_alone() {
    assert_eq!(Tier::of(Some(&json!({"pro_tier": true}))), Tier::Pro);
    assert_eq!(Tier::of(Some(&json!({"pro_tier": false}))), Tier::Free);
    assert_eq!(Tier::of(Some(&json!({"detail": "error"}))), Tier::Unknown);
    assert_eq!(Tier::of(None), Tier::Unknown);
}

/// A scripted stand-in for `POST /season/pro-bundle`: answers in order, records what it was asked.
#[derive(Clone, Default)]
struct Mock {
    script: Arc<Mutex<VecDeque<(u16, Value)>>>,
    seen: Arc<Mutex<Vec<Value>>>,
    events: Arc<Mutex<Vec<String>>>,
}

async fn bundle(AxState(m): AxState<Mock>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    m.events.lock().push(format!("post {}", body["request_id"].as_str().unwrap_or("?")));
    m.seen.lock().push(body);
    let (code, answer) = m.script.lock().pop_front().unwrap_or((500, json!({})));
    (StatusCode::from_u16(code).unwrap(), Json(answer))
}

async fn serve(script: &[(u16, Value)]) -> (String, Mock) {
    let mock = Mock { script: Arc::new(Mutex::new(script.iter().cloned().collect())), ..Mock::default() };
    let app = Router::new().route("/season/pro-bundle", post(bundle)).with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, mock)
}

fn receipt_body(purchased: i64, remaining: i64, cents: i64) -> Value {
    json!({"seasons_purchased": purchased, "seasons_remaining": remaining, "amount_charged_cents": cents})
}

#[tokio::test]
async fn a_402_steps_down_the_ladder_and_each_rung_has_its_own_id_saved_before_the_request() {
    let (base, mock) = serve(&[(402, json!({})), (200, receipt_body(1, 1, 500))]).await;
    let http = reqwest::Client::new();
    let rest = Rest { http: &http, base: &base, key: "k" };
    let mut state = State::default();
    let events = mock.events.clone();
    let mut save = |s: &State| events.lock().push(format!("save {}", s.pending.as_ref().map_or("none", |p| p.request_id.as_str())));
    let got = execute(&rest, &mut state, Plan::Buy { ladder: vec![3, 1] }, 5_000.0, &mut save).await.expect("the one-season rung buys");
    assert_eq!((got.seasons_purchased, got.amount_charged_cents), (1, 500));
    let seen = mock.seen.lock().clone();
    assert_eq!(seen.iter().map(|b| b["seasons"].as_i64().unwrap()).collect::<Vec<_>>(), vec![3, 1]);
    assert_ne!(seen[0]["request_id"], seen[1]["request_id"], "a different bundle is a different purchase");
    let log = mock.events.lock().clone();
    for b in &seen {
        let id = b["request_id"].as_str().unwrap();
        let (saved, posted) = (log.iter().position(|e| e == &format!("save {id}")), log.iter().position(|e| e == &format!("post {id}")));
        assert!(saved.unwrap() < posted.unwrap(), "{id} is on disk before it is sent: {log:?}");
    }
    assert!(state.pending.is_none() && state.receipt.is_some() && state.quiet_until == 5_000.0 + QUIET_AFTER_PURCHASE_SECS);
}

#[tokio::test]
async fn no_credit_on_any_rung_buys_nothing_and_says_so() {
    let (base, mock) = serve(&[(402, json!({})), (402, json!({}))]).await;
    let http = reqwest::Client::new();
    let rest = Rest { http: &http, base: &base, key: "k" };
    let mut state = State::default();
    let got = execute(&rest, &mut state, Plan::Buy { ladder: vec![3, 1] }, 1.0, &mut |_| {}).await;
    assert!(got.is_none() && state.receipt.is_none() && state.pending.is_none());
    assert_eq!(mock.seen.lock().len(), 2);
    assert!(state.last_error.as_deref().is_some_and(|e| e.contains("credit")), "{:?}", state.last_error);
    assert_eq!(state.quiet_until, 1.0 + BACKOFF_SECS, "and waits before trying again");
}

#[tokio::test]
async fn a_lost_answer_keeps_the_id_and_the_retry_reuses_it() {
    let (base, mock) = serve(&[(500, json!({})), (200, receipt_body(3, 3, 1200))]).await;
    let http = reqwest::Client::new();
    let rest = Rest { http: &http, base: &base, key: "k" };
    let mut state = State::default();
    assert!(execute(&rest, &mut state, Plan::Buy { ladder: vec![3, 1] }, 1_000.0, &mut |_| {}).await.is_none());
    let pending = state.pending.clone().expect("the request of unknown fate stays pending");
    assert_eq!(mock.seen.lock().len(), 1, "an error is not a reason to try the cheaper rung: the first may have been charged");
    let again = plan(ON, &state, free(), 1_000.0 + RETRY_GAP_SECS);
    assert_eq!(again, Plan::Retry { seasons: 3, request_id: pending.request_id.clone() });
    let got = execute(&rest, &mut state, again, 1_000.0 + RETRY_GAP_SECS, &mut |_| {}).await.expect("the retry lands");
    assert_eq!((got.seasons_purchased, got.amount_charged_cents, got.request_id.as_str()), (3, 1200, pending.request_id.as_str()));
    let ids: Vec<_> = mock.seen.lock().iter().map(|b| b["request_id"].as_str().unwrap().to_string()).collect();
    assert_eq!(ids[0], ids[1], "one purchase, one id, however many tries");
    assert!(state.pending.is_none());
}

#[tokio::test]
async fn off_and_nothing_make_no_request_at_all() {
    let (base, mock) = serve(&[]).await;
    let http = reqwest::Client::new();
    let rest = Rest { http: &http, base: &base, key: "k" };
    let mut state = State::default();
    for p in [Plan::Off, Plan::Nothing("Pro is active")] {
        assert!(execute(&rest, &mut state, p, 1.0, &mut |_| panic!("nothing to save")).await.is_none());
    }
    assert!(mock.seen.lock().is_empty());
    assert_eq!(state, State::default(), "not even last_attempt_at moves");
}

/// The operator's rule (2026-10-04): renewal spends the account's credit balance and nothing else, never
/// real money. The one purchase call is `POST /season/pro-bundle` ("from credit balance", `402` when the
/// balance is short); the token purchase, the single-season endpoint and anything wallet-shaped must
/// never appear in the module.
#[test]
fn only_the_credit_bundle_endpoint_is_ever_called() {
    let source = include_str!("../proauto.rs");
    assert!(source.contains("/season/pro-bundle"), "the credit bundle endpoint is the purchase");
    for forbidden in ["season/pro/token", "season/pro/quote", "/season/pro\"", "wallet", "deposit", "tx_hash", "usdc", "stripe", "payment"]
    {
        assert!(!source.to_ascii_lowercase().contains(forbidden), "proauto.rs must not mention {forbidden}");
    }
    assert_eq!(source.matches(".post(").count(), 1, "exactly one POST in the module");
}

#[test]
fn settings_default_off_and_snap_to_a_real_bundle() {
    let with = |pairs: &'static [(&'static str, &'static str)]| {
        Settings::parse(|k| pairs.iter().find(|(key, _)| *key == k).map(|(_, v)| v.to_string()))
    };
    assert_eq!(with(&[]), Settings { enabled: false, max_seasons: 3 }, "unset is off, three seasons");
    assert!(!with(&[("SVANBOT_AUTO_RENEW_PRO", "0")]).enabled);
    assert!(!with(&[("SVANBOT_AUTO_RENEW_PRO", "yes")]).enabled, "only 1 or true turns it on");
    assert!(with(&[("SVANBOT_AUTO_RENEW_PRO", " 1 ")]).enabled);
    for (raw, want) in [("1", 1), ("3", 3), ("6", 6), ("2", 3), ("12", 3), ("-1", 3), ("many", 3)] {
        let got = Settings::parse(|k| (k == "SVANBOT_AUTO_RENEW_SEASONS").then(|| raw.to_string())).max_seasons;
        assert_eq!(got, want, "SVANBOT_AUTO_RENEW_SEASONS={raw}");
    }
}

//! The replayer's routes: one bot's traced hands, one hand's trace, and the two ways a store or a
// name can be wrong. Separate from the fleet's tests because it is a separate page.

#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

mod support;

use serde_json::Value;
use support::{get, serve, traced_store, undecodable_trace};

#[test]
fn the_hands_route_lists_a_bots_traced_hands_newest_first() {
    let (addr, dir) = serve("hands-route");
    traced_store(&dir.join("data"), "Alpha");
    let (code, body) = get(addr, "/api/hands/Alpha");
    assert_eq!(code, 200);
    let listed: Value = serde_json::from_str(&body).unwrap();
    let rows = listed["hands"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "only the traced hand is listed");
    assert_eq!(rows[0]["hand_id"], "h-1");
    assert_eq!(rows[0]["decisions"], 1);
}

#[test]
fn the_trace_route_reads_one_hands_decisions() {
    let (addr, dir) = serve("trace-route");
    traced_store(&dir.join("data"), "Alpha");
    let (code, body) = get(addr, "/api/trace/Alpha/h-1");
    assert_eq!(code, 200);
    let trace: Value = serde_json::from_str(&body).unwrap();
    let decisions = trace["decisions"].as_array().unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0]["branch"], "priced");
    assert_eq!(decisions[0]["street"], "preflop");
    assert_eq!(decisions[0]["price"], 0.25);
    // The trace carries the cards the decision was made with, so the replayer needs nothing
    // from the hand's frames to say what the bot held.
    assert_eq!(decisions[0]["hole"], "AhAs");
    assert_eq!(decisions[0]["board"], "");
    // The store keeps what the pricer read by, and the route passes it through.
    assert_eq!(decisions[0]["exploit"], 0.4);
    // Each range names a seat, which is what a reader can find at the table, not a position in
    // the list.
    assert_eq!(decisions[0]["ranges"][0]["seat"], 3);
}

#[test]
fn a_name_no_bot_can_have_is_a_404_on_both_json_routes() {
    // The page route refuses a name that is not a name; the JSON routes refuse it too, rather
    // than echoing it back as a bot with no hands.
    let (addr, _dir) = serve("bad-name-route");
    for path in ["/api/hands/..%2f..", "/api/hands/a/b", "/api/hands/"] {
        let (code, body) = get(addr, path);
        assert_eq!(code, 404, "{path}: {body}");
    }
    let (code, _) = get(addr, "/api/trace/..%2f../h-1");
    assert_eq!(code, 404);
}

#[test]
fn a_hands_route_that_cannot_read_the_store_is_not_an_empty_list() {
    // A store that exists and cannot be read is not a bot that has played nothing: the page is
    // told the difference, so a corrupt store does not read as an idle bot.
    let (addr, dir) = serve("broken-store-route");
    let data = dir.join("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("Broken.db"), b"not a database at all").unwrap();
    let (code, body) = get(addr, "/api/hands/Broken");
    assert_eq!(code, 500, "{body}");
    assert!(body.contains("cannot be read"), "{body}");
    // The trace route makes the same distinction: a broken store is not a hand nobody played.
    let (code, body) = get(addr, "/api/trace/Broken/h-1");
    assert_eq!(code, 500, "{body}");
    assert!(body.contains("cannot be read"), "{body}");
}

#[test]
fn an_untraced_hand_says_so_rather_than_rendering_an_empty_trace() {
    let (addr, dir) = serve("untraced-route");
    traced_store(&dir.join("data"), "Alpha");
    let (code, body) = get(addr, "/api/trace/Alpha/h-2");
    assert_eq!(code, 200);
    let trace: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(trace["traced"], false);
    assert!(trace["decisions"].as_array().unwrap().is_empty());
    assert!(
        trace["note"].as_str().unwrap().contains("no trace"),
        "the page is told why it has nothing to show"
    );
}

#[test]
fn a_hand_the_store_does_not_hold_is_a_404() {
    let (addr, dir) = serve("missing-hand-route");
    traced_store(&dir.join("data"), "Alpha");
    let (code, _) = get(addr, "/api/trace/Alpha/h-missing");
    assert_eq!(code, 404);
}

#[test]
fn a_trace_this_code_cannot_read_is_a_500_not_an_untraced_hand() {
    // A decision the codec cannot read is not a hand with no trace: the hands list counts that
    // decision, so a 200 that said "no trace" would show the page an empty hand the store says it
    // holds a decision for. The two routes agree that the store cannot be read.
    let (addr, dir) = serve("undecodable-trace-route");
    undecodable_trace(&dir.join("data"), "Alpha");
    let (code, body) = get(addr, "/api/hands/Alpha");
    assert_eq!(code, 200, "{body}");
    let listed: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        listed["hands"][0]["decisions"], 1,
        "the hands list counts it"
    );
    let (code, body) = get(addr, "/api/trace/Alpha/h-3");
    assert_eq!(code, 500, "{body}");
    assert!(body.contains("trace cannot be read"), "{body}");
}

#[test]
fn a_bot_with_no_store_lists_no_hands_rather_than_failing() {
    // A bot that has just started has decided nothing yet, and the page asks before its store
    // exists.
    let (addr, _dir) = serve("no-store-route");
    let (code, body) = get(addr, "/api/hands/Nobody");
    assert_eq!(code, 200);
    let listed: Value = serde_json::from_str(&body).unwrap();
    assert!(listed["hands"].as_array().unwrap().is_empty());
    // The trace route agrees: a bot with no store has no hand, so this is a 404 — not a 500 that
    // would say the store is broken.
    let (code, _) = get(addr, "/api/trace/Nobody/h-1");
    assert_eq!(code, 404);
}

#[test]
fn a_hands_page_is_the_dashboard_document_and_nothing_else_is() {
    // The page is one document the script routes: `/bot/Alpha/hand/h-1` is the replayer's page,
    // and anything deeper is not a page at all.
    let (addr, _dir) = serve("hand-page");
    let (code, body) = get(addr, "/bot/Alpha/hand/h-1");
    assert_eq!(code, 200);
    assert!(body.contains("<!doctype html"), "the dashboard document");
    let (code, _) = get(addr, "/bot/Alpha/hand/h-1/extra");
    assert_eq!(code, 404);
    let (code, _) = get(addr, "/bot/Alpha/hand/../../etc/passwd");
    assert_eq!(code, 404);
}

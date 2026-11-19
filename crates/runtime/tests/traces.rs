//! The store's traces: the record of what made each decision, kept with its hand and capped, and
//! the reader that lets the dashboard look without claiming the bot's store.

#![allow(clippy::disallowed_methods, clippy::disallowed_types)]

mod support;

use serde_json::json;
use support::{hand, scratch};
use svanbot_core::card::CardSet;
use svanbot_core::rules::Street;
use svanbot_runtime::record::{Hand, PENDING_LIMIT, Recorder};
use svanbot_runtime::store::{Source, Store};
use svanbot_runtime::wire::parse_frame;
use svanbot_strategy::policy::{Abstract, Branch, PricedTrace, Ranged, ResolvedTrace, Trace};

/// A trace the policy might have made, as the store keeps it.
fn trace(branch: Branch) -> Trace {
    match branch {
        Branch::Priced => Trace {
            branch,
            street: Street::Preflop,
            hole: "AhAs".parse().expect("a hole"),
            board: CardSet::EMPTY,
            resolved: None,
            priced: Some(PricedTrace {
                equity: 0.61,
                price: Some(0.25),
                exploit: Some(0.4),
                ranges: vec![Ranged {
                    ordinal: 0,
                    masses: vec![0.0, 0.0, 0.1, 0.2, 0.3, 0.2, 0.1, 0.1],
                }],
            }),
        },
        Branch::Resolved => Trace {
            branch,
            street: Street::Flop,
            hole: "AhKh".parse().expect("a hole"),
            board: "QhJhTh".parse().expect("a board"),
            resolved: Some(ResolvedTrace {
                values: vec![1.5, -0.25],
                strategy: vec![0.75, 0.25],
                taken: 0,
                actions: vec![Abstract::Check, Abstract::Bet(200)],
                ranges: vec![Ranged {
                    ordinal: 0,
                    masses: vec![0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                }],
            }),
            priced: None,
        },
    }
}

fn traced_hand(id: &str, branches: &[Branch]) -> Hand {
    Hand {
        hand_id: id.into(),
        table_id: "t-1".into(),
        frames: vec![
            json!({"type": "hand_start", "hand_id": id}).to_string(),
            json!({"type": "hand_result", "hand_id": id}).to_string(),
        ],
        traced: branches.iter().copied().map(trace).collect(),
    }
}

#[test]
fn a_hand_keeps_the_traces_that_made_its_decisions() {
    let path = scratch("traces").join("SurSvan.db");
    let store = Store::open(&path, "SurSvan").unwrap();
    let hand = traced_hand("h-1", &[Branch::Priced, Branch::Resolved, Branch::Priced]);
    assert!(store.append_hand(&hand, Source::Live, 1_000).unwrap());
    let kept = store.traces("h-1").unwrap().expect("a traced hand");
    assert_eq!(kept.len(), 3);
    assert_eq!(kept[0].branch, Branch::Priced);
    assert_eq!(kept[1].branch, Branch::Resolved);
    assert_eq!(kept[1].resolved.as_ref().unwrap().values, [1.5, -0.25]);
    assert_eq!(kept[2].branch, Branch::Priced);
}

#[test]
fn a_hand_with_no_traces_is_untraced_rather_than_empty() {
    // A hand played before the trace existed is not a hand that made no decisions, and the
    // replayer must tell the two apart.
    let path = scratch("untraced").join("SurSvan.db");
    let store = Store::open(&path, "SurSvan").unwrap();
    assert!(
        store
            .append_hand(&hand("h-1"), Source::Live, 1_000)
            .unwrap()
    );
    assert!(store.traces("h-1").unwrap().is_none());
    // A hand the store does not hold at all is a third thing again.
    assert!(store.traces("h-missing").unwrap().is_none());
}

#[test]
fn a_hand_that_was_kept_once_keeps_no_second_copy_of_its_traces() {
    let path = scratch("traces-once").join("SurSvan.db");
    let store = Store::open(&path, "SurSvan").unwrap();
    let hand = traced_hand("h-1", &[Branch::Priced]);
    assert!(store.append_hand(&hand, Source::Live, 1_000).unwrap());
    assert!(!store.append_hand(&hand, Source::Live, 2_000).unwrap());
    assert_eq!(store.traces("h-1").unwrap().unwrap().len(), 1);
}

#[test]
fn the_oldest_traced_hands_fall_off_past_the_cap() {
    // The cap is the newest hands, so a long-running bot's store does not grow without limit.
    let path = scratch("cap").join("SurSvan.db");
    let store = Store::open(&path, "SurSvan").unwrap();
    let over = svanbot_runtime::store::TRACED_HANDS + 3;
    for number in 0..over {
        let hand = traced_hand(&format!("h-{number}"), &[Branch::Priced]);
        // Each hand recorded a moment later than the last, so the cap's order has something to
        // read: with one `recorded_ms` for all of them the test would only ever exercise `rowid`.
        assert!(
            store
                .append_hand(&hand, Source::Live, 1_000 + i64::try_from(number).unwrap())
                .unwrap()
        );
    }
    // The newest are all there.
    assert!(
        store.traces(&format!("h-{}", over - 1)).unwrap().is_some(),
        "the newest hand is traced"
    );
    // The three that fell off the start are not.
    for number in 0..3 {
        assert!(
            store.traces(&format!("h-{number}")).unwrap().is_none(),
            "h-{number} fell off the cap"
        );
    }
    assert_eq!(
        store.traced_hands().unwrap().len(),
        svanbot_runtime::store::TRACED_HANDS
    );
}

#[test]
fn a_traced_hand_is_capped_by_its_recorded_time_not_its_insert_order() {
    // The cap orders by `recorded_ms`, then `rowid`. A hand written last with an earlier recorded
    // time — a backfill, or two writes in one batch — falls off before the newer ones, so the
    // order is the recorded clock and only the tie-break is insertion order.
    let path = scratch("cap-order").join("SurSvan.db");
    let store = Store::open(&path, "SurSvan").unwrap();
    for number in 0..svanbot_runtime::store::TRACED_HANDS {
        let hand = traced_hand(&format!("new-{number}"), &[Branch::Priced]);
        assert!(
            store
                .append_hand(&hand, Source::Live, 10_000 + i64::try_from(number).unwrap())
                .unwrap()
        );
    }
    // One more, written last but recorded a minute before any of them.
    assert!(
        store
            .append_hand(&traced_hand("late", &[Branch::Priced]), Source::Live, 9_000)
            .unwrap()
    );
    assert!(
        store.traces("late").unwrap().is_none(),
        "the late-written old hand is the one the cap drops"
    );
    assert!(
        store.traces("new-0").unwrap().is_some(),
        "the oldest of the new hands stays"
    );
}

#[test]
fn the_hands_list_names_the_newest_traced_hands_first() {
    let path = scratch("hand-list").join("SurSvan.db");
    let store = Store::open(&path, "SurSvan").unwrap();
    for number in 0..3 {
        let hand = traced_hand(&format!("h-{number}"), &[Branch::Priced]);
        assert!(
            store
                .append_hand(&hand, Source::Live, 1_000 + i64::from(number))
                .unwrap()
        );
    }
    assert!(
        store
            .append_hand(&hand("h-untraced"), Source::Live, 9_000)
            .unwrap()
    );
    let listed = store.traced_hands().unwrap();
    let ids: Vec<&str> = listed.iter().map(|hand| hand.hand_id.as_str()).collect();
    assert_eq!(ids, ["h-2", "h-1", "h-0"]);
    assert_eq!(listed[0].decisions, 1);
}

#[test]
fn a_reader_reads_a_bots_store_without_claiming_it() {
    // The dashboard is not the bot: it takes one look, and it must neither need the bot's name nor
    // be able to write through what it opened.
    let path = scratch("reader").join("SurSvan.db");
    let store = Store::open(&path, "SurSvan").unwrap();
    assert!(
        store
            .append_hand(&traced_hand("h-1", &[Branch::Priced]), Source::Live, 1_000)
            .unwrap()
    );
    let reader = Store::open_reader(&path).unwrap();
    assert_eq!(reader.traces("h-1").unwrap().unwrap().len(), 1);
    assert_eq!(reader.traced_hands().unwrap().len(), 1);
    // Read-only is read-only: the writer's own path is closed to a reader.
    assert!(
        reader
            .append_hand(&hand("h-2"), Source::Live, 2_000)
            .is_err(),
        "a reader may not write"
    );
}

#[test]
fn a_value_with_no_json_spelling_keeps_its_hand_and_loses_only_itself() {
    // `NaN` and the infinities have no JSON number, and the column's `json_valid` would refuse
    // the whole row — losing a decision the bot played. The value is written as `null` instead:
    // a recorded hole in the values is better than a hand the store never kept.
    let path = scratch("non-finite").join("SurSvan.db");
    let store = Store::open(&path, "SurSvan").unwrap();
    let mut hand = traced_hand("h-1", &[Branch::Resolved]);
    let resolved = hand.traced[0].resolved.as_mut().expect("resolved");
    resolved.values = vec![f64::NAN, f64::INFINITY, 1.5];
    assert!(store.append_hand(&hand, Source::Live, 1_000).unwrap());
    let kept = store.traces("h-1").unwrap().expect("the hand survived");
    let values = &kept[0].resolved.as_ref().expect("resolved").values;
    assert_eq!(values.len(), 3);
    assert_eq!(values[2], 1.5, "the finite value reads back as written");
    // A JSON `null` reads back as `NaN`, which is finite nothing but is not the number it was.
    assert!(values[0].is_nan());
}

#[test]
fn a_trace_waiting_for_a_hand_that_never_opens_is_not_held_forever() {
    // A trace can arrive before its hand when a resync replays a turn first. One whose hand never
    // arrives — a connection that is dropped mid-replay — must not accumulate for the life of the
    // process, so past a hand's worth of waiting traces the oldest are dropped.
    let mut recorder = Recorder::default();
    for number in 0..(PENDING_LIMIT + 5) {
        recorder.trace(vec![(format!("h-{number}"), preflop_trace())]);
    }
    // The newest wait for their hand; the earliest are gone rather than held.
    let hand = recorder_hand(&mut recorder, &format!("h-{}", PENDING_LIMIT + 4));
    assert_eq!(hand.traced.len(), 1, "the newest waiting trace is kept");
    let old = recorder_hand(&mut recorder, "h-0");
    assert!(
        old.traced.is_empty(),
        "the oldest waiting trace was dropped"
    );
}

/// A hand's start frame, as the venue sends it.
fn hand_start(hand_id: &str) -> String {
    json!({"type": "hand_start", "hand_id": hand_id, "seat": 2, "dealer_seat": 0,
           "blinds": {"small_blind": 10, "big_blind": 20}})
    .to_string()
}

/// The frames of one hand, as the recorder takes them.
fn recorder_hand(recorder: &mut Recorder, hand_id: &str) -> Hand {
    let texts = [
        hand_start(hand_id),
        json!({"type": "hand_result", "hand_id": hand_id, "final_stacks": {"2": 2000}}).to_string(),
    ];
    let mut finished = Vec::new();
    for text in &texts {
        let frame = parse_frame(text).unwrap();
        finished.extend(recorder.record(text, &frame));
    }
    finished.pop().expect("the hand is closed")
}

fn preflop_trace() -> Trace {
    Trace {
        branch: Branch::Priced,
        street: Street::Preflop,
        hole: "AhAs".parse().expect("a hole"),
        board: CardSet::EMPTY,
        resolved: None,
        priced: Some(PricedTrace {
            equity: 0.5,
            price: Some(0.25),
            exploit: None,
            ranges: Vec::new(),
        }),
    }
}

#[test]
fn a_traced_decision_reaches_the_hand_it_was_made_in() {
    // The session reports a trace with its hand id; the recorder is the only thing that holds
    // both the trace and the hand, so it files it there.
    let mut recorder = Recorder::default();
    let trace = preflop_trace();
    // A trace may arrive before its hand: a replayed turn can precede the hand start on a resync.
    recorder.trace(vec![("h-1".to_owned(), trace.clone())]);
    let hand = recorder_hand(&mut recorder, "h-1");
    assert_eq!(hand.traced, vec![trace]);
}

#[test]
fn a_trace_for_another_hand_is_not_filed_in_this_one() {
    // A trace belongs to its hand or nowhere. One that names a different hand would have the
    // replayer explain a decision that was not made here.
    let mut recorder = Recorder::default();
    let text = hand_start("h-1");
    recorder.record(&text, &parse_frame(&text).unwrap());
    recorder.trace(vec![("h-2".to_owned(), preflop_trace())]);
    let hand = recorder_hand_after_start(&mut recorder, "h-1");
    assert!(hand.traced.is_empty(), "h-2's trace is not h-1's");
}

#[test]
fn a_trace_for_a_hand_after_this_one_waits_for_it_rather_than_being_dropped() {
    // A resync can replay the next hand's turn while this one is still open. That trace is not
    // this hand's, and it is not garbage either: it waits and is filed when its own hand opens.
    let mut recorder = Recorder::default();
    let text = hand_start("h-1");
    recorder.record(&text, &parse_frame(&text).unwrap());
    let trace = preflop_trace();
    recorder.trace(vec![("h-2".to_owned(), trace.clone())]);
    let first = recorder_hand_after_start(&mut recorder, "h-1");
    assert!(first.traced.is_empty(), "h-2's trace is not h-1's");
    // The hand it was for opens next, and takes it.
    let second = recorder_hand(&mut recorder, "h-2");
    assert_eq!(second.traced, vec![trace], "h-2's trace reached h-2");
}

/// Closes an already-started hand `hand_id` and returns it.
fn recorder_hand_after_start(recorder: &mut Recorder, hand_id: &str) -> Hand {
    let text =
        json!({"type": "hand_result", "hand_id": hand_id, "final_stacks": {"2": 2000}}).to_string();
    recorder
        .record(&text, &parse_frame(&text).unwrap())
        .expect("the hand is closed")
}

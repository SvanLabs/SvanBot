//! Tests for the dashboard's state payload: each panel's rows and the fields they promise.
//!
//! Split out of `state.rs` (0320: the 500-line rule).

use super::*;

#[test]
fn the_champion_carries_every_knob_the_dashboard_shows() {
    // 2026-09-27: the Champion profile read "Short-stack open 0.00" and "Preflop jam at or below
    // 0.00" (live values 2.5 and 30): the panel lists knobs the API never sent.
    let web = include_str!("../../../../../../web/src/training.tsx");
    let list = web.split("const knobs = [").nth(1).and_then(|r| r.split("] as const").next()).expect("the knob list");
    let keys: Vec<&str> = list.split("['").skip(1).filter_map(|k| k.split('\'').next()).collect();
    assert!(keys.len() >= 17, "{keys:?}");
    let shared = Shared::for_test("champion-knobs", &["A"]);
    let t = training_json(&shared);
    for key in keys {
        assert!(t["champion"][key].is_number(), "champion lacks {key}");
    }
}

#[test]
fn training_json_names_stale_loops_instead_of_hiding_them() {
    // An empty store has reported nothing: every loop reads stale, deterministically.
    let shared = Shared::for_test("stale-loops", &["A"]);
    let training = training_json(&shared);
    let names: Vec<&str> = training["stale_loops"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["learner", "analyst", "fold calibration", "backups", "experiment poller"]);
}

#[test]
fn the_state_hash_row_carries_this_run_lifetime_and_the_newest_mismatch() {
    // 0301: a hot swap happens every ~20 minutes live, and the Runtime health panel read
    // "0 checks" again after each one because only this process's counters existed.
    let shared = Shared::for_test("state-hash-row", &["A", "B"]);
    shared.update(0, |b| {
        crate::live::count_state_hash(b, true, None);
        crate::live::count_state_hash(b, true, None);
        crate::live::count_state_hash(
            b,
            false,
            Some(crate::live::StateHashMismatch {
                at: 1_700_000_000.0,
                table: "t1".into(),
                verdict: "DIVERGED".into(),
                summary: "DIVERGED table_seq=42 (last applied 41)".into(),
            }),
        );
    });
    shared.update(1, |b| crate::live::count_state_hash(b, true, None));
    let a = state_hash_json(&shared.bots[0].read());
    assert_eq!((a["ok"].as_u64(), a["bad"].as_u64()), (Some(2), Some(1)));
    assert_eq!((a["lifetime_ok"].as_u64(), a["lifetime_bad"].as_u64()), (Some(2), Some(1)));
    assert_eq!(a["last_mismatch"]["table"], "t1");
    assert_eq!(a["last_mismatch"]["verdict"], "DIVERGED");
    assert_eq!(a["last_mismatch"]["at"], 1_700_000_000.0);
    assert!(state_hash_json(&shared.bots[1].read())["last_mismatch"].is_null(), "no mismatch, no row");
    let fleet = fleet_state_hash(&shared);
    assert_eq!((fleet["ok"].as_u64(), fleet["bad"].as_u64()), (Some(3), Some(1)));
    assert_eq!((fleet["lifetime_ok"].as_u64(), fleet["lifetime_bad"].as_u64()), (Some(3), Some(1)));
    assert_eq!(fleet["last_mismatch"]["bot"], "A", "the fleet row names the bot that saw it");
    // The dashboard reads it from the snapshot, not only from a bot's own metrics.
    assert_eq!(snapshot(&shared)["state_hash"]["lifetime_bad"], 1);
}

fn seated(shared: &Shared) {
    shared.update(0, |b| {
        b.seat = Some(1);
        b.pot = 123;
        b.seats = vec![
            sv10_venue::tracker::SeatView { seat: 1, name: "A".into(), stack: 2_000, in_hand: true, ..Default::default() },
            sv10_venue::tracker::SeatView { seat: 3, name: "villain".into(), stack: 1_500, in_hand: true, ..Default::default() },
        ];
    });
}

#[test]
fn the_table_payload_reads_each_opponent_and_leaves_metrics_to_the_snapshot() {
    let shared = Shared::for_test("table-json", &["A"]);
    shared.models.write().players.entry("villain".into()).or_default().hands = 250.0;
    seated(&shared);
    let b = shared.bots[0].read().clone();
    let table = table_json(&shared, &b);
    assert!(table.get("metrics").is_none(), "the realtime payload stays small");
    let seats = table["seats"].as_array().unwrap();
    assert!(seats[0]["read"].is_null(), "no read of ourselves");
    assert_eq!(seats[1]["read"]["hands"], json!(250.0));
    assert!(seats[1]["read"]["vpip"].as_f64().is_some() && seats[1]["read"]["response_ratio"].is_null());
    assert!(seats[1]["read"]["size_tell"].is_null(), "no tell installed, none shown");
    assert!(bot_json(&shared, &b).get("metrics").is_some());
}

#[tokio::test]
async fn the_stream_pushes_a_table_update_right_after_a_change() {
    use futures_util::StreamExt;
    let shared = Shared::for_test("table-stream", &["A"]);
    let mut body = events(State(shared.clone())).await.into_response().into_body().into_data_stream();
    let first = String::from_utf8(body.next().await.unwrap().unwrap().to_vec()).unwrap();
    assert!(first.starts_with("event: state"), "{}", &first[..first.len().min(80)]);
    seated(&shared);
    let t0 = Instant::now();
    let mut seen = String::new();
    while !seen.contains("event: table") {
        let chunk = tokio::time::timeout(Duration::from_secs(2), body.next()).await.expect("a table event within 2 s").unwrap().unwrap();
        seen.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(t0.elapsed() < STATE_EVERY, "table updates do not wait for the snapshot");
    assert!(seen.contains("\"pot\":123") && seen.contains("\"slot\":0"), "{seen}");
}

#[test]
fn a_worker_heartbeat_takes_the_heads_slot() {
    // Each split-fleet worker runs one bot, so every heartbeat says slot 0; the head must show
    // them under its own slots, or every bot tab selects the first bot and the per-slot metrics
    // cache mixes the bots' figures (2026-09-23, first split-fleet run).
    let shared = Shared::for_test("remote-slots", &["A", "B"]);
    for name in ["A", "B"] {
        let worker = BotLive { name: name.into(), slot: 0, mode: "playing".into(), ..Default::default() };
        shared.store.put_kv(&crate::live::heartbeat_key(name), &crate::live::wrap_heartbeat(&worker, now_secs()).to_string()).unwrap();
    }
    let placeholder = shared.bots[1].read().clone();
    let (remote, _) = remote_bot(&shared, &placeholder).expect("fresh heartbeat");
    assert_eq!((remote.name.as_str(), remote.slot), ("B", 1));
}

#[test]
fn compute_reports_decision_time_against_the_45_second_deadline() {
    assert_eq!(latency_summary(vec![])["n"], 0);
    let v = latency_summary((1..=100).map(f64::from).collect());
    assert_eq!(
        (v["p50"].as_f64(), v["p95"].as_f64(), v["p99"].as_f64(), v["max"].as_f64()),
        (Some(51.0), Some(96.0), Some(100.0), Some(100.0))
    );
    let shared = Shared::for_test("compute", &["A"]);
    for (street, ms) in [("flop", 120.0), ("river", 4.0), ("flop", 180.0)] {
        shared.store.insert_decision("A", "h", street, "call", None, Some(0.5), 100, 20, ms, "{}").unwrap();
    }
    let c = compute_value(&shared);
    assert_eq!(c["decisions"]["n"], 3);
    assert_eq!(c["by_street"]["flop"]["max"], 180.0);
    assert_eq!(c["by_street"]["turn"]["n"], 0);
    assert!((c["max_share_of_deadline"].as_f64().unwrap() - 180.0 / 45_000.0).abs() < 1e-12);
}

#[test]
fn the_snapshot_reports_the_retry_queue_and_the_keepalive_hold() {
    let shared = Shared::for_test("ops-state", &["A"]);
    let v = snapshot(&shared);
    assert_eq!((v["ops"]["unstored_hands"].as_u64(), v["ops"]["hold_until"].as_str()), (Some(0), None));
    std::fs::write(shared.config.artifacts.join("hold-until"), "forever\n").unwrap();
    let row = sv10_store::store::HandRow {
        bot: "A".into(),
        hand_id: "h".into(),
        table_id: "t".into(),
        ended_at: "2026-09-23T00:00:00Z".into(),
        hero_seat: Some(1),
        hole: String::new(),
        board: String::new(),
        pot: 0,
        net: Some(0),
        winners: String::new(),
        summary: "{}".into(),
        showdown: false,
    };
    shared.unstored_hands.lock().push((row, 3));
    let v = snapshot(&shared);
    assert_eq!(v["ops"]["unstored_hands"], 1);
    assert_eq!(v["ops"]["hold_until"], "forever");
    std::fs::write(shared.config.artifacts.join("hold-until"), "1790000000\n").unwrap();
    assert_eq!(snapshot(&shared)["ops"]["hold_until"], 1_790_000_000);
    assert_eq!(snapshot(&shared)["ops"]["keepalive_last"], Value::Null, "the keepalive never had to act");
    std::fs::create_dir_all(shared.config.artifacts.join("logs")).unwrap();
    std::fs::write(
        shared.config.artifacts.join("logs").join("keepalive.log"),
        "2026-09-23T01:00:00Z fleet down with no hold in force (hold-until: none); restarting svanbot10.service\n\n",
    )
    .unwrap();
    assert!(snapshot(&shared)["ops"]["keepalive_last"].as_str().unwrap().starts_with("2026-09-23T01:00:00Z fleet down"));
}

#[test]
fn dashboard_rejects_a_legacy_active_response_model() {
    let stored = crate::StoredNet {
        net: sv10_core::nn::Mlp::new(&[sv10_core::features::N_FEATURES, 48, 24, 3], 7),
        active: true,
        training_contract: String::new(),
        paired_poker_approved: false,
        val_loss: 0.4,
        baseline_loss: 0.5,
        train_samples: 2_000,
        val_samples: 300,
        trained_at: 1.0,
    };
    assert_eq!(response_model_labels(&stored), ("rejected", "stored artifact rejected by current live gates", "rejected by live gates"));
}

/// 0326: a failed results read is reported, not defaulted — and the figures a playing bot showed
/// are kept rather than replaced by zeros.
#[test]
fn a_failed_results_read_keeps_the_last_good_figures_and_says_so() {
    let rows = vec![(Some(300), None, false, "2026-09-27T00:00:00Z".to_string())];
    let (good, err) = figures(rows, None, &None, 20.0, None);
    assert!(err.is_none());
    assert_eq!(good["hands"], json!(1));
    assert_eq!(good["net_chips"], json!(300));

    let (v, err) = figures(vec![], Some("store unreadable (bot results): boom".into()), &None, 20.0, Some(good.clone()));
    assert_eq!(v["hands"], good["hands"], "the last good figures stand");
    assert_eq!(v["net_chips"], json!(300));
    assert_eq!(err.as_deref(), Some("store unreadable (bot results): boom"), "and the read failure is carried out");

    // Nothing good to keep: the panel's shape is there for a renderer, but the payload says so.
    let (v, err) = figures(vec![], Some("boom".into()), &None, 20.0, None);
    assert_eq!(v["hands"], json!(0));
    assert_eq!(err.as_deref(), Some("boom"));
    assert!(v.get("series").is_some() && v.get("all_time").is_some() && v.get("season").is_some(), "{v}");
}

/// 0326: `stale` and `error` are always in the payload, so "0 hands" and "unreadable" differ.
#[test]
fn a_snapshot_says_whether_its_figures_are_this_seconds_or_the_last_good_ones() {
    let b = BotLive { name: "A".into(), big_blind: 20, ..Default::default() };
    let fresh = served(json!({"hands": 12}), &b, None);
    assert_eq!(fresh["stale"], json!(false));
    assert!(fresh["error"].is_null());
    let stale = served(json!({"hands": 12}), &b, Some("store unreadable (bot results): boom"));
    assert_eq!(stale["stale"], json!(true));
    assert_eq!(stale["error"], json!("store unreadable (bot results): boom"));
    assert_eq!(stale["hands"], json!(12), "the figures themselves are the last good ones");
}

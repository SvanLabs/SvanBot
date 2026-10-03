//! Tests for insight reports and their season scope.
use super::leaderboard::{HistoricalStanding, LeaderboardHistory, LeaderboardState, enrich_leaderboard, transition_leaderboard};
use super::*;
use std::collections::HashMap;

fn standings() -> Vec<Value> {
    vec![
        json!({"rank":1,"bot_name":"leader","score":2_000,"hands_played":100,"win_rate":3.0,"pro":true}),
        json!({"rank":2,"bot_name":"alpha","score":1_800,"hands_played":100,"win_rate":2.0,"pro":false}),
        json!({"rank":4,"bot_name":"fourth","score":1_500,"hands_played":100,"win_rate":1.0,"pro":false}),
        json!({"rank":7,"bot_name":"ours","score":1_100,"hands_played":100,"win_rate":0.5,"pro":false}),
    ]
}

#[test]
fn leaderboard_derives_identity_deltas_actual_rank_gaps_and_velocity() {
    let previous = LeaderboardHistory {
        updated: 1_000.0,
        season: Some("12".into()),
        standings: HashMap::from([
            ("ours".into(), HistoricalStanding { rank: Some(5), score: Some(1_000), hands: Some(80) }),
            ("alpha".into(), HistoricalStanding { rank: Some(3), score: Some(1_700), hands: Some(90) }),
        ]),
    };
    let entries = standings();
    let value = enrich_leaderboard(&entries, &["ours".into()], json!({"season_number":12}), Some(&previous), 4_600.0);
    let rows = value["entries"].as_array().unwrap();
    let ours = rows.iter().find(|row| row["name"] == "ours").unwrap();
    assert_eq!(ours["rank_delta"], -2);
    assert_eq!(ours["score_delta"], 100);
    assert_eq!(ours["gap_to_first"], 900);
    assert_eq!(ours["gap_to_next"], 400);
    assert_eq!(ours["gap_to_four"], 400);
    assert_eq!(ours["score_velocity_per_hour"], 100.0);
    assert_eq!(ours["hands_delta"], 20);
    assert_eq!(ours["hands_velocity_per_hour"], 20.0);
    assert_eq!(rows[1]["rank_delta"], 1);
    assert_eq!(rows[1]["gap_to_next"], 200);
    assert_eq!(rows[2]["gap_to_four"], 0);
    assert_eq!(value["stale"], false);
    assert!(value["error"].is_null());
}

#[test]
fn leaderboard_keeps_unmeasurable_fields_null() {
    let previous = LeaderboardHistory { updated: 1_000.0, season: Some("12".into()), standings: HashMap::new() };
    let entries = vec![
        json!({"rank":1,"bot_name":"leader","score":2_000}),
        json!({"rank":"four","bot_name":"broken","score":"many","hands_played":"lots"}),
    ];
    let value = enrich_leaderboard(&entries, &[], json!({"season_number":12}), Some(&previous), 1_299.0);
    let broken = &value["entries"][1];
    for field in [
        "rank",
        "score",
        "hands",
        "rank_delta",
        "gap_to_first",
        "gap_to_next",
        "gap_to_four",
        "score_delta",
        "score_velocity_per_hour",
        "hands_delta",
        "hands_velocity_per_hour",
    ] {
        assert!(broken[field].is_null(), "{field} should be null");
    }
    assert!(value["entries"][0]["score_velocity_per_hour"].is_null());
}

#[test]
fn leaderboard_cache_transitions_preserve_last_success_on_failure() {
    let empty = LeaderboardState::empty();
    let failed =
        transition_leaderboard(&empty, Err("request contained secret-token and much more detail than operators need".into()), &[], 100.0);
    assert_eq!(failed.value["entries"], json!([]));
    assert!(failed.value["updated"].is_null());
    assert_eq!(failed.value["stale"], true);
    assert!(failed.value["error"].as_str().unwrap().len() <= 96);

    let fresh = transition_leaderboard(&empty, Ok((standings(), json!({"season_number":12}))), &["ours".into()], 200.0);
    assert_eq!(fresh.value["updated"], 200.0);
    assert_eq!(fresh.value["stale"], false);
    let stale = transition_leaderboard(&fresh, Err("upstream unavailable".into()), &["ours".into()], 500.0);
    assert_eq!(stale.value["entries"], fresh.value["entries"]);
    assert_eq!(stale.value["season"], fresh.value["season"]);
    assert_eq!(stale.value["updated"], 200.0);
    assert_eq!(stale.value["stale"], true);
    assert_eq!(stale.history.as_ref().unwrap().updated, 200.0);
}

#[test]
fn leaderboard_rate_frontier_survives_short_polls_and_resets_on_season_change() {
    let empty = LeaderboardState::empty();
    let first = transition_leaderboard(&empty, Ok((standings(), json!({"season_number":12}))), &["ours".into()], 1_000.0);
    let mut changed = standings();
    changed[3]["score"] = json!(1_160);
    changed[3]["hands_played"] = json!(112);

    let short = transition_leaderboard(&first, Ok((changed.clone(), json!({"season_number":12}))), &["ours".into()], 1_060.0);
    assert_eq!(short.history.as_ref().unwrap().updated, 1_000.0, "one-minute polls must retain the rate frontier");
    assert!(short.value["entries"][3]["hands_velocity_per_hour"].is_null());

    let measurable = transition_leaderboard(&short, Ok((changed.clone(), json!({"season_number":12}))), &["ours".into()], 1_300.0);
    assert_eq!(measurable.value["entries"][3]["score_velocity_per_hour"], 720.0);
    assert_eq!(measurable.value["entries"][3]["hands_velocity_per_hour"], 144.0);
    assert_eq!(measurable.history.as_ref().unwrap().updated, 1_300.0);

    let rollover = transition_leaderboard(&measurable, Ok((standings(), json!({"season_number":13}))), &["ours".into()], 1_360.0);
    assert!(rollover.value["entries"][3]["score_delta"].is_null());
    assert!(rollover.value["entries"][3]["hands_velocity_per_hour"].is_null());
    assert_eq!(rollover.history.as_ref().unwrap().season.as_deref(), Some("13"));
    assert_eq!(rollover.history.as_ref().unwrap().updated, 1_360.0);
}

fn hand(bot: &str, id: &str, hole: &str, board: &str, pot: i64, net: i64, showdown: bool) -> sv10_store::store::HandRow {
    at("2026-09-17T00:00:00Z", hand_at(bot, id, hole, board, pot, net, showdown))
}

fn at(ended_at: &str, mut row: sv10_store::store::HandRow) -> sv10_store::store::HandRow {
    row.ended_at = ended_at.into();
    row
}

fn hand_at(bot: &str, id: &str, hole: &str, board: &str, pot: i64, net: i64, showdown: bool) -> sv10_store::store::HandRow {
    sv10_store::store::HandRow {
        bot: bot.into(),
        hand_id: id.into(),
        table_id: "t".into(),
        ended_at: "2026-09-17T00:00:00Z".into(),
        hero_seat: None,
        hole: hole.into(),
        board: board.into(),
        pot,
        net: Some(net),
        winners: bot.into(),
        summary: String::new(),
        showdown,
    }
}

#[test]
fn highlights_reports_slams_royalty_and_locked_milestones() {
    let shared = Shared::for_test("highlights", &["b"]);
    shared.store.insert_hand(&hand("b", "h1", "AhKd", "AcAsAd2s3d", 9_000, 8_000, true)).unwrap();
    shared.store.insert_hand(&hand("b", "h2", "7c2d", "Kh9s4dQc5h", 400, 200, false)).unwrap();
    let v = highlights_blocking(&shared).unwrap().0;
    assert_eq!(v["fleet_hands"], 2);
    assert_eq!(v["slams"].as_array().unwrap().len(), 1);
    assert_eq!(v["slams"][0]["hand_id"], "h1");
    let ms: HashMap<String, bool> = v["milestones"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["id"].as_str().unwrap().to_string(), m["unlocked"].as_bool().unwrap()))
        .collect();
    assert!(ms["royalty"], "quads win unlocks Royalty");
    assert!(ms["monster"]);
    assert!(!ms["million-club"]);
    assert!(!ms["unstoppable"]);
}

/// Season 13 opened 2026-09-20 11:46:23 UTC; season 12's run must not be plotted into it.
fn season_13() -> crate::season::CurrentSeason {
    crate::season::CurrentSeason::from_current(&json!({
        "season_number": 13, "season_id": "ac9f1eb2", "start_date": "2026-09-20T11:46:23.998128+00:00"
    }))
    .unwrap()
}

/// One season-12 hand and two season-13 hands for bot `b`.
fn across_the_boundary(shared: &Shared) {
    for (ts, id, net) in [
        ("2026-09-14T10:14:00+00:00", "s12", 900_000i64),
        ("2026-09-20T13:46:00+00:00", "s13a", 8_000),
        ("2026-09-20T14:58:00+00:00", "s13b", 5_000),
    ] {
        shared.store.insert_hand(&at(ts, hand_at("b", id, "AhKd", "2c3c4c5c7d", 400, net, false))).unwrap();
    }
}

#[test]
fn the_fleet_race_runs_from_this_seasons_first_hand_and_keeps_the_lifetime_total() {
    let shared = Shared::for_test("fleet-season", &["b"]);
    across_the_boundary(&shared);

    let merged = fleet_blocking(&shared).unwrap().0;
    assert_eq!(merged["season"]["scoped"], false, "with no known boundary nothing is dropped");
    assert_eq!(merged["bots"][0]["hands"], 3);
    assert_eq!(merged["bots"][0]["total"], 913_000);

    *shared.current_season.write() = Some(season_13());
    let v = fleet_blocking(&shared).unwrap().0;
    assert_eq!(v["season"]["scoped"], true);
    assert_eq!(v["season"]["number"], 13);
    let bot = &v["bots"][0];
    assert_eq!(bot["hands"], 2, "season 12's hands are a different contest");
    assert_eq!(bot["total"], 13_000);
    assert_eq!(bot["all_time"], json!({"hands": 3, "total": 913_000}), "the lifetime run is kept, labelled");
    let points = bot["points"].as_array().unwrap();
    assert_eq!(points.len(), 2);
    assert_eq!(points[0]["hand"], 1, "hand 1 is this season's first hand");
    assert_eq!(points[0]["ts"], parse_ts("2026-09-20T13:46:00+00:00"));
    assert_eq!(points[0]["total"], 8_000);
    assert_eq!(points[1]["total"], 13_000);
}

#[test]
fn range_explorer_equities_carry_a_standard_error_under_an_eighth_of_a_point() {
    assert!(equity_se(0.5, RANGE_EXPLORER_SAMPLES) < 0.00115, "worst case at 50%");
    assert_eq!(equity_se(0.0, RANGE_EXPLORER_SAMPLES), 0.0);
    assert!((equity_se(0.5, 3_000) - 0.00913).abs() < 1e-4, "the old 3,000-sample error");
}

#[test]
fn a_renamed_bot_keeps_one_season_record_in_every_panel() {
    // 2026-09-23: SvanBotV7 was renamed SvanBotV10 (server kept its #1 score); hands before the
    // rename are stored as SvanBotV7.
    let shared = Shared::for_test("renamed", &["SvanBotV10"]);
    for (bot, id, ts, net) in
        [("SvanBotV7", "old1", "2026-09-21T10:00:00+00:00", 250_000i64), ("SvanBotV10", "new1", "2026-09-22T23:00:00+00:00", -2_500)]
    {
        shared.store.insert_hand(&at(ts, hand_at(bot, id, "AhKd", "2c3c4c5c7d", 400, net, false))).unwrap();
    }
    *shared.current_season.write() = Some(season_13());
    let alone = fleet_blocking(&shared).unwrap().0;
    assert_eq!(alone["bots"][0]["hands"], 1, "without the alias only the new name counts");
    shared.aliases.write().insert("SvanBotV10".into(), vec!["SvanBotV10".into(), "SvanBotV7".into()]);
    let f = fleet_blocking(&shared).unwrap().0;
    assert_eq!(
        (f["bots"][0]["name"].as_str(), f["bots"][0]["hands"].as_i64(), f["bots"][0]["total"].as_i64()),
        (Some("SvanBotV10"), Some(2), Some(247_500))
    );
    let h = highlights_blocking(&shared).unwrap().0;
    assert_eq!((h["fleet_total"].as_i64(), h["fleet_hands"].as_i64()), (Some(247_500), Some(2)));
    let mut bot = shared.bots[0].read().clone();
    bot.slot = 90_155;
    let m = metrics(&shared, &bot);
    assert_eq!((m["net_chips"].as_i64(), m["hands"].as_i64()), (Some(247_500), Some(2)));
}

#[test]
fn calibration_shows_the_stored_fits_next_to_the_shifts_in_use() {
    let shared = Shared::for_test("calibration-fits", &["b"]);
    let empty = calibration_value(&shared).unwrap();
    assert_eq!(empty["fold"], Value::Null, "no fit stored yet");
    assert_eq!(empty["river_jam"], Value::Null);
    assert_eq!(empty["live"]["river_jam_call_shift"], 0.0);
    let fold = crate::foldcal::FoldCalibration { shift: [0.24, 0.0, -0.62], ..Default::default() };
    shared.store.put_kv(crate::foldcal::FOLD_CAL_KEY, &serde_json::to_string(&fold).unwrap()).unwrap();
    let jam = crate::raisewar::RiverJamFit { n: 882, train_shift: 0.09, held_out_saved: 39.0, held_out_lower: -75.0, ..Default::default() };
    shared.store.put_kv(crate::raisewar::RIVER_JAM_KEY, &serde_json::to_string(&jam).unwrap()).unwrap();
    shared.params.write().fold_logit_shift = [0.24, 0.0, -0.62];
    let v = calibration_value(&shared).unwrap();
    assert_eq!(v["fold"]["shift"], json!([0.24, 0.0, -0.62]));
    assert_eq!(v["river_jam"]["n"], 882);
    assert_eq!(v["river_jam"]["active"], false);
    assert_eq!(v["live"]["fold_logit_shift"], json!([0.24, 0.0, -0.62]), "what the policy actually uses");
}

#[test]
fn a_bots_net_winnings_tile_reads_this_season_and_keeps_the_lifetime_total() {
    let shared = Shared::for_test("metrics-season", &["b"]);
    across_the_boundary(&shared);
    *shared.current_season.write() = Some(season_13());
    // A slot no other test uses: the metrics cache is process-wide and keyed by slot.
    let mut bot = shared.bots[0].read().clone();
    bot.slot = 90_154;
    let v = metrics(&shared, &bot);
    assert_eq!(v["season"]["number"], 13);
    assert_eq!((v["net_chips"].as_i64(), v["hands"].as_i64()), (Some(13_000), Some(2)), "season 12's 900k is not this season's winnings");
    assert_eq!((v["all_time"]["net_chips"].as_i64(), v["all_time"]["hands"].as_i64()), (Some(913_000), Some(3)));
    let series = v["series"].as_array().unwrap();
    assert_eq!(series.last().unwrap()["total"], 13_000, "the curve starts at this season's first hand");
}

#[test]
fn highlights_present_this_season_while_the_badges_stay_lifetime_achievements() {
    let shared = Shared::for_test("highlights-season", &["b"]);
    across_the_boundary(&shared);
    *shared.current_season.write() = Some(season_13());

    let v = highlights_blocking(&shared).unwrap().0;
    assert_eq!(v["season"]["number"], 13);
    assert_eq!((v["fleet_total"].as_i64(), v["fleet_hands"].as_i64()), (Some(13_000), Some(2)));
    let wins: Vec<&str> = v["biggest_wins"].as_array().unwrap().iter().map(|h| h["hand_id"].as_str().unwrap()).collect();
    assert_eq!(wins, ["s13a", "s13b"], "season 12's 900k pot is not one of this season's wins");
    assert_eq!(v["best_streak"]["length"], 2, "this season's streak");
    let ms: HashMap<String, bool> = v["milestones"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["id"].as_str().unwrap().to_string(), m["unlocked"].as_bool().unwrap()))
        .collect();
    // 913,000 lifetime against 13,000 this season: the badge stays earned across the rollover.
    assert!(ms["hundred-k"], "an achievement already earned survives the season rollover");
    assert!(ms["deep-stack"]);
    assert!(!ms["million-club"], "badges still read the real lifetime total, not an inflated one");
}

#[test]
fn the_leak_finder_reads_every_season_but_reports_this_ones_result_separately() {
    let shared = Shared::for_test("analysis-season", &["b"]);
    let summary = HandSummary {
        players: vec![(0, "b".into())],
        button: 0,
        bb: 20,
        history: Vec::new(),
        board: Vec::new(),
        shown: Vec::new(),
        stacks: Vec::new(),
    };
    for (ts, id, net) in [("2026-09-14T10:14:00+00:00", "s12", 900_000i64), ("2026-09-20T13:46:00+00:00", "s13a", 8_000)] {
        let mut row = hand_at("b", id, "AhKd", "2c3c4c5c7d", 400, net, false);
        row.ended_at = ts.into();
        row.summary = serde_json::to_string(&summary).unwrap();
        shared.store.insert_hand(&row).unwrap();
    }
    let season = season_13();
    let report = crate::analysis::report(&shared.store, &["b".to_string()], &HashMap::new(), None, Some(&season)).unwrap();
    assert_eq!(report["hands"], 2, "leaks are learning: they read every season");
    assert_eq!(report["overall"]["chips"], 908_000);
    assert_eq!(report["season"]["scoped"], true);
    assert_eq!(report["season"]["number"], 13);
    assert_eq!(report["season"]["hands"], 1);
    assert_eq!(report["season"]["chips"], 8_000);

    let unscoped = crate::analysis::report(&shared.store, &["b".to_string()], &HashMap::new(), None, None).unwrap();
    assert_eq!(unscoped["season"]["scoped"], false);
    assert_eq!(unscoped["season"]["chips"], 908_000, "no boundary means nothing is dropped, and the panel says so");
}

#[test]
fn the_leak_finder_normalizes_each_hand_by_its_own_recorded_blind() {
    let shared = Shared::for_test("analysis-mixed-blind", &["b"]);
    // +10 chips at BB 10 and +20 chips at BB 20 are both +1 bb/hand.
    for (id, net, bb) in [("m1", 10i64, 10i64), ("m2", 20, 20)] {
        let mut row = hand_at("b", id, "AhKd", "2c3c4c5c7d", 400, net, false);
        let summary = HandSummary {
            players: vec![(0, "b".into())],
            button: 0,
            bb,
            history: Vec::new(),
            board: Vec::new(),
            shown: Vec::new(),
            stacks: Vec::new(),
        };
        row.summary = serde_json::to_string(&summary).unwrap();
        shared.store.insert_hand(&row).unwrap();
    }
    // With no blind argument left to vary, one report proves the mixed-blind pricing: the
    // per-hand normalization is structural, not a divisor the caller supplies.
    let report = crate::analysis::report(&shared.store, &["b".to_string()], &HashMap::new(), None, None).unwrap();
    assert_eq!(report["overall"]["bb100"], 100.0, "mixed blinds average +100 bb/100");
    assert_eq!(report["overall"]["priced_hands"], 2);
    assert_eq!(report["season"]["bb100"], 100.0, "the unscoped season matches overall");
    assert_eq!(report["trend"][0]["bb100"], 100.0, "the trend block prices its own hands");
}

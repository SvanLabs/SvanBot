use super::*;
use sv10_store::store::HandRow;

async fn body(response: Response) -> Value {
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn fixture(tag: &str) -> Arc<Shared> {
    use sv10_core::engine::{ActionKind, ActionRecord, Street};
    let s = Shared::for_test(tag, &["New"]);
    s.aliases.write().insert("New".into(), vec!["New".into(), "Old".into()]);
    for (bot, id, ts, net) in [("Old", "old", "2026-09-29T00:00:00Z", 40), ("New", "new", "2026-09-30T00:00:00Z", -20)] {
        let summary = HandSummary {
            players: vec![(0, bot.into()), (1, "Villain".into())],
            button: 0,
            bb: 20,
            history: vec![ActionRecord {
                seat: 0,
                street: Street::Preflop,
                kind: ActionKind::Fold,
                to: 0,
                pot_before: 30,
                to_call_before: 20,
                bet_before: 0,
                full_raise: false,
                think_ms: None,
                street_open: false,
            }],
            board: vec![],
            shown: vec![],
            stacks: [(0, 2000), (1, 2000)].into_iter().collect(),
        };
        s.store
            .insert_hand(&HandRow {
                bot: bot.into(),
                hand_id: id.into(),
                table_id: "t".into(),
                ended_at: ts.into(),
                hero_seat: Some(0),
                hole: "AhKd".into(),
                board: String::new(),
                pot: 60,
                net: Some(net),
                winners: if net > 0 { bot.into() } else { "Villain".into() },
                summary: serde_json::to_string(&summary).unwrap(),
                showdown: false,
            })
            .unwrap();
        let detail = json!({"version":format!("v-{bot}"), "reason":"recorded decision"}).to_string();
        s.store.insert_decision(bot, id, "preflop", "fold", None, None, 30, 20, 1.0, &detail).unwrap();
    }
    s
}

#[tokio::test]
async fn renamed_seat_hand_list_includes_old_hands_and_their_versions() {
    let s = fixture("alias-hand-list");
    let list = body(hands(State(s), Path(0)).await).await;
    assert_eq!(list.as_array().unwrap().len(), 2, "{list}");
    assert_eq!(list[0]["hand_id"], "new");
    assert_eq!(list[1]["hand_id"], "old");
    assert_eq!(list[1]["version"], "v-Old");
}

#[tokio::test]
async fn renamed_seat_replay_reads_old_hand_and_old_decisions() {
    let s = fixture("alias-replay");
    let replay = body(replay(State(s), Path((0, "old".into()))).await).await;
    assert!(replay.as_array().unwrap().iter().any(|v| v["type"] == "decision" && v["data"]["reason"] == "recorded decision"));
}

#[tokio::test]
async fn renamed_seat_opponent_totals_include_old_wins() {
    let s = fixture("alias-opponent");
    let detail = body(opponent_detail(State(s), Path((0, "Villain".into()))).await).await;
    assert_eq!(detail["head_to_head_hands"], 2);
    assert_eq!(detail["hero_wins"], 1);
    assert_eq!(detail["won_from"], 40);
    assert_eq!(detail["lost_to"], 20);
    assert_eq!(detail["table_net"], 20);
}

#[tokio::test]
async fn renamed_seat_own_cards_include_all_names_and_style_samples() {
    let s = fixture("alias-player");
    for name in ["New", "Old"] {
        let card = body(player_card(State(s.clone()), Path(name.into())).await).await;
        assert_eq!(card["ours"], true, "{name}");
        assert_eq!(card["hands_observed"], 2.0, "{name}: {card}");
        assert_eq!(card["vs_us"]["hands"], 2, "{name}");
        assert_eq!(card["vs_us"]["won_pots"], 1, "{name}");
        assert_eq!(card["vs_us"]["by_bot"].as_array().unwrap().len(), 1);
    }
}

#[test]
fn renamed_seat_recent_limit_applies_to_the_combined_history() {
    let s = fixture("alias-limit");
    let rows = seat_history::recent_seat_hands(&s, "New", 1, true).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].hand_id, "new");
    let rows = seat_history::recent_seat_hands(&s, "Old", 2, false).unwrap();
    assert_eq!(rows.iter().map(|r| r.hand_id.as_str()).collect::<Vec<_>>(), ["new", "old"]);
}

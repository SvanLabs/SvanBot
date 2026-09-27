//! Hand history and replays.

use super::*;

pub(super) async fn hands(State(s): State<Arc<Shared>>, Path(slot): Path<usize>) -> Response {
    off_runtime(move || hands_blocking(&s, slot)).await.into_response()
}

fn hands_blocking(s: &Shared, slot: usize) -> Result<Response, ApiError> {
    let name = bot_name(s, slot)?;
    let rows = store_read("recent hands", s.store.recent_hands_light(&name, 50))?;
    let ids: Vec<String> = rows.iter().map(|r| r.hand_id.clone()).collect();
    let versions = store_read("decision versions", s.store.decision_versions(&name, &ids))?;
    let big_blind = s.bots.get(slot).map(|rl| rl.read().big_blind).filter(|bb| *bb > 0).unwrap_or(s.big_blind() as i64);
    let out: Vec<Value> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| json!({"id": i, "hand_id": r.hand_id, "ts": parse_ts(&r.ended_at), "hole": split_cards(&r.hole), "board": split_cards(&r.board), "net": r.net, "big_blind": big_blind, "version": versions.get(&r.hand_id).cloned().unwrap_or_else(|| "not recorded".into())}))
        .collect();
    Ok(Json(out).into_response())
}

pub(super) async fn replay(State(s): State<Arc<Shared>>, Path((slot, hand_id)): Path<(usize, String)>) -> Response {
    off_runtime(move || replay_blocking(&s, slot, hand_id)).await.into_response()
}

fn replay_blocking(s: &Shared, slot: usize, hand_id: String) -> Result<Response, ApiError> {
    let name = bot_name(s, slot)?;
    let Some(row) = store_read("hand", s.store.hand(&name, &hand_id))? else {
        return Err((StatusCode::NOT_FOUND, Json(json!({"detail": "Hand not found"}))).into_response().into());
    };
    let ts = parse_ts(&row.ended_at);
    let summary = serde_json::from_str::<HandSummary>(&row.summary).map_err(|e| server_error("stored hand summary unreadable", e))?;
    let names: HashMap<usize, String> = summary.players.iter().cloned().collect();
    let mut events = vec![
        json!({"type": "hand_start", "ts": ts, "data": {"dealer_seat": summary.button, "hole_cards": split_cards(&row.hole), "players": summary.players.iter().map(|(s, n)| json!({"seat": s, "name": n})).collect::<Vec<_>>()}}),
    ];
    let decisions = store_read("hand decisions", s.store.decisions_for_hand(&name, &hand_id))?;
    let mut street = sv10_core::engine::Street::Preflop;
    let mut dec_iter = decisions.iter();
    for rec in &summary.history {
        if rec.street != street {
            street = rec.street;
            let n = street.board_len().min(summary.board.len());
            events.push(json!({"type": "community_cards", "ts": ts, "data": {"street": street.name(), "cards": summary.board[..n].iter().map(|c| c.to_string()).collect::<Vec<_>>()}}));
        }
        let is_hero = row.hero_seat == Some(rec.seat as i64);
        if is_hero && let Some(d) = dec_iter.next() {
            events.push(json!({"type": "decision", "ts": parse_ts(d["ts"].as_str().unwrap_or("")), "data": {"name": name, "action": d["action"], "amount": d["amount"], "equity": d["equity"], "reason": d["detail"]["reason"]}}));
        }
        let action = match rec.kind {
            sv10_core::engine::ActionKind::Fold => "fold",
            sv10_core::engine::ActionKind::Check => "check",
            sv10_core::engine::ActionKind::Call => "call",
            sv10_core::engine::ActionKind::Raise => "raise",
            sv10_core::engine::ActionKind::AllIn => "all_in",
        };
        events.push(json!({"type": "player_action", "ts": ts, "data": {"seat": rec.seat, "name": names.get(&rec.seat), "action": action, "amount": if rec.to > 0 { json!(rec.to) } else { Value::Null }, "street": rec.street.name(), "pot": rec.pot_before}}));
    }
    if summary.board.len() > street.board_len() {
        events.push(json!({"type": "community_cards", "ts": ts, "data": {"street": "river", "cards": summary.board.iter().map(|c| c.to_string()).collect::<Vec<_>>()}}));
    }
    events.push(json!({"type": "hand_result", "ts": ts, "data": {"name": row.winners, "amount": row.pot, "net": row.net, "board": split_cards(&row.board), "shown": summary.shown.iter().map(|(s, c)| json!({"seat": s, "name": names.get(s), "cards": [c[0].to_string(), c[1].to_string()]})).collect::<Vec<_>>()}}));
    Ok(Json(events).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(r: Result<Response, ApiError>) -> StatusCode {
        r.into_response().status()
    }

    #[test]
    fn hand_list_answers_ok_404_and_500() {
        let s = Shared::for_test("api-hands", &["A"]);
        assert_eq!(status(hands_blocking(&s, 0)), StatusCode::OK);
        assert_eq!(status(hands_blocking(&s, 9)), StatusCode::NOT_FOUND, "unknown slot");
        assert_eq!(status(replay_blocking(&s, 0, "missing".into())), StatusCode::NOT_FOUND, "unknown hand");

        // A store that cannot be read answers 500, never an empty list (LESSONS 24).
        let conn = rusqlite::Connection::open(s.config.artifacts.join("svanbot10.db")).unwrap();
        conn.execute_batch("ALTER TABLE hands RENAME TO hands_hidden").unwrap();
        assert_eq!(status(hands_blocking(&s, 0)), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(status(replay_blocking(&s, 0, "missing".into())), StatusCode::INTERNAL_SERVER_ERROR);
    }
}

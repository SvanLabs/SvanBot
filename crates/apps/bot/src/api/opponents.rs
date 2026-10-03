//! Opponent profiles: style classification, stat estimates and the opponent endpoints.

use super::*;

pub(super) fn estimate(c: &Counter) -> Value {
    if c.opp <= 0.0 {
        return Value::Null;
    }
    let p = (c.hit / c.opp) as f64;
    let se = (p * (1.0 - p) / c.opp as f64).sqrt();
    json!({"value": p, "samples": c.opp, "count": c.hit, "standard_error": se, "lower": (p - 1.96 * se).max(0.0), "upper": (p + 1.96 * se).min(1.0)})
}

/// The classifier lives in [`crate::style`], shared with the monitor: one function, so the scout
/// view and the alert stream never call one player two things (0246).
pub(super) use crate::style::{aggression_counter, style_of, sum3};

pub(super) fn opponent_json(name: &str, st: &PlayerStats, rep: Option<&crate::reputation::Reputation>) -> Value {
    let (style, mut advice) = style_of(st);
    let aggression = aggression_counter(st);
    let style = match rep {
        Some(r) if r.top10 > 0 => {
            format!("{style} · best #{} (S{})", r.best_rank, r.finishes.iter().min_by_key(|f| f.rank).map(|f| f.season).unwrap_or(0))
        }
        Some(r) if r.seasons > 1 => format!("{style} · {} seasons", r.seasons),
        _ => style,
    };
    if let Some(r) = rep {
        advice.push_str(&format!(
            " History: {} season(s), best rank #{}, {} top-10 finish(es), {} lifetime hands, strength {:.0}%.",
            r.seasons,
            r.best_rank,
            r.top10,
            r.lifetime_hands,
            r.strength * 100.0
        ));
    }
    json!({
        "name": name,
        "style": style,
        "advice": advice,
        "evidence_hands": st.hands,
        "vpip": estimate(&st.vpip),
        "pfr": estimate(&st.pfr),
        "aggression": estimate(&aggression),
        "fold_to_bet": estimate(&sum3(&st.fold_vs_bet)),
        "reputation": rep,
        "fold_to_raise": estimate(&st.fold_to_3bet),
        "wtsd": estimate(&st.wtsd),
        "won_at_showdown": estimate(&st.won_showdown),
        "three_bet": estimate(&st.three_bet),
        "cbet": estimate(&st.cbet),
        "fold_to_cbet": estimate(&st.fold_to_cbet),
        "by_position": {
            "early": {"vpip": estimate(&st.vpip_pos[0]), "pfr": estimate(&st.pfr_pos[0])},
            "late": {"vpip": estimate(&st.vpip_pos[1]), "pfr": estimate(&st.pfr_pos[1])},
            "blinds": {"vpip": estimate(&st.vpip_pos[2]), "pfr": estimate(&st.pfr_pos[2])},
        },
    })
}

pub(super) fn best_five(hole: &[Card], board: &[Card]) -> Option<(String, Vec<String>)> {
    if board.len() < 3 {
        return None;
    }
    let all: Vec<Card> = hole.iter().chain(board.iter()).copied().collect();
    let v = eval(mask_of(&all));
    let n = all.len();
    let mut best: Option<Vec<Card>> = None;
    let idx: Vec<usize> = (0..n).collect();
    let mut choose = |combo: &[usize]| {
        let five: Vec<Card> = combo.iter().map(|&i| all[i]).collect();
        if best.is_none() && eval(mask_of(&five)) == v {
            best = Some(five);
        }
    };
    for a in 0..n {
        for b in a + 1..n {
            for c in b + 1..n {
                for d in c + 1..n {
                    for e in d + 1..n {
                        choose(&[idx[a], idx[b], idx[c], idx[d], idx[e]]);
                    }
                }
            }
        }
    }
    let name = CATEGORY_NAMES[category(v) as usize];
    let mut title = name.to_string();
    if let Some(f) = title.get_mut(0..1) {
        f.make_ascii_uppercase();
    }
    Some((title, best?.iter().map(|c| c.to_string()).collect()))
}

pub(super) async fn opponents(State(s): State<Arc<Shared>>, Path(slot): Path<usize>) -> Response {
    let table: Vec<String> = match s.bots.get(slot) {
        Some(b) => b.read().seats.iter().map(|x| x.name.clone()).collect(),
        None => return (StatusCode::NOT_FOUND, Json(json!({"detail": "Unknown bot slot"}))).into_response(),
    };
    let own: Vec<String> = s.bots.iter().map(|b| b.read().name.clone()).collect();
    let models = s.models.read();
    let mut list: Vec<(&String, &PlayerStats)> = models.players.iter().filter(|(n, _)| !own.contains(n)).collect();
    list.sort_by(|a, b| {
        let at = table.contains(a.0);
        let bt = table.contains(b.0);
        bt.cmp(&at).then(b.1.hands.total_cmp(&a.1.hands))
    });
    let book = s.reputation.read();
    Json(list.iter().take(60).map(|(n, st)| opponent_json(n, st, book.get(n))).collect::<Vec<_>>()).into_response()
}

pub(super) async fn opponent_detail(State(s): State<Arc<Shared>>, Path((slot, name)): Path<(usize, String)>) -> Response {
    off_runtime(move || opponent_detail_blocking(&s, slot, name)).await.into_response()
}

fn opponent_detail_blocking(s: &Shared, slot: usize, name: String) -> Result<Response, ApiError> {
    let bot = bot_name(s, slot)?;
    let st = s.models.read().players.get(&name).cloned().unwrap_or_default();
    let rep = s.reputation.read().get(&name).cloned();
    let mut detail = opponent_json(&name, &st, rep.as_ref());
    let rows = seat_history::recent_seat_hands(s, &bot, 3000, false)?;
    let (mut shared_hands, mut hero_wins, mut opp_wins, mut won_from, mut lost_to, mut table_net) = (0, 0, 0, 0i64, 0i64, 0i64);
    let mut trend = Vec::new();
    for r in rows.iter().rev() {
        let Ok(h) = serde_json::from_str::<HandSummary>(&r.summary) else { continue };
        if !h.players.iter().any(|(_, n)| n == &name) {
            continue;
        }
        shared_hands += 1;
        let net = r.net.unwrap_or(0);
        table_net += net;
        let winners: Vec<&str> = r.winners.split(',').collect();
        let mut direct = 0;
        if net > 0 && winners.contains(&r.bot.as_str()) {
            hero_wins += 1;
            won_from += net;
            direct = net;
        } else if net < 0 && winners.contains(&name.as_str()) {
            opp_wins += 1;
            lost_to += -net;
            direct = net;
        }
        trend.push(json!({"hand_id": r.hand_id, "ts": parse_ts(&r.ended_at), "net": r.net, "direct": direct, "cumulative": table_net, "result": if net > 0 { "won" } else if net < 0 { "lost" } else { "even" }}));
    }
    detail["head_to_head_hands"] = json!(shared_hands);
    detail["hero_wins"] = json!(hero_wins);
    detail["opponent_wins"] = json!(opp_wins);
    detail["won_from"] = json!(won_from);
    detail["lost_to"] = json!(lost_to);
    detail["table_net"] = json!(table_net);
    detail["trend"] = json!(trend);
    Ok(Json(detail).into_response())
}

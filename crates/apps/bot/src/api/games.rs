//! Decision accuracy and the "What would Svanbot do?" quiz (0220), both graded by
//! [`sv10_stats::grading`].

use super::*;
use sv10_stats::grading::{Grade, accuracy, grade, report};

/// Days of analyst audits the accuracy panel grades.
const ACCURACY_DAYS: i64 = 7;

pub(super) async fn decision_accuracy(State(s): State<Arc<Shared>>) -> Response {
    off_runtime(move || Json(accuracy_json(&s.store, |b| s.current_name(b)))).await.into_response()
}

/// Grades of the analyst's deep re-solves (the live choice against the deep search's best) over the
/// last [`ACCURACY_DAYS`], fleet-wide, per bot and per street, with the worst decisions.
/// `current` maps a stored bot name to the bot's current name, so a renamed bot is one row.
pub(super) fn accuracy_json(store: &sv10_store::store::Store, current: impl Fn(&str) -> String) -> Value {
    let since = (chrono::Utc::now() - chrono::Duration::days(ACCURACY_DAYS)).to_rfc3339();
    let mut rows = snapshot_read("decision audits", store.audit_results_since(&since));
    for (_, r) in rows.iter_mut() {
        r.bot = current(&r.bot);
    }
    let fleet = report(rows.iter().map(|(_, r)| (r.gap_bb, r.pot_bb)));
    let mut bots: Vec<String> = rows.iter().map(|(_, r)| r.bot.clone()).collect();
    bots.sort();
    bots.dedup();
    let per_bot: Vec<Value> = bots
        .iter()
        .map(|b| json!({"bot": b, "report": report(rows.iter().filter(|(_, r)| &r.bot == b).map(|(_, r)| (r.gap_bb, r.pot_bb)))}))
        .collect();
    let per_street: Vec<Value> = ["preflop", "flop", "turn", "river"]
        .iter()
        .map(|st| json!({"street": st, "report": report(rows.iter().filter(|(_, r)| r.street == *st).map(|(_, r)| (r.gap_bb, r.pot_bb)))}))
        .collect();
    let mut worst: Vec<&(String, sv10_store::store::AuditResult)> =
        rows.iter().filter(|(_, r)| grade(r.gap_bb, r.pot_bb) >= Grade::Mistake).collect();
    worst.sort_by(|a, b| (b.1.gap_bb / b.1.pot_bb.max(1.0)).total_cmp(&(a.1.gap_bb / a.1.pot_bb.max(1.0))));
    let worst: Vec<Value> = worst
        .iter()
        .take(8)
        .map(|(ts, r)| {
            json!({"ts": parse_ts(ts), "bot": r.bot, "hand_id": r.hand_id, "street": r.street, "live_action": r.live_action,
                "deep_action": r.deep_action, "loss_bb": r.gap_bb, "pot_bb": r.pot_bb, "grade": grade(r.gap_bb, r.pot_bb)})
        })
        .collect();
    json!({"days": ACCURACY_DAYS, "fleet": fleet, "bots": per_bot, "streets": per_street, "worst": worst})
}

pub(super) async fn quiz(State(s): State<Arc<Shared>>) -> Response {
    let spot = off_runtime(move || {
        let bb = s.big_blind();
        s.store.random_quiz_spot(50_000, 3, (10.0 * bb) as i64).map(|q| q.and_then(|q| quiz_json(&q, bb)))
    })
    .await;
    match spot.and_then(|r| store_read("quiz spot", r)) {
        Err(r) => r.into_response(),
        Ok(Some(v)) => Json(v).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(json!({"detail": "No quiz spot recorded yet"}))).into_response(),
    }
}

/// A quiz question from a recorded decision: the situation and every candidate the bot weighed,
/// each pre-graded as if chosen (loss against the best candidate's EV, in `bb`-sized big blinds).
pub(super) fn quiz_json(q: &sv10_store::store::QuizSpot, bb: f64) -> Option<Value> {
    let detail: Value = serde_json::from_str(&q.detail).ok()?;
    let candidates = detail["candidates"].as_array()?;
    let evs: Vec<f64> = candidates.iter().map(|c| c["ev"].as_f64().unwrap_or(f64::NEG_INFINITY)).collect();
    let best = evs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !best.is_finite() {
        return None;
    }
    let pot_bb = (q.pot + q.to_call) as f64 / bb.max(1.0);
    let options: Vec<Value> = candidates
        .iter()
        .zip(&evs)
        .map(|(c, ev)| {
            let loss_bb = (best - ev).max(0.0) / bb.max(1.0);
            json!({"action": c["action"], "amount": c["amount"], "ev_bb": ev / bb.max(1.0), "loss_bb": loss_bb,
                "grade": grade(loss_bb, pot_bb), "accuracy": accuracy(loss_bb, pot_bb), "fold_prob": c["fold_prob"],
                "chosen_by_bot": c["action"] == json!(q.action) && (c["amount"].is_null() || c["amount"].as_i64() == q.amount)})
        })
        .collect();
    Some(json!({"id": q.id, "bot": q.bot, "street": q.street, "hole": detail["hole"], "board": detail["board"], "pot": q.pot,
        "to_call": q.to_call, "bb": bb, "opponents": detail["opponents"], "bot_action": q.action, "options": options, "reason": detail["reason"]}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quiz_question_grades_every_option_against_the_best() {
        let q = sv10_store::store::QuizSpot {
            id: 7,
            bot: "A".into(),
            hand_id: "h".into(),
            street: "turn".into(),
            action: "check".into(),
            amount: None,
            pot: 1_000,
            to_call: 0,
            detail: json!({"hole": ["As", "Kd"], "board": ["2c", "7d", "9h", "Js"], "opponents": 1, "candidates": [
                {"action": "check", "amount": null, "ev": 240.0}, {"action": "raise", "amount": 500, "ev": 160.0},
                {"action": "raise", "amount": 1000, "ev": -200.0}]})
            .to_string(),
        };
        let v = quiz_json(&q, 20.0).unwrap();
        let opts = v["options"].as_array().unwrap();
        assert_eq!(opts[0]["grade"], json!("best"));
        assert_eq!(opts[0]["loss_bb"], json!(0.0));
        assert!((opts[1]["loss_bb"].as_f64().unwrap() - 4.0).abs() < 1e-9);
        assert_eq!(opts[1]["grade"], json!("good"), "4 bb of a 50 bb pot is 8%");
        assert_eq!(opts[2]["grade"], json!("blunder"), "22 bb of a 50 bb pot");
        assert_eq!(opts[0]["chosen_by_bot"], json!(true));
        assert_eq!(opts[1]["chosen_by_bot"], json!(false));
        let raised = sv10_store::store::QuizSpot { action: "raise".into(), amount: Some(1000), ..q.clone() };
        let v = quiz_json(&raised, 20.0).unwrap();
        assert_eq!(
            v["options"].as_array().unwrap().iter().map(|o| o["chosen_by_bot"] == json!(true)).collect::<Vec<_>>(),
            [false, false, true]
        );
        assert_eq!(v["hole"], json!(["As", "Kd"]));
        let empty = sv10_store::store::QuizSpot { detail: json!({"candidates": []}).to_string(), ..q };
        assert!(quiz_json(&empty, 20.0).is_none());
    }

    #[test]
    fn accuracy_on_an_empty_store_is_an_empty_report() {
        let shared = Shared::for_test("accuracy-empty", &["A"]);
        let v = accuracy_json(&shared.store, |b| shared.current_name(b));
        assert_eq!(v["fleet"]["decisions"], json!(0));
        assert!(v["worst"].as_array().unwrap().is_empty());
    }

    #[test]
    fn a_renamed_bot_is_one_accuracy_row() {
        // 2026-09-27: Decision accuracy listed SvanBotV7 next to SvanBotV10, the same bot renamed.
        let shared = Shared::for_test("accuracy-rename", &["SvanBotV10"]);
        shared.aliases.write().insert("SvanBotV10".into(), vec!["SvanBotV10".into(), "SvanBotV7".into()]);
        for (bot, hand) in [("SvanBotV7", "h1"), ("SvanBotV10", "h2")] {
            shared.store.insert_audit(bot, hand, "{}", None).unwrap();
            let job = shared.store.audit_batch(1).unwrap().remove(0);
            let r = sv10_store::store::AuditResult {
                bot: bot.into(),
                hand_id: hand.into(),
                street: "river".into(),
                live_action: "call".into(),
                deep_action: "call".into(),
                ..Default::default()
            };
            shared.store.finish_audit(job.id, Some(&r)).unwrap();
        }
        let v = accuracy_json(&shared.store, |b| shared.current_name(b));
        let bots: Vec<&str> = v["bots"].as_array().unwrap().iter().map(|b| b["bot"].as_str().unwrap()).collect();
        assert_eq!(bots, ["SvanBotV10"]);
        assert_eq!(v["bots"][0]["report"]["decisions"], json!(2));
    }
}

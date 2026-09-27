//! Stories for the dashboard (0178): a daily recap and a season timeline built from stored hands.
//! Presentation only: nothing here feeds decisions.

use serde_json::{Value, json};

/// One stored hand, reduced to what a story needs.
#[derive(Clone, Debug, PartialEq)]
pub struct StoryHand {
    /// Our bot that played it.
    pub bot: String,
    /// Server hand id.
    pub hand_id: String,
    /// End time, Unix seconds.
    pub ts: f64,
    /// Our net chips.
    pub net: i64,
    /// Final pot in chips.
    pub pot: i64,
}

/// Chips between timeline milestones for the fleet's cumulative result.
pub const MILESTONE_STEP: i64 = 100_000;

fn hand_json(h: &StoryHand, bb: f64) -> Value {
    json!({"bot": h.bot, "hand_id": h.hand_id, "ts": h.ts, "net": h.net, "net_bb": (h.net as f64 / bb * 10.0).round() / 10.0, "pot_bb": (h.pot as f64 / bb).round()})
}

/// The last `window_secs` before `now`: each bot's hands and net, the fleet total, and the biggest
/// win and loss.
pub fn recap(hands: &[StoryHand], now: f64, window_secs: f64, bb: f64) -> Value {
    let recent: Vec<&StoryHand> = hands.iter().filter(|h| h.ts > now - window_secs && h.ts <= now).collect();
    let mut bots: Vec<(String, i64, i64)> = Vec::new();
    for h in &recent {
        match bots.iter_mut().find(|b| b.0 == h.bot) {
            Some(b) => {
                b.1 += 1;
                b.2 += h.net;
            }
            None => bots.push((h.bot.clone(), 1, h.net)),
        }
    }
    bots.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    let best = recent.iter().max_by_key(|h| h.net).filter(|h| h.net > 0);
    let worst = recent.iter().min_by_key(|h| h.net).filter(|h| h.net < 0);
    json!({
        "window_hours": window_secs / 3600.0,
        "hands": recent.len(),
        "net": recent.iter().map(|h| h.net).sum::<i64>(),
        "bots": bots.iter().map(|(b, n, net)| json!({"bot": b, "hands": n, "net": net})).collect::<Vec<_>>(),
        "best": best.map(|h| hand_json(h, bb)),
        "worst": worst.map(|h| hand_json(h, bb)),
    })
}

/// Season story beats, oldest first: each time the fleet's cumulative result first crosses a
/// multiple of [`MILESTONE_STEP`], and the three biggest pots. `hands` must be oldest first.
pub fn timeline(hands: &[StoryHand], bb: f64) -> Vec<Value> {
    let mut out = Vec::new();
    let mut total = 0i64;
    let mut reached = 0i64;
    for h in hands {
        total += h.net;
        while total >= (reached + 1) * MILESTONE_STEP {
            reached += 1;
            out.push(json!({"ts": h.ts, "kind": "milestone", "text": format!("The fleet passes +{} chips", fmt_thousands(reached * MILESTONE_STEP))}));
        }
    }
    let mut big: Vec<&StoryHand> = hands.iter().collect();
    big.sort_by(|a, b| b.pot.cmp(&a.pot).then_with(|| a.ts.total_cmp(&b.ts)));
    for h in big.into_iter().take(3) {
        // A chopped pot nets next to nothing: "SvanBotV10 wins a 7180 bb pot (+0 bb)" (2026-09-27).
        let verb = if (h.net.unsigned_abs() as f64) < h.pot as f64 * 0.05 {
            "splits"
        } else if h.net > 0 {
            "wins"
        } else {
            "loses"
        };
        out.push(json!({"ts": h.ts, "kind": "big_pot", "bot": h.bot, "hand_id": h.hand_id,
            "text": format!("{} {} a {} bb pot ({:+} bb)", h.bot, verb, (h.pot as f64 / bb).round(), (h.net as f64 / bb).round())}));
    }
    out.sort_by(|a, b| a["ts"].as_f64().unwrap_or(0.0).total_cmp(&b["ts"].as_f64().unwrap_or(0.0)));
    out
}

fn fmt_thousands(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(bot: &str, id: &str, ts: f64, net: i64, pot: i64) -> StoryHand {
        StoryHand { bot: bot.into(), hand_id: id.into(), ts, net, pot }
    }

    #[test]
    fn recap_covers_only_the_window() {
        let hands = [
            h("A", "1", 100.0, 500, 1_000),
            h("A", "2", 5_000.0, -2_000, 4_000),
            h("B", "3", 6_000.0, 9_000, 18_000),
            h("B", "4", 9_000.0, 10, 40),
        ];
        let r = recap(&hands, 9_000.0, 5_000.0, 20.0);
        assert_eq!(r["hands"], 3, "the hand at t=100 is outside the window");
        assert_eq!(r["net"], 7_010);
        assert_eq!(r["bots"][0]["bot"], "B");
        assert_eq!(r["best"]["hand_id"], "3");
        assert_eq!(r["worst"]["net_bb"], -100.0);
        // A window with only losses has no best hand.
        assert!(recap(&[h("A", "9", 10.0, -5, 10)], 10.0, 60.0, 20.0)["best"].is_null());
    }

    #[test]
    fn a_chopped_pot_is_a_split_not_a_win() {
        let t = timeline(&[h("A", "1", 1.0, 3, 143_600), h("A", "2", 2.0, 60_000, 140_000), h("A", "3", 3.0, -70_000, 139_000)], 20.0);
        let texts: Vec<&str> = t.iter().filter(|e| e["kind"] == "big_pot").map(|e| e["text"].as_str().unwrap()).collect();
        assert_eq!(texts, ["A splits a 7180 bb pot (+0 bb)", "A wins a 7000 bb pot (+3000 bb)", "A loses a 6950 bb pot (-3500 bb)"]);
    }

    #[test]
    fn timeline_marks_each_milestone_once_and_the_biggest_pots() {
        let hands = [
            h("A", "1", 1.0, 150_000, 300_000),
            h("B", "2", 2.0, -60_000, 120_000),
            h("A", "3", 3.0, 120_000, 240_000),
            h("B", "4", 4.0, 5, 10),
        ];
        let t = timeline(&hands, 20.0);
        let milestones: Vec<&str> = t.iter().filter(|e| e["kind"] == "milestone").map(|e| e["text"].as_str().unwrap()).collect();
        assert_eq!(milestones, ["The fleet passes +100,000 chips", "The fleet passes +200,000 chips"]);
        let pots: Vec<&str> = t.iter().filter(|e| e["kind"] == "big_pot").map(|e| e["hand_id"].as_str().unwrap()).collect();
        assert_eq!(pots, ["1", "2", "3"]);
        assert!(t.windows(2).all(|w| w[0]["ts"].as_f64() <= w[1]["ts"].as_f64()), "oldest first");
    }
}

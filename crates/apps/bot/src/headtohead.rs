//! Measured results against each opponent: the big blinds that moved between us and them in
//! every ordinary hand they were dealt into ([`sv10_core::flow`], 0221), with a
//! normal-approximation confidence interval. Used to steer table selection away from players
//! who demonstrably beat us, and reported by `review`, the monitor and the dashboard.
//!
//! Every contribution is priced in the hand's own recorded blind before it enters the ledger,
//! so a result at BB 10 weighs the same as the same result at BB 20. Hands with no positive
//! recorded blind never enter a rate. All moments below are therefore in big blinds, and no
//! reader divides by a current blind.
//!
//! The population is champion play. Experiment-mode treatment hands (0291) are left out — the read
//! is [`Store::ordinary_results_after`] — because the verdict moves the fleet between tables and a
//! learner challenger's hands are not evidence about the policy that plays there; a control hand is
//! champion play and stays in. Every surface that prints the number says so (0361).
//!
//! "Demonstrably" is family-wise: with a hundred or more opponents tested at 95% each, a few
//! would clear the bar by luck alone, so a verdict needs the upper bound below zero at the
//! Bonferroni-corrected level for the number of opponents tested ([`family_z`]).

use serde::Serialize;
use std::collections::HashMap;
use sv10_core::model::HandSummary;
use sv10_stats::normal::family_z;
use sv10_store::store::Store;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct HeadToHead {
    /// Shared hands with a positive recorded blind: the only hands that count in a rate.
    pub hands: f64,
    sum: f64,
    sum_sq: f64,
    /// The single worst hand, so one cooler does not read as a leak (0209). Not part of any
    /// serialized read: it exists for [`Self::mean_without_biggest_loss`].
    #[serde(skip)]
    low: f64,
}

/// A reconciled chip flow in big blinds of the hand it came from, or `None` when the
/// recorded blind is missing or nonpositive: an unpriced hand never enters a rate, and no
/// current-blind fallback is invented for it.
fn price(chips: f64, bb: i64) -> Option<f64> {
    (bb > 0).then_some(chips / bb as f64)
}

impl HeadToHead {
    pub(crate) fn add(&mut self, net_bb: f64) {
        self.hands += 1.0;
        self.sum += net_bb;
        self.sum_sq += net_bb * net_bb;
        self.low = if self.hands == 1.0 { net_bb } else { self.low.min(net_bb) };
    }

    /// Mean big blinds per priced hand.
    pub fn mean(&self) -> f64 {
        sv10_stats::moments::mean(self.hands, self.sum)
    }

    /// Mean with the single worst hand removed, so one cooler does not read as a leak (0209).
    pub fn mean_without_biggest_loss(&self) -> f64 {
        if self.hands < 2.0 { self.mean() } else { (self.sum - self.low) / (self.hands - 1.0) }
    }

    /// Half-width of the interval of the mean at `z` standard errors, in big blinds per hand.
    fn half_width(&self, z: f64) -> f64 {
        sv10_stats::moments::half_width(self.hands, self.sum, self.sum_sq, z)
    }

    /// Upper end of the interval of the mean at `z` standard errors, in big blinds per hand.
    pub fn upper(&self, z: f64) -> f64 {
        self.mean() + self.half_width(z)
    }

    /// Upper end of this opponent's own 95% interval of the mean, in big blinds per hand.
    pub fn upper_95(&self) -> f64 {
        self.upper(1.96)
    }

    /// Whether this opponent beats us over at least `min_hands` hands, at 95% family-wise across
    /// the `tested` opponents with that many hands.
    pub fn beats_us(&self, min_hands: f64, tested: usize) -> bool {
        self.hands >= min_hands && self.mean() + self.half_width(family_z(tested)) < 0.0
    }

    /// Whether we beat this opponent, on the same family-wise terms as [`Self::beats_us`].
    pub fn we_beat(&self, min_hands: f64, tested: usize) -> bool {
        self.hands >= min_hands && self.mean() - self.half_width(family_z(tested)) > 0.0
    }
}

/// Opponents with at least `min_hands` hands: the size of the family a verdict is drawn from.
pub fn tested(table: &HashMap<String, HeadToHead>, min_hands: f64) -> usize {
    table.values().filter(|h| h.hands >= min_hands).count()
}

/// Head-to-head totals kept current incrementally: only ordinary hands stored since the last update
/// are parsed (a treatment hand is skipped by the read, not by a filter here — 0361).
#[derive(Default)]
pub struct Ledger {
    pub table: HashMap<String, HeadToHead>,
    /// Hands left out because their contributions do not reproduce the stored pot (LESSONS 20).
    pub unreconciled: u64,
    watermark: i64,
}

impl Ledger {
    pub fn update(&mut self, store: &Store, fleet: &[String]) {
        let Ok(rows) = store.ordinary_results_after(self.watermark) else { return };
        // The net is not asked for: the flow below is computed from the pot and the winners. A
        // recovered hand is stored before its net is filled, and the watermark has passed it by then,
        // so requiring one left those hands out until a restart counted them.
        for (rowid, bot, _net, summary, pot, winners) in rows {
            self.watermark = rowid;
            if !fleet.contains(&bot) {
                continue;
            }
            let Ok(h) = serde_json::from_str::<HandSummary>(&summary) else { continue };
            let Some(hero) = h.players.iter().find(|p| p.1 == bot).map(|p| p.0) else { continue };
            let winners: Vec<&str> = winners.split(',').filter(|w| !w.is_empty()).collect();
            let Some(flows) = sv10_core::flow::flow_to_hero(&h, pot, &winners, hero) else {
                self.unreconciled += 1;
                continue;
            };
            for (seat, chips) in flows {
                let Some(q) = price(chips, h.bb) else { continue };
                let name = &h.players.iter().find(|p| p.0 == seat).expect("flow seats are dealt in").1;
                if !fleet.contains(name) {
                    self.table.entry(name.clone()).or_default().add(q);
                }
            }
        }
    }
}

/// Head-to-head totals for every opponent across all of the fleet's recorded ordinary hands.
pub fn compute(store: &Store, fleet: &[String]) -> HashMap<String, HeadToHead> {
    let mut ledger = Ledger::default();
    ledger.update(store, fleet);
    ledger.table
}

/// The head-to-head read of one opponent: the big-blind flow attributed to *their seat* in the fleet's
/// ordinary hands, in bb/100, with its 95% interval and the family-wise flags, or `None` below
/// `min_hands` shared hands.
///
/// One implementation, so every surface that shows a rivalry number shows the same one. The seat
/// card's own record is a different quantity — our net in every hand they were dealt into, a table
/// result shared with everyone at it — and labelling that as an edge put "+6.4 bb/100" in front of
/// the operator for the one opponent the ledger calls a −415 bb/100 nemesis (0280).
pub fn read_one(table: &HashMap<String, HeadToHead>, name: &str, min_hands: f64) -> Option<serde_json::Value> {
    let h = table.get(name).filter(|h| h.hands >= min_hands)?;
    let family = table.values().filter(|o| o.hands >= min_hands).count();
    let per100 = |bb: f64| (bb * 100.0 * 10.0).round() / 10.0;
    let half = h.upper_95() - h.mean();
    Some(serde_json::json!({
        "hands": h.hands, "bb_per_100": per100(h.mean()),
        "low_95": per100(h.mean() - half), "high_95": per100(h.upper_95()),
        "beats_us": h.beats_us(min_hands, family), "we_beat": h.we_beat(min_hands, family),
    }))
}

/// Rivalry cards for the dashboard (0180): the `n` opponents we do worst and best against over at
/// least `min_hands` shared hands, in bb/100 with the 95% interval, and their avatar when seen.
pub fn rivals(table: &HashMap<String, HeadToHead>, avatars: &HashMap<String, String>, min_hands: f64, n: usize) -> serde_json::Value {
    let mut rows: Vec<(&String, &HeadToHead)> = table.iter().filter(|(_, h)| h.hands >= min_hands).collect();
    rows.sort_by(|a, b| a.1.mean().total_cmp(&b.1.mean()).then_with(|| a.0.cmp(b.0)));
    let card = |(name, _h): &(&String, &HeadToHead)| {
        let mut read = read_one(table, name, min_hands).unwrap_or(serde_json::Value::Null);
        read["name"] = serde_json::json!(name);
        read["avatar_url"] = serde_json::json!(avatars.get(*name));
        read
    };
    let nemeses: Vec<_> = rows.iter().take(n).filter(|(_, h)| h.mean() < 0.0).map(card).collect();
    let donors: Vec<_> = rows.iter().rev().take(n).filter(|(_, h)| h.mean() > 0.0).map(card).collect();
    serde_json::json!({"min_hands": min_hands, "opponents": rows.len(), "nemeses": nemeses, "donors": donors})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(results: &[f64]) -> HeadToHead {
        let mut h = HeadToHead::default();
        results.iter().for_each(|r| h.add(*r));
        h
    }

    /// Mixed blinds must not move a verdict: 200 losses of 10 chips at BB 10 (-1 bb each)
    /// plus 100 wins of 20 chips at BB 20 (+1 bb each) average 0 chips a hand — the old
    /// chip-weighted ledger read that as inconclusive (mean 0, upper +1.6 chips at current
    /// BB 20) — but -1/3 bb a hand, a genuine beating the corrected ledger flags.
    #[test]
    fn mixed_blinds_price_each_hand_before_the_verdict() {
        assert_eq!(price(10.0, 10), Some(1.0));
        assert_eq!(price(20.0, 20), Some(1.0));
        assert_eq!(price(10.0, 0), None);
        assert_eq!(price(10.0, -20), None);

        let shared = crate::live::Shared::for_test("h2h-mixed-blind", &["b"]);
        // Hero ("b") is seat 0 and posts the small blind; "Villain" is seat 1.
        // Group A: bb 10, hero calls 5 more, villain wins a pot of 20: flow -10 chips.
        // Group B: bb 20, hero calls 10 more, hero wins a pot of 40: flow +20 chips.
        let hand = |id: &str, bb: i64, calls: &[(usize, i64, i64, i64)], pot: i64, net: i64, winners: &str| {
            let summary = serde_json::json!({
                "players": [[0, "b"], [1, "Villain"]], "button": 0, "bb": bb, "stacks": [],
                "history": calls.iter().map(|(seat, to, bet_before, pot_before)| serde_json::json!(
                    {"seat": seat, "street": "Preflop", "kind": "Call", "to": to,
                     "pot_before": pot_before, "to_call_before": to - bet_before,
                     "bet_before": bet_before, "full_raise": false})).collect::<Vec<_>>(),
                "board": [], "shown": [],
            });
            sv10_store::store::HandRow {
                bot: "b".into(),
                hand_id: id.into(),
                table_id: "t".into(),
                ended_at: "2026-09-17T00:00:00Z".into(),
                hole: "AhKd".into(),
                board: "2c3c4c5c7d".into(),
                pot,
                net: Some(net),
                winners: winners.into(),
                summary: serde_json::to_string(&summary).unwrap(),
                ..Default::default()
            }
        };
        for i in 0..200 {
            shared.store.insert_hand(&hand(&format!("a{i}"), 10, &[(0, 10, 5, 15)], 20, -10, "Villain")).unwrap();
        }
        for i in 0..100 {
            shared.store.insert_hand(&hand(&format!("b{i}"), 20, &[(0, 20, 10, 30)], 40, 20, "b")).unwrap();
        }
        // A hand with no positive recorded blind reconciles but never enters a rate.
        shared.store.insert_hand(&hand("c0", 0, &[(0, 10, 0, 0), (1, 10, 0, 10)], 20, 10, "b")).unwrap();
        let mut ledger = Ledger::default();
        ledger.update(&shared.store, &["b".to_string()]);
        assert_eq!(ledger.unreconciled, 0, "all three fixtures reconcile their stored pots");
        let table = ledger.table;
        let h = &table["Villain"];
        assert_eq!(h.hands, 300.0, "only priced hands count");
        assert!((h.mean() + 1.0 / 3.0).abs() < 1e-9, "(-200 + 100) / 300 bb a hand, not 0 chips");
        assert!(h.beats_us(300.0, 1), "a significant -1/3 bb a hand is a beating");
        assert!(!h.we_beat(300.0, 1));
        let read = read_one(&table, "Villain", 300.0).unwrap();
        assert_eq!(read["hands"], 300.0);
        assert_eq!(read["bb_per_100"], -33.3);
        assert_eq!(read["low_95"], -44.0);
        assert_eq!(read["high_95"], -22.7);
        assert_eq!(read["beats_us"], true);
        // Paired old-vs-new on the same 300 deals: the old chip-weighted moments read mean 0
        // with an upper bound above zero at current BB 20 — inconclusive — while the priced
        // ledger above flags the beating. A policy `sim paired` has no surface here: no seated
        // decision reads the ledger, only table selection does.
        let mut old = HeadToHead::default();
        for _ in 0..200 {
            old.add(-10.0);
        }
        for _ in 0..100 {
            old.add(20.0);
        }
        assert!((old.mean()).abs() < 1e-9);
        assert!(old.upper_95() / 20.0 > 0.0, "the old pricing could not flag these same deals");
        let panel = rivals(&table, &HashMap::new(), 300.0, 5);
        let shown = panel["nemeses"].as_array().unwrap().iter().find(|r| r["name"] == "Villain").unwrap();
        for field in ["hands", "bb_per_100", "low_95", "high_95", "beats_us", "we_beat"] {
            assert_eq!(read[field], shown[field], "{field} differs between the two surfaces");
        }
        let nemeses = crate::findings::nemesis(&table);
        assert_eq!(nemeses.len(), 1);
        assert!((nemeses[0].value + 100.0 / 3.0).abs() < 1e-9, "findings price the same normalized rate");
    }

    /// 0280: the seat card and the Rivals panel must show the *same* number for an opponent. The
    /// card used to show our bot's net in the hands they sat in — a result the whole table shares —
    /// under the label OUR EDGE, so the one opponent the ledger calls a −415 bb/100 nemesis read
    /// +6.4 on the card.
    #[test]
    fn the_seat_read_is_the_number_the_rivals_panel_shows() {
        let table: HashMap<String, HeadToHead> = [
            ("silentflute".to_string(), with(&[-2.0, -3.0].repeat(100))),
            ("w1nner".to_string(), with(&[1.5, 2.5].repeat(100))),
            ("quiet".to_string(), with(&[0.05, -0.05].repeat(10))),
        ]
        .into_iter()
        .collect();
        let panel = rivals(&table, &HashMap::new(), 150.0, 5);
        let read = read_one(&table, "silentflute", 150.0).unwrap();
        let shown = panel["nemeses"].as_array().unwrap().iter().find(|r| r["name"] == "silentflute").unwrap();
        for field in ["hands", "bb_per_100", "low_95", "high_95", "beats_us", "we_beat"] {
            assert_eq!(read[field], shown[field], "{field} differs between the two surfaces");
        }
        assert!(read["bb_per_100"].as_f64().unwrap() < 0.0, "a nemesis reads negative on both");
        assert!(read["beats_us"] == true, "a consistent −2.5 bb a hand clears 95% even in a family");
        assert!(read_one(&table, "quiet", 150.0).is_none(), "below the shared-hands floor");
        assert!(read_one(&table, "nobody", 150.0).is_none(), "an opponent we have never shared a hand with");
        let w = read_one(&table, "w1nner", 150.0).unwrap();
        assert!(w["we_beat"] == true, "and a donor is flagged as ours to beat");
    }

    #[test]
    fn family_z_grows_with_the_opponents_tested() {
        assert!((family_z(1) - 1.96).abs() < 1e-3, "one opponent: the ordinary 95% bound");
        assert!((family_z(130) - 3.55).abs() < 0.01, "{}", family_z(130));
        assert!((sv10_stats::normal::normal_quantile(0.975) - 1.959964).abs() < 1e-6);
    }

    #[test]
    fn a_verdict_that_clears_95_alone_does_not_survive_a_large_family() {
        // Mean -1 bb, SD ~5.1 over 200 hands: upper bound -1 + 1.96 * 0.36 < 0, but not at z 3.55.
        let h = with(&[4.0, -6.0].repeat(100));
        assert!(h.beats_us(150.0, 1));
        assert!(!h.beats_us(150.0, 130), "one of 130 at 95% is what luck alone produces");
        assert!(!h.beats_us(300.0, 1), "under min_hands never counts");
        assert!(with(&[-2.0, -3.0].repeat(100)).beats_us(150.0, 130), "a real edge survives the correction");
        assert!(with(&[2.0, 3.0].repeat(100)).we_beat(150.0, 130));
    }

    #[test]
    fn rivals_rank_nemeses_and_donors_with_intervals_and_avatars() {
        let mut table = HashMap::new();
        table.insert("Kenza".to_string(), with(&[-2.0, -3.0].repeat(100)));
        table.insert("jonnaBee3".to_string(), with(&[1.5, 0.5].repeat(100)));
        table.insert("coinflip".to_string(), with(&[25.0, -25.0].repeat(100)));
        table.insert("rare".to_string(), with(&[-45.0; 10]));
        let avatars = HashMap::from([("Kenza".to_string(), "https://x/k.png".to_string())]);
        let r = rivals(&table, &avatars, 150.0, 5);
        assert_eq!(r["opponents"], 3, "under 150 hands is left out");
        assert_eq!(r["nemeses"][0]["name"], "Kenza");
        assert_eq!(r["nemeses"][0]["bb_per_100"], -250.0);
        assert_eq!(r["nemeses"][0]["beats_us"], true);
        assert_eq!(r["nemeses"][0]["avatar_url"], "https://x/k.png");
        assert_eq!(r["donors"][0]["name"], "jonnaBee3");
        assert_eq!(r["donors"][0]["we_beat"], true);
        // A break-even coin flip is neither a nemesis nor a donor.
        assert!(r["nemeses"].as_array().unwrap().iter().chain(r["donors"].as_array().unwrap()).all(|c| c["name"] != "coinflip"));
    }
}

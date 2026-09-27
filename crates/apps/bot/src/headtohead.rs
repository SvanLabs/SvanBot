//! Measured results against each opponent: the chips that moved between us and them in every
//! ordinary hand they were dealt into ([`sv10_core::flow`], 0221), with a normal-approximation
//! confidence interval. Used to steer table selection away from players who demonstrably beat us,
//! and reported by `review`, the monitor and the dashboard.
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
    pub hands: f64,
    sum: f64,
    sum_sq: f64,
}

impl HeadToHead {
    pub(crate) fn add(&mut self, net: f64) {
        self.hands += 1.0;
        self.sum += net;
        self.sum_sq += net * net;
    }

    /// Mean chips per hand.
    pub fn mean(&self) -> f64 {
        sv10_stats::moments::mean(self.hands, self.sum)
    }

    /// Half-width of the interval of the mean at `z` standard errors, in chips per hand.
    fn half_width(&self, z: f64) -> f64 {
        sv10_stats::moments::half_width(self.hands, self.sum, self.sum_sq, z)
    }

    /// Upper end of this opponent's own 95% interval of the mean, in chips per hand.
    pub fn upper_95(&self) -> f64 {
        self.mean() + self.half_width(1.96)
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
        for (rowid, bot, net, summary, pot, winners) in rows {
            self.watermark = rowid;
            if !fleet.contains(&bot) {
                continue;
            }
            let (Some(_), Ok(h)) = (net, serde_json::from_str::<HandSummary>(&summary)) else { continue };
            let Some(hero) = h.players.iter().find(|p| p.1 == bot).map(|p| p.0) else { continue };
            let winners: Vec<&str> = winners.split(',').filter(|w| !w.is_empty()).collect();
            let Some(flows) = sv10_core::flow::flow_to_hero(&h, pot, &winners, hero) else {
                self.unreconciled += 1;
                continue;
            };
            for (seat, chips) in flows {
                let name = &h.players.iter().find(|p| p.0 == seat).expect("flow seats are dealt in").1;
                if !fleet.contains(name) {
                    self.table.entry(name.clone()).or_default().add(chips);
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

/// The head-to-head read of one opponent: the chip flow attributed to *their seat* in the fleet's
/// ordinary hands, in bb/100, with its 95% interval and the family-wise flags, or `None` below
/// `min_hands` shared hands.
///
/// One implementation, so every surface that shows a rivalry number shows the same one. The seat
/// card's own record is a different quantity — our net in every hand they were dealt into, a table
/// result shared with everyone at it — and labelling that as an edge put "+6.4 bb/100" in front of
/// the operator for the one opponent the ledger calls a −415 bb/100 nemesis (0280).
pub fn read_one(table: &HashMap<String, HeadToHead>, name: &str, min_hands: f64, bb: f64) -> Option<serde_json::Value> {
    let h = table.get(name).filter(|h| h.hands >= min_hands)?;
    let family = table.values().filter(|o| o.hands >= min_hands).count();
    let per100 = |chips: f64| (chips / bb.max(1.0) * 100.0 * 10.0).round() / 10.0;
    let half = h.upper_95() - h.mean();
    Some(serde_json::json!({
        "hands": h.hands, "bb_per_100": per100(h.mean()),
        "low_95": per100(h.mean() - half), "high_95": per100(h.upper_95()),
        "beats_us": h.beats_us(min_hands, family), "we_beat": h.we_beat(min_hands, family),
    }))
}

/// Rivalry cards for the dashboard (0180): the `n` opponents we do worst and best against over at
/// least `min_hands` shared hands, in bb/100 with the 95% interval, and their avatar when seen.
pub fn rivals(
    table: &HashMap<String, HeadToHead>,
    avatars: &HashMap<String, String>,
    min_hands: f64,
    bb: f64,
    n: usize,
) -> serde_json::Value {
    let mut rows: Vec<(&String, &HeadToHead)> = table.iter().filter(|(_, h)| h.hands >= min_hands).collect();
    rows.sort_by(|a, b| a.1.mean().total_cmp(&b.1.mean()).then_with(|| a.0.cmp(b.0)));
    let card = |(name, _h): &(&String, &HeadToHead)| {
        let mut read = read_one(table, name, min_hands, bb).unwrap_or(serde_json::Value::Null);
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

    /// 0280: the seat card and the Rivals panel must show the *same* number for an opponent. The
    /// card used to show our bot's net in the hands they sat in — a result the whole table shares —
    /// under the label OUR EDGE, so the one opponent the ledger calls a −415 bb/100 nemesis read
    /// +6.4 on the card.
    #[test]
    fn the_seat_read_is_the_number_the_rivals_panel_shows() {
        let table: HashMap<String, HeadToHead> = [
            ("villain0".to_string(), with(&[-40.0, -60.0].repeat(100))),
            ("villain1".to_string(), with(&[30.0, 50.0].repeat(100))),
            ("quiet".to_string(), with(&[1.0, -1.0].repeat(10))),
        ]
        .into_iter()
        .collect();
        let panel = rivals(&table, &HashMap::new(), 150.0, 20.0, 5);
        let read = read_one(&table, "villain0", 150.0, 20.0).unwrap();
        let shown = panel["nemeses"].as_array().unwrap().iter().find(|r| r["name"] == "villain0").unwrap();
        for field in ["hands", "bb_per_100", "low_95", "high_95", "beats_us", "we_beat"] {
            assert_eq!(read[field], shown[field], "{field} differs between the two surfaces");
        }
        assert!(read["bb_per_100"].as_f64().unwrap() < 0.0, "a nemesis reads negative on both");
        assert!(read["beats_us"] == true, "a consistent −50 chips a hand clears 95% even in a family");
        assert!(read_one(&table, "quiet", 150.0, 20.0).is_none(), "below the shared-hands floor");
        assert!(read_one(&table, "nobody", 150.0, 20.0).is_none(), "an opponent we have never shared a hand with");
        let w = read_one(&table, "villain1", 150.0, 20.0).unwrap();
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
        // Mean -1 chip, SD ~5.1 over 200 hands: upper bound -1 + 1.96 * 0.36 < 0, but not at z 3.55.
        let h = with(&[4.0, -6.0].repeat(100));
        assert!(h.beats_us(150.0, 1));
        assert!(!h.beats_us(150.0, 130), "one of 130 at 95% is what luck alone produces");
        assert!(!h.beats_us(300.0, 1), "under min_hands never counts");
        assert!(with(&[-40.0, -60.0].repeat(100)).beats_us(150.0, 130), "a real edge survives the correction");
        assert!(with(&[40.0, 60.0].repeat(100)).we_beat(150.0, 130));
    }

    #[test]
    fn rivals_rank_nemeses_and_donors_with_intervals_and_avatars() {
        let mut table = HashMap::new();
        table.insert("villain0".to_string(), with(&[-40.0, -60.0].repeat(100)));
        table.insert("villain2".to_string(), with(&[30.0, 10.0].repeat(100)));
        table.insert("coinflip".to_string(), with(&[500.0, -500.0].repeat(100)));
        table.insert("rare".to_string(), with(&[-900.0; 10]));
        let avatars = HashMap::from([("villain0".to_string(), "https://x/k.png".to_string())]);
        let r = rivals(&table, &avatars, 150.0, 20.0, 5);
        assert_eq!(r["opponents"], 3, "under 150 hands is left out");
        assert_eq!(r["nemeses"][0]["name"], "villain0");
        assert_eq!(r["nemeses"][0]["bb_per_100"], -250.0);
        assert_eq!(r["nemeses"][0]["beats_us"], true);
        assert_eq!(r["nemeses"][0]["avatar_url"], "https://x/k.png");
        assert_eq!(r["donors"][0]["name"], "villain2");
        assert_eq!(r["donors"][0]["we_beat"], true);
        // A break-even coin flip is neither a nemesis nor a donor.
        assert!(r["nemeses"].as_array().unwrap().iter().chain(r["donors"].as_array().unwrap()).all(|c| c["name"] != "coinflip"));
    }
}

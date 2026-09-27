//! Table selection inputs: how soft or tough the current table is.

use super::*;

pub struct TableQuality {
    pub soft: usize,
    pub tough: usize,
    pub opponents: usize,
    /// Opponents on the current season leaderboard at or above `seek_top_rank`, as (name, rank).
    pub top: Vec<(String, i64)>,
    pub summary: String,
}

/// Count soft, tough and top-ranked opponents at the table from live stats and season reputation.
pub(super) fn table_quality(shared: &Shared, tracker: &TableTracker, hero: &str) -> TableQuality {
    let models = shared.models.read();
    let book = shared.reputation.read();
    let h2h = shared.head_to_head.read();
    let tested = crate::headtohead::tested(&h2h, 300.0);
    let rate = |c: &sv10_core::model::Counter| if c.opp > 0.0 { Some((c.hit / c.opp) as f64) } else { None };
    let (mut soft, mut tough, mut n) = (0, 0, 0);
    let mut parts = Vec::new();
    let mut top = Vec::new();
    let seek = shared.config.seek_top_rank;
    for seat in tracker.seats.values().filter(|s| s.name != hero && !s.name.is_empty()) {
        n += 1;
        let st = models.players.get(&seat.name);
        let rep = book.get(&seat.name).or_else(|| book.get(&seat.name.to_lowercase()));
        let observed = st.map(|s| s.hands >= 25.0).unwrap_or(false);
        let (vpip, pfr, ftb) = match st {
            Some(s) => {
                let f = sv10_core::model::Counter {
                    opp: s.fold_vs_bet.iter().map(|c| c.opp).sum(),
                    hit: s.fold_vs_bet.iter().map(|c| c.hit).sum(),
                };
                (rate(&s.vpip).unwrap_or(0.3), rate(&s.pfr).unwrap_or(0.15), if f.opp >= 10.0 { rate(&f) } else { None })
            }
            None => (0.3, 0.15, None),
        };
        let is_soft = (observed && (vpip >= 0.38 || ftb.map(|f| f >= 0.6 || (f <= 0.2 && vpip >= 0.28)).unwrap_or(false)))
            || rep.map(|r| r.seasons >= 1 && r.strength < 0.3 && r.lifetime_hands >= 500).unwrap_or(false);
        // Measured results override style guesses: a player who beats us over hundreds of hands is tough.
        let beats_us = h2h.get(&seat.name).map(|h| h.beats_us(300.0, tested)).unwrap_or(false);
        let is_soft = is_soft && !beats_us;
        let is_tough = beats_us
            || !is_soft
                && (rep.map(|r| r.strength >= 0.85 && r.top10 >= 1).unwrap_or(false)
                    || (observed
                        && (0.16..=0.30).contains(&vpip)
                        && pfr >= 0.13
                        && ftb.map(|f| (0.35..=0.55).contains(&f)).unwrap_or(false)));
        if is_soft {
            soft += 1;
        }
        if is_tough {
            tough += 1;
        }
        let rank = rep.map(|r| r.current_rank).unwrap_or(0);
        let season_hands = rep.map(|r| r.current_hands).unwrap_or(0);
        let fleet = shared.config.bots.iter().any(|b| b.name.eq_ignore_ascii_case(&seat.name));
        // Seek established top bots only: early-season ranks are noise, and a top bot that
        // demonstrably beats us is not worth chasing.
        if seek > 0 && rank > 0 && rank <= seek && season_hands >= 300 && !fleet && !beats_us {
            top.push((seat.name.clone(), rank));
        }
        let label = if beats_us {
            "beats-us"
        } else if is_soft {
            "soft"
        } else if is_tough {
            "tough"
        } else {
            "?"
        };
        parts.push(if rank > 0 { format!("{}={label}#{rank}", seat.name) } else { format!("{}={label}", seat.name) });
    }
    TableQuality { soft, tough, opponents: n, top, summary: parts.join(", ") }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv10_core::model::{Counter, PlayerStats};
    use sv10_venue::tracker::SeatView;

    fn seat(seat: usize, name: &str) -> SeatView {
        SeatView { seat, name: name.into(), stack: 5_000, in_hand: true, ..Default::default() }
    }

    fn stats(hands: f32, vpip: f32, pfr: f32, fold: f32) -> PlayerStats {
        let c = |rate: f32, n: f32| Counter { opp: n, hit: rate * n };
        PlayerStats {
            hands,
            vpip: c(vpip, hands),
            pfr: c(pfr, hands),
            fold_vs_bet: [c(fold, 20.0), c(fold, 10.0), c(fold, 5.0)],
            ..Default::default()
        }
    }

    #[test]
    fn table_quality_labels_soft_tough_and_seekable_top_bots() {
        let shared = Shared::for_test("quality", &["Hero"]);
        {
            let mut m = shared.models.write();
            m.players.insert("Station".into(), stats(200.0, 0.55, 0.08, 0.2));
            m.players.insert("Reg".into(), stats(300.0, 0.22, 0.18, 0.45));
            m.players.insert("Shark".into(), stats(400.0, 0.24, 0.19, 0.45));
        }
        {
            let mut book = shared.reputation.write();
            let top = |rank| crate::reputation::Reputation { current_rank: rank, current_hands: 5_000, ..Default::default() };
            book.by_name.insert("Reg".into(), top(4));
            book.by_name.insert("Shark".into(), top(2));
        }
        // Shark beats us over 400 hands: tough, and never chased even though it ranks higher.
        let mut ledger = crate::headtohead::HeadToHead::default();
        for _ in 0..400 {
            ledger.add(-60.0);
        }
        ledger.add(10.0);
        shared.head_to_head.write().insert("Shark".into(), ledger);
        let mut t = TableTracker::default();
        t.reset_table();
        for (i, n) in ["Hero", "Station", "Reg", "Shark", "Unknown"].iter().enumerate() {
            t.seats.insert(i, seat(i, n));
        }
        let q = table_quality(&shared, &t, "Hero");
        assert_eq!(q.opponents, 4);
        assert_eq!((q.soft, q.tough), (1, 2), "{}", q.summary);
        assert_eq!(q.top, vec![("Reg".to_string(), 4)], "{}", q.summary);
        assert!(q.summary.contains("Shark=beats-us#2"), "{}", q.summary);
    }
}

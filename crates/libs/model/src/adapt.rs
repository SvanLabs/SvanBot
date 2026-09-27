//! Opponent adaptation study (0168): do recency-weighted opponent stats predict what opponents do
//! next better than all-time stats? Live observations enter the models at weight 1 forever, so an
//! opponent that changes style (a bot update mid-season) is read from thousands of stale hands.
//!
//! Hands are replayed in time order. Before each hand is observed, every tracked decision in it
//! (VPIP, PFR, 3-bet, fold to 3-bet, c-bet, fold to c-bet, bet first, fold and raise facing a bet
//! per street, went to showdown) is predicted from each opponent's stats so far, shrunk toward the
//! all-time population rate, and scored by log-loss. Each variant decays a player's own tallies
//! by `0.5^(1/half_life)` per hand of theirs; the population prior is never decayed, so every
//! variant shrinks toward the same prior.

use crate::model::{Counter, HandSummary, PlayerStats, hand_stats};
use std::collections::HashMap;

/// Prior opportunities a player's rate is shrunk toward the population with.
const PRIOR_WEIGHT: f32 = 12.0;

/// Score for one half-life: mean log-loss per scored decision, and its paired gain over all-time.
#[derive(Clone, Debug, PartialEq)]
pub struct AdaptScore {
    /// Half-life in the player's own hands (`f64::INFINITY` = all-time, today's model).
    pub half_life: f64,
    /// Decisions scored.
    pub n: usize,
    /// Mean log-loss, nats per decision.
    pub loss: f64,
    /// Mean log-loss improvement over all-time, nats per decision (positive = better).
    pub gain: f64,
    /// 95% half-width of `gain`.
    pub half_width: f64,
}

fn counters(s: &PlayerStats) -> Vec<&Counter> {
    let mut v = vec![&s.vpip, &s.pfr, &s.three_bet, &s.fold_to_3bet, &s.cbet, &s.fold_to_cbet, &s.wtsd];
    for street in 0..3 {
        v.push(&s.bet_first[street]);
        v.push(&s.fold_vs_bet[street]);
        v.push(&s.raise_vs_bet[street]);
    }
    v
}

fn decayed(s: &PlayerStats, f: f32) -> PlayerStats {
    let mut out = PlayerStats::default();
    out.merge_weighted(s, f);
    out
}

/// Replay `hands` (oldest first; each with the name of our bot in it, whose seat is skipped) and
/// score each half-life on the hands after the first `warmup`.
pub fn study(hands: &[(HandSummary, String)], half_lives: &[f64], warmup: usize) -> Vec<AdaptScore> {
    let k = half_lives.len();
    let mut players: Vec<HashMap<String, PlayerStats>> = vec![HashMap::new(); k];
    let mut population = PlayerStats::default();
    // Per scored decision: loss under each half-life.
    let mut losses: Vec<Vec<f64>> = vec![Vec::new(); k];
    for (i, (hand, hero)) in hands.iter().enumerate() {
        let stats = hand_stats(hand);
        for (seat, name) in &hand.players {
            if name == hero {
                continue;
            }
            let Some(now) = stats.get(seat) else { continue };
            if i >= warmup {
                let pop = counters(&population);
                for v in 0..half_lives.len() {
                    let own = players[v].get(name).cloned().unwrap_or_default();
                    for ((c, o), p) in counters(now).into_iter().zip(counters(&own)).zip(&pop) {
                        if c.opp <= 0.0 {
                            continue;
                        }
                        let prior = if p.opp > 0.0 { p.hit / p.opp } else { 0.5 };
                        let rate = f64::from(o.rate(prior, PRIOR_WEIGHT)).clamp(1e-4, 1.0 - 1e-4);
                        let (hit, miss) = (f64::from(c.hit), f64::from(c.opp - c.hit));
                        losses[v].push(-(hit * rate.ln() + miss * (1.0 - rate).ln()) / f64::from(c.opp));
                    }
                }
            }
            for (v, h) in half_lives.iter().enumerate() {
                let entry = players[v].entry(name.clone()).or_default();
                if h.is_finite() {
                    *entry = decayed(entry, 0.5f64.powf(1.0 / h) as f32);
                }
                entry.merge(now);
            }
            population.merge(now);
        }
    }
    let base = half_lives.iter().position(|h| h.is_infinite());
    (0..k)
        .map(|v| {
            let n = losses[v].len();
            let loss = losses[v].iter().sum::<f64>() / n.max(1) as f64;
            let (gain, half_width) = match base {
                Some(b) if n > 1 => {
                    let d: Vec<f64> = losses[b].iter().zip(&losses[v]).map(|(a, x)| a - x).collect();
                    let m = d.iter().sum::<f64>() / n as f64;
                    let var = d.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1) as f64;
                    (m, 1.96 * (var / n as f64).sqrt())
                }
                _ => (0.0, 0.0),
            };
            AdaptScore { half_life: half_lives[v], n, loss, gain, half_width }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sv10_engine::engine::{ActionKind, ActionRecord, Street};

    /// Heads-up preflop: V acts first (seat 1 is the button in this two-seat hand) and either
    /// raises (voluntarily in) or folds; Hero is in the big blind.
    fn hand(v_plays: bool) -> (HandSummary, String) {
        let kind = if v_plays { ActionKind::Raise } else { ActionKind::Fold };
        let rec = ActionRecord {
            seat: 1,
            street: Street::Preflop,
            kind,
            to: if v_plays { 60 } else { 0 },
            pot_before: 30,
            to_call_before: 10,
            bet_before: 10,
            full_raise: v_plays,
            think_ms: None,
            street_open: false,
        };
        let summary = HandSummary {
            players: vec![(0, "Hero".into()), (1, "V".into())],
            button: 1,
            bb: 20,
            history: vec![rec],
            board: vec![],
            shown: vec![],
            stacks: vec![(0, 2_000), (1, 2_000)],
        };
        (summary, "Hero".into())
    }

    #[test]
    fn a_model_store_with_a_half_life_reads_the_current_style_and_keeps_hand_counts() {
        let pattern = |i: usize, rate_pct: usize| (i * 37) % 100 < rate_pct;
        let hands: Vec<_> = (0..3_000).map(|i| hand(pattern(i, 80))).chain((0..1_000).map(|i| hand(pattern(i, 20)))).collect();
        let mut all_time = crate::model::ModelStore::default();
        let mut recent = crate::model::ModelStore { half_life_hands: 1_000.0, ..Default::default() };
        for (h, hero) in &hands {
            all_time.observe(h, Some(hero));
            recent.observe(h, Some(hero));
        }
        let rate = |m: &crate::model::ModelStore| {
            let v = &m.players["V"].vpip;
            v.hit / v.opp
        };
        assert!((rate(&all_time) - 0.65).abs() < 0.02, "all-time {}", rate(&all_time));
        // After exactly one half-life of the new 20% style the old 80% still weighs half: about 0.48,
        // well on the way from the all-time 0.65.
        assert!(rate(&recent) < rate(&all_time) - 0.15 && rate(&recent) > 0.2, "recent {}", rate(&recent));
        assert_eq!(recent.players["V"].hands, all_time.players["V"].hands, "hand counts are not decayed");
        assert_eq!(recent.population.vpip.opp, all_time.population.vpip.opp, "the population prior is not decayed");
    }

    #[test]
    fn recency_wins_when_an_opponent_changes_style_and_costs_little_when_not() {
        // V plays 80% of hands for 3,000 hands, then 20% for 1,000.
        let pattern = |i: usize, rate_pct: usize| (i * 37) % 100 < rate_pct;
        let mut changing: Vec<_> = (0..3_000).map(|i| hand(pattern(i, 80))).collect();
        changing.extend((0..1_000).map(|i| hand(pattern(i, 20))));
        let scores = study(&changing, &[f64::INFINITY, 200.0], 3_000);
        let (all_time, recent) = (&scores[0], &scores[1]);
        assert!(recent.gain > 0.05 && recent.gain - recent.half_width > 0.0, "{recent:?}");
        assert!(recent.loss < all_time.loss);
        // A steady opponent: recency only adds noise, so the gain is about zero or negative.
        let steady: Vec<_> = (0..4_000).map(|i| hand(pattern(i, 50))).collect();
        let scores = study(&steady, &[f64::INFINITY, 200.0], 3_000);
        assert!(scores[1].gain < 0.01, "{:?}", scores[1]);
    }
}

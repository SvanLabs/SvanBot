//! Pacing study (0244): how much the learner's inputs move between cycles.
//!
//! A search cycle plays one-knob challengers against clones of the most-observed opponents and
//! against the opponent models. This module rebuilds those models as they stood at chosen hand
//! rows (one chronological replay) and measures, between two snapshots, how far the population
//! moved and how far the candidate ranking moved. `learner pacing-study` runs the evaluations and
//! compares the population's effect with the seed noise two consecutive cycles see anyway.

use std::collections::HashSet;
use sv10_core::model::{HandSummary, ModelStore};

/// Chronological replay of stored hands into opponent models.
pub struct Replay {
    /// The models so far.
    pub models: ModelStore,
    seen: HashSet<String>,
    ours: Vec<String>,
}

impl Replay {
    /// Replay into `models`; `ours` are our seats (the hero of each hand, never modelled).
    pub fn new(models: ModelStore, ours: Vec<String>) -> Replay {
        Replay { models, seen: HashSet::new(), ours }
    }

    /// Fold in one hand. A hand two of our seats shared is stored once per seat; `hand_id`
    /// keeps it from counting twice. Returns whether the hand was new.
    pub fn observe(&mut self, hand_id: &str, hand: &HandSummary) -> bool {
        if !hand_id.is_empty() && !self.seen.insert(hand_id.to_string()) {
            return false;
        }
        let hero = hand.players.iter().find_map(|(_, name)| self.ours.contains(name).then_some(name.as_str()));
        self.models.observe(hand, hero);
        true
    }
}

/// The search pool as `live_pool` picks it: at least `min_hands`, most hands first, `max` of them.
pub fn pool(models: &ModelStore, min_hands: f32, max: usize) -> Vec<(String, f32)> {
    let mut p: Vec<(String, f32)> =
        models.players.iter().filter(|(_, s)| s.hands >= min_hands).map(|(n, s)| (n.clone(), s.hands)).collect();
    p.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    p.truncate(max);
    p
}

/// Every rate of a player's blended profile that the clones play from, as fractions.
pub fn rates(models: &ModelStore, name: &str) -> Vec<f32> {
    let p = models.profile(name);
    let mut v = vec![
        p.vpip,
        p.pfr,
        p.open_raise,
        p.limp,
        p.three_bet,
        p.call_open,
        p.fold_to_3bet,
        p.four_bet,
        p.fold_to_4bet,
        p.cbet,
        p.fold_to_cbet,
        p.wtsd,
        p.river_bluff,
    ];
    for a in [p.bet_first, p.fold_vs_bet, p.raise_vs_bet, p.fold_vs_size, p.open_pos, p.vpip_pos] {
        v.extend(a);
    }
    v
}

/// How far the search population moved between two snapshots.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct PopulationShift {
    /// Pool members in `b` that were not in `a`'s pool.
    pub members_changed: usize,
    /// Pool size in `b`.
    pub members: usize,
    /// Hand-weighted mean absolute change of the members' rates, in percentage points (members in
    /// both pools).
    pub rate_shift_pp: f64,
}

/// Population shift from `a` to `b` for the pool `min_hands`/`max` selects.
pub fn population_shift(a: &ModelStore, b: &ModelStore, min_hands: f32, max: usize) -> PopulationShift {
    let pa = pool(a, min_hands, max);
    let pb = pool(b, min_hands, max);
    let before: HashSet<&str> = pa.iter().map(|(n, _)| n.as_str()).collect();
    let (mut sum, mut weight) = (0.0f64, 0.0f64);
    for (name, hands) in pb.iter().filter(|(n, _)| before.contains(n.as_str())) {
        let (ra, rb) = (rates(a, name), rates(b, name));
        let d = ra.iter().zip(&rb).map(|(x, y)| (x - y).abs() as f64).sum::<f64>() / ra.len().max(1) as f64;
        sum += d * *hands as f64;
        weight += *hands as f64;
    }
    PopulationShift {
        members_changed: pb.iter().filter(|(n, _)| !before.contains(n.as_str())).count(),
        members: pb.len(),
        rate_shift_pp: if weight > 0.0 { 100.0 * sum / weight } else { 0.0 },
    }
}

/// Ranks (1 = smallest) with ties sharing their average rank.
fn ranks(x: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..x.len()).collect();
    idx.sort_by(|&i, &j| x[i].total_cmp(&x[j]));
    let mut r = vec![0.0; x.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && x[idx[j + 1]] == x[idx[i]] {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0 + 1.0;
        for k in &idx[i..=j] {
            r[*k] = avg;
        }
        i = j + 1;
    }
    r
}

/// Spearman rank correlation (Pearson on average ranks); 0 when either side is constant.
pub fn spearman(a: &[f64], b: &[f64]) -> f64 {
    let (ra, rb) = (ranks(a), ranks(b));
    let n = ra.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let (ma, mb) = (ra.iter().sum::<f64>() / n, rb.iter().sum::<f64>() / n);
    let cov: f64 = ra.iter().zip(&rb).map(|(x, y)| (x - ma) * (y - mb)).sum();
    let va: f64 = ra.iter().map(|x| (x - ma).powi(2)).sum();
    let vb: f64 = rb.iter().map(|y| (y - mb).powi(2)).sum();
    if va == 0.0 || vb == 0.0 { 0.0 } else { cov / (va * vb).sqrt() }
}

fn top(x: &[f64], k: usize) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..x.len()).collect();
    idx.sort_by(|&i, &j| x[j].total_cmp(&x[i]).then(i.cmp(&j)));
    idx.truncate(k);
    idx
}

/// How far the candidate ranking moved between two evaluations of the same candidates.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct RankShift {
    /// Spearman correlation of the candidates' measured edges.
    pub spearman: f64,
    /// Candidates in both top fives.
    pub top5_shared: usize,
    /// Whether the best candidate is the same.
    pub same_leader: bool,
    /// Mean absolute change of a candidate's edge, bb/100.
    pub mean_abs_change_bb100: f64,
}

/// Rank shift between edges `a` and `b` (bb per hand, one per candidate, same order).
pub fn rank_shift(a: &[f64], b: &[f64]) -> RankShift {
    let (ta, tb) = (top(a, 5), top(b, 5));
    RankShift {
        spearman: spearman(a, b),
        top5_shared: ta.iter().filter(|i| tb.contains(i)).count(),
        same_leader: ta.first() == tb.first(),
        mean_abs_change_bb100: 100.0 * a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f64>() / a.len().max(1) as f64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spearman_uses_average_ranks_and_handles_constants() {
        assert!((spearman(&[1.0, 2.0, 3.0, 4.0], &[10.0, 20.0, 30.0, 40.0]) - 1.0).abs() < 1e-12);
        assert!((spearman(&[1.0, 2.0, 3.0, 4.0], &[4.0, 3.0, 2.0, 1.0]) + 1.0).abs() < 1e-12);
        assert_eq!(ranks(&[5.0, 1.0, 5.0, 3.0]), vec![3.5, 1.0, 3.5, 2.0]);
        assert_eq!(spearman(&[1.0, 1.0, 1.0], &[1.0, 2.0, 3.0]), 0.0);
        // A monotone transform keeps the ranking.
        assert!((spearman(&[0.1, -0.3, 0.2, 0.05], &[1.1, -3.0, 9.0, 1.0]) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn rank_shift_reports_leader_top_five_and_size_of_the_moves() {
        let a = [0.05, 0.04, 0.03, 0.02, 0.01, 0.0, -0.01];
        let same = rank_shift(&a, &a);
        assert_eq!((same.top5_shared, same.same_leader, same.mean_abs_change_bb100), (5, true, 0.0));
        let mut b = a;
        b.swap(0, 6);
        let moved = rank_shift(&a, &b);
        assert!(!moved.same_leader);
        assert_eq!(moved.top5_shared, 4);
        assert!((moved.mean_abs_change_bb100 - 100.0 * 0.12 / 7.0).abs() < 1e-9);
    }

    fn hand(players: &[&str], acts: usize) -> HandSummary {
        use sv10_core::engine::{ActionKind, ActionRecord, Street};
        let players: Vec<(usize, String)> = players.iter().enumerate().map(|(i, n)| (i, n.to_string())).collect();
        let stacks = players.iter().map(|(s, _)| (*s, 2_000)).collect();
        // The first `acts` seats fold preflop: each observed player gets a hand and a decision.
        let history = (0..acts)
            .map(|seat| ActionRecord {
                seat,
                street: Street::Preflop,
                kind: ActionKind::Fold,
                to: 0,
                pot_before: 30,
                to_call_before: 20,
                bet_before: 0,
                full_raise: false,
                think_ms: None,
                street_open: false,
            })
            .collect();
        HandSummary { players, button: 0, bb: 20, history, board: vec![], shown: vec![], stacks }
    }

    #[test]
    fn replay_counts_a_shared_hand_once_and_never_models_our_seats() {
        let mut r = Replay::new(ModelStore::default(), vec!["us".into(), "us2".into()]);
        let h = hand(&["us", "v1", "v2", "us2"], 3);
        assert!(r.observe("h1", &h));
        assert!(!r.observe("h1", &h), "the second seat's row of the same hand");
        assert!(r.observe("h2", &h));
        assert_eq!(r.models.players.get("v1").map(|s| s.hands), Some(2.0));
        assert!(!r.models.players.contains_key("us"), "the hero is not an opponent");
    }

    #[test]
    fn population_shift_counts_new_members_and_weights_rate_moves_by_hands() {
        let mut r = Replay::new(ModelStore::default(), vec!["us".into()]);
        for i in 0..40 {
            r.observe(&format!("a{i}"), &hand(&["us", "v1", "v2"], 2));
        }
        let before = r.models.clone();
        assert_eq!(population_shift(&before, &before, 30.0, 16), PopulationShift { members_changed: 0, members: 2, rate_shift_pp: 0.0 });
        for i in 0..40 {
            r.observe(&format!("b{i}"), &hand(&["us", "v3", "v1"], 3));
        }
        let after = population_shift(&before, &r.models, 30.0, 16);
        assert_eq!((after.members_changed, after.members), (1, 3), "v3 joined the pool");
        assert!(after.rate_shift_pp >= 0.0);
        // The pool is the most-observed players, capped.
        assert_eq!(pool(&r.models, 30.0, 1)[0].0, "v1");
    }
}

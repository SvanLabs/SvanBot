//! Table simulation: rotate the button, reload stacks every hand (cash-game
//! accounting), feed completed hands back to every agent, tally results.

use crate::agents::Agent;
use std::collections::HashMap;
use sv10_engine::engine::Hand;
use sv10_engine::situation::Situation;
use sv10_model::model::HandSummary;
use sv10_rng::SeedableRng;
use sv10_rng::rngs::SmallRng;

/// Running result of one agent at a simulated table.
#[derive(Clone, Debug, Default)]
pub struct Tally {
    /// Hands played.
    pub hands: u64,
    /// Net chips.
    pub net: f64,
    /// Sum of squared per-hand nets (for the standard error).
    pub sq: f64,
}

impl Tally {
    /// Win rate in big blinds per 100 hands.
    pub fn bb100(&self, bb: f64) -> f64 {
        if self.hands == 0 { 0.0 } else { self.net / bb / self.hands as f64 * 100.0 }
    }
    /// Standard error of bb/100.
    pub fn se100(&self, bb: f64) -> f64 {
        if self.hands < 2 {
            return f64::INFINITY;
        }
        let n = self.hands as f64;
        let mean = self.net / n;
        let var = (self.sq / n - mean * mean).max(0.0);
        (var / n).sqrt() / bb * 100.0
    }
    /// Add another tally.
    pub fn merge(&mut self, o: &Tally) {
        self.hands += o.hands;
        self.net += o.net;
        self.sq += o.sq;
    }
}

/// Play `hands` hands with stacks reloaded to `stack_bb` big blinds each hand; tallies by agent name.
pub fn run_table(agents: &mut [Box<dyn Agent>], hands: usize, stack_bb: i64, seed: u64) -> HashMap<String, Tally> {
    run_table_observed(agents, hands, stack_bb, seed, None)
}

/// `run_table`, also feeding every finished hand to `observer`.
pub fn run_table_observed(
    agents: &mut [Box<dyn Agent>],
    hands: usize,
    stack_bb: i64,
    seed: u64,
    observer: Option<&mut sv10_model::model::ModelStore>,
) -> HashMap<String, Tally> {
    run_table_logged(agents, hands, stack_bb, seed, observer, None)
}

/// Like `run_table_observed`, optionally recording seat 0's net for every hand.
pub fn run_table_logged(
    agents: &mut [Box<dyn Agent>],
    hands: usize,
    stack_bb: i64,
    seed: u64,
    observer: Option<&mut sv10_model::model::ModelStore>,
    per_hand: Option<&mut Vec<f64>>,
) -> HashMap<String, Tally> {
    run_table_salted(agents, hands, stack_bb, seed, 0, observer, per_hand)
}

/// Every seat's decision stream salted (see [`run_table_salted_seats`]).
pub const ALL_SEATS: u64 = u64::MAX;

/// [`run_table_logged`] with the decision streams salted: the same cards (and chance-correction
/// runouts), different decision randomness. Salt 0 is exactly `run_table_logged` (0190).
pub fn run_table_salted(
    agents: &mut [Box<dyn Agent>],
    hands: usize,
    stack_bb: i64,
    seed: u64,
    decision_salt: u64,
    observer: Option<&mut sv10_model::model::ModelStore>,
    per_hand: Option<&mut Vec<f64>>,
) -> HashMap<String, Tally> {
    run_table_salted_seats(agents, hands, stack_bb, seed, decision_salt, ALL_SEATS, observer, per_hand)
}

/// [`run_table_salted`] salting only the seats whose bit is set in `salt_seats` (bit 0 = the hero),
/// to tell apart where decision randomness comes from.
#[allow(clippy::too_many_arguments)]
pub fn run_table_salted_seats(
    agents: &mut [Box<dyn Agent>],
    hands: usize,
    stack_bb: i64,
    seed: u64,
    decision_salt: u64,
    salt_seats: u64,
    mut observer: Option<&mut sv10_model::model::ModelStore>,
    mut per_hand: Option<&mut Vec<f64>>,
) -> HashMap<String, Tally> {
    let (sb, bb) = (10, 20);
    let n = agents.len();
    // Deals and decisions use separate streams so two runs with different agents see identical
    // cards (paired evaluation). Decisions get a fresh stream per hand and seat: one shared stream
    // would desynchronise every later opponent decision after the first hand the two heroes play
    // differently, leaving the runs paired on cards only.
    let mut deal_rng = SmallRng::seed_from_u64(seed);
    let names: Vec<String> = agents.iter().map(|a| a.name().to_string()).collect();
    let mut tallies: HashMap<String, Tally> = HashMap::new();
    for h in 0..hands {
        let stacks = vec![stack_bb * bb; n];
        let mut hand = Hand::new(&stacks, h % n, sb, bb, &mut deal_rng);
        let mut seat_rngs: Vec<SmallRng> = (0..n)
            .map(|i| {
                SmallRng::seed_from_u64(
                    seed ^ 0x9E37_79B9_7F4A_7C15
                        ^ ((h as u64) << 8 | i as u64).wrapping_mul(0xD1B5_4A32_D192_ED03)
                        ^ if salt_seats >> i & 1 == 1 { decision_salt.wrapping_mul(0xA24B_AED4_963E_E407) } else { 0 },
                )
            })
            .collect();
        let mut guard = 0;
        while let Some(seat) = hand.actor() {
            let sit = Situation::from_hand(&hand, seat, &names);
            let action = agents[seat].act(&sit, &mut seat_rngs[seat]);
            if hand.apply(action).is_err() {
                let fallback = if hand.legal().can_check { sv10_engine::engine::Action::Check } else { sv10_engine::engine::Action::Fold };
                hand.apply(fallback).unwrap();
            }
            guard += 1;
            assert!(guard < 500, "runaway hand");
        }
        let summary = HandSummary {
            players: names.iter().cloned().enumerate().collect(),
            button: hand.button,
            bb,
            history: hand.history.clone(),
            board: hand.board.clone(),
            stacks: hand.seats.iter().enumerate().map(|(i, s)| (i, s.start_stack)).collect(),
            shown: if hand.showdown() {
                hand.seats.iter().enumerate().filter(|(_, s)| !s.folded).map(|(i, s)| (i, s.hole)).collect()
            } else {
                vec![]
            },
        };
        for a in agents.iter_mut() {
            a.observe(&summary);
        }
        if let Some(o) = observer.as_deref_mut() {
            o.observe(&summary, None);
        }
        if let Some(log) = per_hand.as_deref_mut() {
            // Luck-reduced outcome for evaluation; its all-in runout sample depends only on the table seed and
            // hand number, so both sides of a paired comparison share it.
            let mut luck_rng = SmallRng::seed_from_u64(seed ^ 0x5851_F42D_4C95_7F2D ^ (h as u64).wrapping_mul(0x2545_F491_4F6C_DD1D));
            // Turn and river card luck removed too (zero-mean chance correction, 0123).
            log.push(hand.expected_net(600, &mut luck_rng)[0] - hand.chance_correction(0));
        }
        for (i, net) in hand.net().into_iter().enumerate() {
            let t = tallies.entry(names[i].clone()).or_default();
            t.hands += 1;
            t.net += net as f64;
            t.sq += (net as f64) * (net as f64);
        }
    }
    tallies
}

/// Challenger-minus-champion result of a paired evaluation.
#[derive(Clone, Debug, serde::Serialize)]
pub struct PairedResult {
    /// Hands compared.
    pub hands: u64,
    /// Challenger minus champion, big blinds per hand.
    pub mean_bb: f64,
    /// Standard error of `mean_bb`.
    pub se_bb: f64,
    /// Hands whose outcome differed between the two policies (0 = the change never mattered).
    pub differing: u64,
}

impl Default for PairedResult {
    /// No hands compared: no estimate, and no interval either (an empty result is not a precise one).
    fn default() -> PairedResult {
        PairedResult { hands: 0, mean_bb: 0.0, se_bb: f64::INFINITY, differing: 0 }
    }
}

impl PairedResult {
    /// Pool two independent evaluations (different deals) of the same pair.
    pub fn combine(&self, other: &PairedResult) -> PairedResult {
        let (n1, n2) = (self.hands as f64, other.hands as f64);
        let n = (n1 + n2).max(1.0);
        let mean = (self.mean_bb * n1 + other.mean_bb * n2) / n;
        let var_mean = (n1 * n1 * self.se_bb * self.se_bb + n2 * n2 * other.se_bb * other.se_bb) / (n * n);
        PairedResult { hands: self.hands + other.hands, mean_bb: mean, se_bb: var_mean.sqrt(), differing: self.differing + other.differing }
    }

    /// Lower end of the 95% interval (the promotion gate).
    pub fn lower_95(&self) -> f64 {
        self.mean_bb - 1.96 * self.se_bb
    }
    /// Upper end of the 95% interval.
    pub fn upper_95(&self) -> f64 {
        self.mean_bb + 1.96 * self.se_bb
    }
}

/// Paired comparison of two policy parameter sets: every table is played twice
/// with identical deals and opponents, once per hero policy, and the per-hand
/// differences are aggregated.
#[allow(clippy::too_many_arguments)]
pub fn paired_eval<O: crate::agents::OpponentSpec>(
    champion: &crate::policy::Params,
    challenger: &crate::policy::Params,
    opponents: &[(O, f64)],
    models: &sv10_model::model::ModelStore,
    nn: Option<std::sync::Arc<sv10_nn::nn::Mlp>>,
    tables: usize,
    hands: usize,
    stack_bb: i64,
    seed: u64,
) -> PairedResult {
    paired_eval_many(champion, std::slice::from_ref(challenger), opponents, models, nn, tables, hands, stack_bb, seed)
        .pop()
        .expect("one challenger")
}

/// [`paired_eval`] for several challengers on the same deals. The champion's side of a table does not
/// depend on the challenger, so it is played once per table and shared; every (challenger, table) run
/// is then scheduled on the pool together. Each result is bit-identical to a separate `paired_eval`.
#[allow(clippy::too_many_arguments)]
pub fn paired_eval_many<O: crate::agents::OpponentSpec>(
    champion: &crate::policy::Params,
    challengers: &[crate::policy::Params],
    opponents: &[(O, f64)],
    models: &sv10_model::model::ModelStore,
    nn: Option<std::sync::Arc<sv10_nn::nn::Mlp>>,
    tables: usize,
    hands: usize,
    stack_bb: i64,
    seed: u64,
) -> Vec<PairedResult> {
    let arms: Vec<Arm<'_>> = challengers.iter().map(|p| Arm { params: p, nn: nn.clone() }).collect();
    paired_eval_arms(&Arm { params: champion, nn: nn.clone() }, &arms, opponents, models, tables, hands, stack_bb, seed)
}

/// One side of a paired evaluation: the policy parameters plus the response model that prices
/// villain replies. Two arms may differ in either, so the same machinery measures a parameter
/// challenger and a candidate response model.
pub struct Arm<'a> {
    /// Policy parameters this side plays.
    pub params: &'a crate::policy::Params,
    /// Response model this side prices villains with, if any.
    pub nn: Option<std::sync::Arc<sv10_nn::nn::Mlp>>,
}

/// [`paired_eval_many`] over explicit arms: the champion arm is played once per table and shared
/// by every challenger arm, exactly as in the parameter search.
#[allow(clippy::too_many_arguments)]
pub fn paired_eval_arms<O: crate::agents::OpponentSpec>(
    champion: &Arm<'_>,
    challengers: &[Arm<'_>],
    opponents: &[(O, f64)],
    models: &sv10_model::model::ModelStore,
    tables: usize,
    hands: usize,
    stack_bb: i64,
    seed: u64,
) -> Vec<PairedResult> {
    let (base, rest) = paired_logs(champion, challengers, opponents, models, 0..tables, hands, stack_bb, seed);
    rest.chunks(tables.max(1))
        .map(|runs| {
            let all: Vec<f64> = base.iter().zip(runs).flat_map(|(a, b)| a.iter().zip(b.iter()).map(|(x, y)| (*y - *x) / 20.0)).collect();
            let n = all.len() as f64;
            let differing = all.iter().filter(|d| d.abs() > 1e-9).count() as u64;
            let mean = all.iter().sum::<f64>() / n.max(1.0);
            let var = all.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
            PairedResult { hands: all.len() as u64, mean_bb: mean, se_bb: (var / n.max(1.0)).sqrt(), differing }
        })
        .collect()
}

/// Running totals of paired per-hand differences (challenger minus champion, big blinds), so one
/// evaluation can be spread over several short steps and pooled exactly (0334).
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PairedSums {
    /// Hands compared.
    pub hands: u64,
    /// Sum of the differences.
    pub sum: f64,
    /// Sum of the squared differences.
    pub sum_sq: f64,
    /// Hands whose outcome differed.
    pub differing: u64,
}

impl PairedSums {
    /// Add another slice of the same evaluation (other tables, same deals rule).
    pub fn add(&mut self, other: &PairedSums) {
        self.hands += other.hands;
        self.sum += other.sum;
        self.sum_sq += other.sum_sq;
        self.differing += other.differing;
    }

    /// The result [`paired_eval_arms`] reports for the same hands.
    pub fn result(&self) -> PairedResult {
        if self.hands == 0 {
            return PairedResult::default();
        }
        let n = self.hands as f64;
        let mean = self.sum / n;
        let var = ((self.sum_sq - n * mean * mean) / (n - 1.0).max(1.0)).max(0.0);
        PairedResult { hands: self.hands, mean_bb: mean, se_bb: (var / n).sqrt(), differing: self.differing }
    }
}

/// [`paired_eval_arms`] over the tables in `tables` only, as running totals: table `t` has the
/// same seating and deals as in a whole evaluation, so slices that cover `0..n` pool to it.
#[allow(clippy::too_many_arguments)]
pub fn paired_sums_arms<O: crate::agents::OpponentSpec>(
    champion: &Arm<'_>,
    challengers: &[Arm<'_>],
    opponents: &[(O, f64)],
    models: &sv10_model::model::ModelStore,
    tables: std::ops::Range<usize>,
    hands: usize,
    stack_bb: i64,
    seed: u64,
) -> Vec<PairedSums> {
    let width = tables.len();
    let (base, rest) = paired_logs(champion, challengers, opponents, models, tables, hands, stack_bb, seed);
    rest.chunks(width.max(1))
        .map(|runs| {
            let mut s = PairedSums::default();
            for d in base.iter().zip(runs).flat_map(|(a, b)| a.iter().zip(b.iter()).map(|(x, y)| (*y - *x) / 20.0)) {
                s.hands += 1;
                s.sum += d;
                s.sum_sq += d * d;
                s.differing += u64::from(d.abs() > 1e-9);
            }
            s
        })
        .collect()
}

/// Per-hand results of every arm on `tables`: the champion's logs, then each challenger's in turn.
#[allow(clippy::too_many_arguments)]
fn paired_logs<O: crate::agents::OpponentSpec>(
    champion: &Arm<'_>,
    challengers: &[Arm<'_>],
    opponents: &[(O, f64)],
    models: &sv10_model::model::ModelStore,
    tables: std::ops::Range<usize>,
    hands: usize,
    stack_bb: i64,
    seed: u64,
) -> (Vec<Vec<f64>>, Vec<Vec<f64>>) {
    use crate::agents::PolicyAgent;
    use rayon::prelude::*;
    use sv10_rng::RngExt;
    let table_seed = |t: usize| seed.wrapping_mul(1_000_003).wrapping_add(t as u64);
    let seating: Vec<Vec<&O>> = tables
        .clone()
        .map(|t| {
            let mut pick_rng = SmallRng::seed_from_u64(table_seed(t) ^ 0xABCDEF);
            let total: f64 = opponents.iter().map(|(_, w)| *w).sum();
            (0..5)
                .map(|_| {
                    let mut x = pick_rng.random::<f64>() * total;
                    for (a, w) in opponents {
                        if x < *w {
                            return a;
                        }
                        x -= w;
                    }
                    &opponents.last().unwrap().0
                })
                .collect()
        })
        .collect();
    let run = |arm: &Arm<'_>, t: usize| -> Vec<f64> {
        let mut agents: Vec<Box<dyn Agent>> = vec![Box::new(PolicyAgent {
            label: "SvanBot".into(),
            models: models.clone(),
            params: arm.params.clone(),
            learn: true,
            nn: arm.nn.clone(),
        })];
        for (i, a) in seating[t - tables.start].iter().enumerate() {
            agents.push(a.agent(i));
        }
        let mut log = Vec::with_capacity(hands);
        run_table_logged(&mut agents, hands, stack_bb, table_seed(t), None, Some(&mut log));
        log
    };
    // Index 0 is the champion; challenger k is index k + 1.
    let heroes: Vec<&Arm<'_>> = std::iter::once(champion).chain(challengers.iter()).collect();
    let jobs: Vec<(usize, usize)> = (0..heroes.len()).flat_map(|k| tables.clone().map(move |t| (k, t))).collect();
    let mut logs: Vec<Vec<f64>> = jobs.par_iter().map(|&(k, t)| run(heroes[k], t)).collect();
    let rest = logs.split_off(tables.len().min(logs.len()));
    (logs, rest)
}

/// Where the paired evaluation's remaining variance lives (0190), in bb² per hand.
#[derive(Clone, Debug, serde::Serialize)]
pub struct VarianceSplit {
    /// Hand cells (table × hand) measured.
    pub cells: u64,
    /// Decision salts replayed per cell.
    pub salts: u64,
    /// Variance of one paired per-hand difference (what the gate's SE is built from).
    pub total: f64,
    /// Mean variance across salts on the same cards: decision randomness, the part AIVAT action
    /// corrections could at best remove.
    pub within: f64,
    /// `total - within`: variance of the card-conditional mean, which no action correction touches.
    pub between: f64,
    /// Mean paired difference over every cell and salt (bb per hand).
    #[serde(default)]
    pub mean: f64,
    /// Standard error of `mean` for one salt's worth of hands (cells treated as independent).
    #[serde(default)]
    pub se: f64,
}

impl VarianceSplit {
    /// Hands the gate would need with a perfect action correction, relative to today (≤ 1).
    pub fn hands_ratio(&self) -> f64 {
        if self.total > 0.0 { (self.between / self.total).clamp(0.0, 1.0) } else { 1.0 }
    }
}

/// Split per-cell differences `d[salt][cell]` into within-cell and between-cell variance.
pub fn split_variance(d: &[Vec<f64>]) -> VarianceSplit {
    let s = d.len();
    let cells = d.first().map_or(0, Vec::len);
    let all: Vec<f64> = d.iter().flatten().copied().collect();
    let n = all.len() as f64;
    let mean = all.iter().sum::<f64>() / n.max(1.0);
    let total = all.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    let within = if s < 2 {
        0.0
    } else {
        (0..cells)
            .map(|c| {
                let m = d.iter().map(|r| r[c]).sum::<f64>() / s as f64;
                d.iter().map(|r| (r[c] - m).powi(2)).sum::<f64>() / (s - 1) as f64
            })
            .sum::<f64>()
            / cells.max(1) as f64
    };
    let se = (total / (cells.max(1) as f64)).sqrt();
    VarianceSplit { cells: cells as u64, salts: s as u64, total, within, between: (total - within).max(0.0), mean, se }
}

/// Replay a paired evaluation `salts` times with the same cards and different decision streams, and
/// split its variance (0190). Salt 0 is the evaluation the learner runs.
#[allow(clippy::too_many_arguments)]
pub fn paired_variance_split<O: crate::agents::OpponentSpec>(
    champion: &crate::policy::Params,
    challenger: &crate::policy::Params,
    opponents: &[(O, f64)],
    models: &sv10_model::model::ModelStore,
    tables: usize,
    hands: usize,
    stack_bb: i64,
    seed: u64,
    salts: usize,
    salt_seats: u64,
    learn: bool,
) -> VarianceSplit {
    use crate::agents::PolicyAgent;
    use rayon::prelude::*;
    use sv10_rng::RngExt;
    let table_seed = |t: usize| seed.wrapping_mul(1_000_003).wrapping_add(t as u64);
    // Same seating rule as `paired_eval_arms`.
    let seating: Vec<Vec<&O>> = (0..tables)
        .map(|t| {
            let mut pick_rng = SmallRng::seed_from_u64(table_seed(t) ^ 0xABCDEF);
            let total: f64 = opponents.iter().map(|(_, w)| *w).sum();
            (0..5)
                .map(|_| {
                    let mut x = pick_rng.random::<f64>() * total;
                    for (a, w) in opponents {
                        if x < *w {
                            return a;
                        }
                        x -= w;
                    }
                    &opponents.last().unwrap().0
                })
                .collect()
        })
        .collect();
    let run = |params: &crate::policy::Params, t: usize, salt: u64| -> Vec<f64> {
        let mut agents: Vec<Box<dyn Agent>> =
            vec![Box::new(PolicyAgent { label: "SvanBot".into(), models: models.clone(), params: params.clone(), learn, nn: None })];
        for (i, a) in seating[t].iter().enumerate() {
            agents.push(a.agent(i));
        }
        let mut log = Vec::with_capacity(hands);
        run_table_salted_seats(&mut agents, hands, stack_bb, table_seed(t), salt, salt_seats, None, Some(&mut log));
        log
    };
    let jobs: Vec<(usize, usize)> = (0..salts).flat_map(|k| (0..tables).map(move |t| (k, t))).collect();
    let diffs: Vec<Vec<f64>> = jobs
        .par_iter()
        .map(|&(k, t)| {
            let (a, b) = (run(champion, t, k as u64), run(challenger, t, k as u64));
            a.iter().zip(&b).map(|(x, y)| (y - x) / 20.0).collect()
        })
        .collect();
    let per_salt: Vec<Vec<f64>> = diffs.chunks(tables.max(1)).map(|c| c.concat()).collect();
    split_variance(&per_salt)
}

#[cfg(test)]
mod tests;

//! Paired logs and resumable sums, sharing deals, seating and starting stacks.
use super::*;

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
    sums(champion, challengers, opponents, models, tables, hands, StackPlan::Uniform(stack_bb), seed)
}

/// Paired evaluation with recorded six-seat chip stacks (the simulator uses a 20-chip big blind).
/// Each absolute table/hand has the same layout, seats and deals in every arm and slice.
#[allow(clippy::too_many_arguments)]
pub fn paired_sums_arms_stacked<O: crate::agents::OpponentSpec>(
    champion: &Arm<'_>,
    challengers: &[Arm<'_>],
    opponents: &[(O, f64)],
    models: &sv10_model::model::ModelStore,
    tables: std::ops::Range<usize>,
    hands: usize,
    layouts: &[[i64; 6]],
    seed: u64,
) -> Vec<PairedSums> {
    assert!(!layouts.is_empty() && layouts.iter().flatten().all(|s| *s > 0), "positive recorded stacks required");
    sums(champion, challengers, opponents, models, tables, hands, StackPlan::Recorded(layouts), seed)
}

#[allow(clippy::too_many_arguments)]
fn sums<O: crate::agents::OpponentSpec>(
    champion: &Arm<'_>,
    challengers: &[Arm<'_>],
    opponents: &[(O, f64)],
    models: &sv10_model::model::ModelStore,
    tables: std::ops::Range<usize>,
    hands: usize,
    plan: StackPlan<'_>,
    seed: u64,
) -> Vec<PairedSums> {
    let width = tables.len();
    let (base, rest) = paired_logs(champion, challengers, opponents, models, tables, hands, plan, seed);
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
pub(super) fn paired_logs<O: crate::agents::OpponentSpec>(
    champion: &Arm<'_>,
    challengers: &[Arm<'_>],
    opponents: &[(O, f64)],
    models: &sv10_model::model::ModelStore,
    tables: std::ops::Range<usize>,
    hands: usize,
    plan: StackPlan<'_>,
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
        run_table_planned(&mut agents, hands, plan, table_seed(t), 0, ALL_SEATS, None, Some(&mut log));
        log
    };
    // Index 0 is the champion; challenger k is index k + 1.
    let heroes: Vec<&Arm<'_>> = std::iter::once(champion).chain(challengers.iter()).collect();
    let jobs: Vec<(usize, usize)> = (0..heroes.len()).flat_map(|k| tables.clone().map(move |t| (k, t))).collect();
    let mut logs: Vec<Vec<f64>> = jobs.par_iter().map(|&(k, t)| run(heroes[k], t)).collect();
    let rest = logs.split_off(tables.len().min(logs.len()));
    (logs, rest)
}

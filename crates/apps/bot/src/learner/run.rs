//! The persisted learner run (0334): an evidence refresh or a champion search is stored after
//! every step, so a release or restart waits at most one step and the next process resumes where
//! the last step stopped. Slices of a paired evaluation cover table ranges, and
//! [`PairedSums`] pools them exactly, so a sliced search measures what an unsliced one did.

use serde::{Deserialize, Serialize};
use std::ops::Range;
use sv10_core::policy::Params;
use sv10_core::sim::PairedSums;
use sv10_store::store::Store;

use crate::search_ledger::LedgerEntry;

/// KV key holding the run in progress, if any.
pub const RUN_KEY: &str = "learner.run.v1";
/// Simulation seconds a step plans for.
pub const STEP_TARGET_SECS: f64 = 90.0;
/// Simulation seconds one *slice* of a step plans for (0343). A step is a sequence of slices, and a
/// slice is a range of tables, so its size changes nothing but the machine's exposure: the rate a
/// slice is planned from was measured before it, and load that arrives in between is invisible to
/// the plan. Capping the slice at 40 s of simulation means a rate that has fallen by half costs
/// 80 s — inside the 120 s job budget — instead of turning a 90 s plan into 161 s.
pub const SLICE_TARGET_SECS: f64 = 40.0;
/// Simulation seconds for the first slice a process plays (0343). A rate persisted by an earlier
/// process was measured under load that may not hold now — a run stored on a quiet night and
/// resumed onto a busy day, or resumed at a release — and nothing re-measures it before the first
/// slice commits. 10 s of plan survives the rate being out by five.
pub const RESUME_SLICE_SECS: f64 = 10.0;

/// Table runs (one arm playing one table of the learner's hands) per second, as measured.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rate(pub f64);

impl Rate {
    /// Before any measurement: 0.08 table runs of 2,000 hands per second per thread, below the
    /// 0.10–0.11 the i7-4770K measured with eight threads, so a first step errs short.
    pub fn guess(threads: usize, hands: usize) -> Rate {
        Rate(0.08 * threads.max(1) as f64 * 2_000.0 / hands.max(1) as f64)
    }

    /// Blend in one measured slice; the latest slice weighs half.
    pub fn observe(&mut self, runs: usize, secs: f64) {
        if runs > 0 && secs > 0.5 {
            self.0 = 0.5 * self.0 + 0.5 * runs as f64 / secs;
        }
    }

    /// Table runs that fit in `secs` (at least one: every step makes progress).
    pub fn runs_in(&self, secs: f64) -> usize {
        ((self.0 * secs).floor() as usize).max(1)
    }

    /// Seconds `runs` table runs are expected to take.
    pub fn secs_for(&self, runs: usize) -> f64 {
        runs as f64 / self.0.max(1e-6)
    }

    /// Table runs for the next slice: what the rate affords in the step's remaining `left` seconds,
    /// capped at `cap` seconds of simulation (0343). The cap bounds the *plan*; a rate that has
    /// fallen since it was measured costs the planned seconds divided by that error, which is why
    /// the cap is a third of the job budget.
    pub fn slice_runs(&self, left: f64, cap: f64) -> usize {
        self.runs_in(left.min(cap))
    }
}

/// The next slice of `0..total` tables for `arms` arms (the champion counts) starting at `next`:
/// the remaining tables split into equal slices that each fit `budget` table runs.
pub fn table_slice(next: usize, total: usize, arms: usize, budget: usize) -> Range<usize> {
    let remaining = total.saturating_sub(next);
    let cap = (budget / arms.max(1)).max(1);
    let width = remaining.div_ceil(remaining.div_ceil(cap).max(1));
    next..(next + width).min(total)
}

/// Where a halving round stands: candidates before `next_cand` are done; the batch
/// `next_cand..batch_end` has played tables before `next_table`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoundCursor {
    /// First candidate of the batch in progress.
    pub next_cand: usize,
    /// End of the batch in progress (fixed once its first tables are played).
    pub batch_end: usize,
    /// Next table of the batch in progress.
    pub next_table: usize,
}

impl RoundCursor {
    /// The next slice of a round over `pool` candidates on `round_tables` tables: a batch of
    /// candidates sharing the champion's tables, sized to `budget` table runs.
    pub fn slice(&self, pool: usize, round_tables: usize, budget: usize) -> (Range<usize>, Range<usize>) {
        let end = if self.next_table == 0 {
            let left = pool.saturating_sub(self.next_cand);
            let per = budget.saturating_sub(1).max(1);
            self.next_cand + left.div_ceil(left.div_ceil(per).max(1))
        } else {
            self.batch_end
        };
        (self.next_cand..end, table_slice(self.next_table, round_tables, end - self.next_cand + 1, budget))
    }

    /// Mark a slice played.
    pub fn advance(&mut self, cands: &Range<usize>, tables: &Range<usize>, round_tables: usize) {
        self.batch_end = cands.end;
        self.next_table = tables.end;
        if self.next_table >= round_tables {
            self.next_table = 0;
            self.next_cand = cands.end;
        }
    }

    /// Whether every candidate played every table of the round.
    pub fn done(&self, pool: usize) -> bool {
        self.next_cand >= pool
    }
}

/// The run in progress.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Run {
    /// An evidence refresh.
    Refit(RefitRun),
    /// A champion search.
    Search(Box<SearchRun>),
    /// A paired round-robin between the lineages' champions.
    Tournament(Box<super::tournament::TournamentRun>),
}

/// An evidence refresh in three steps: the range model (daily), the live fits, then the
/// per-opponent fits and the population snapshot.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RefitRun {
    /// Newest hand row when it started (the pacing watermark it records).
    pub start_rowid: i64,
    /// Start time (seconds since the epoch).
    pub started: f64,
    /// Steps finished.
    pub done: u8,
}

/// A champion search: what every step must see unchanged, and where it stands.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchRun {
    /// Frozen paired evaluation objective; absent legacy objectives are explicitly retired.
    #[serde(default)]
    pub stacks: super::stacks::Fixture,
    /// Learner cycle (seeds its clones and deals).
    pub cycle: u64,
    /// Newest hand row when it started.
    pub start_rowid: i64,
    /// Start time (seconds since the epoch).
    pub started: f64,
    /// The lineage this search is for (the shared champion in a store that has no lineages).
    #[serde(default)]
    pub lane: super::lane::Lane,
    /// Changes other lineages promoted, taken from the queue when this search began.
    #[serde(default)]
    pub transfers: Vec<super::transfer::Transfer>,
    /// Champion version searched against.
    pub champion_version: String,
    /// Evidence epoch the search is measured under (the ledger's and the target queue's scope).
    pub refit_rowid: i64,
    /// Digest of the champion's search parameters; a change abandons the run.
    pub champion_digest: String,
    /// Digest of the population models the clones imitate; a change abandons the run.
    pub models_digest: String,
    /// Population id for the experiment records.
    pub population_id: String,
    /// Observed hands behind the clone pool.
    pub evidence: f32,
    /// Measured table runs per second.
    pub rate: Rate,
    /// Steps taken so far.
    pub steps: u32,
    /// The per-opponent fits wait for the newly approved response model.
    pub players_refit_due: bool,
    /// Transitions measured by this search, folded into the ledger when the search phase ends.
    pub ledger_done: Vec<(String, LedgerEntry)>,
    /// Where it stands.
    pub stage: Stage,
}

/// The stage a search is in.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum Stage {
    /// The fresh-deal poker gate for a response model trained this cycle.
    Gate(Gate),
    /// Successive halving on shared deals.
    Halving(Halving),
    /// Sequential fresh-deal confirmation of the survivor.
    Confirm(Box<Confirm>),
}

/// The response model's poker gate.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Gate {
    /// KV key holding the candidate net.
    pub key: String,
    /// The candidate's training time (identifies it).
    pub trained_at: f64,
    /// Next table to play.
    pub next_table: usize,
    /// Candidate minus incumbent so far.
    pub sums: PairedSums,
}

/// One halving candidate.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cand {
    /// Index into this cycle's one-knob proposals.
    pub index: usize,
    /// Knob moved.
    pub knob: String,
    /// Champion value.
    pub old: f64,
    /// Challenger value.
    pub new: f64,
    /// Earlier rounds and the ledger's stored interval, pooled.
    pub prior: Option<LedgerEntry>,
    /// This round's tables so far.
    pub round: PairedSums,
}

/// Successive halving.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Halving {
    /// Round number (seeds its deals).
    pub round: u64,
    /// Tables per candidate this round.
    pub round_tables: usize,
    /// Candidate hands spent (the search budget).
    pub spent: usize,
    /// Candidates still in.
    pub pool: Vec<Cand>,
    /// Progress through this round.
    pub cursor: RoundCursor,
}

/// Fresh-deal confirmation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Confirm {
    /// Knob moved.
    pub knob: String,
    /// Champion value.
    pub old: f64,
    /// Challenger value.
    pub new: f64,
    /// Challenger parameters.
    pub params: Params,
    /// The search interval that nominated it (reported, never gating).
    pub search: LedgerEntry,
    /// Chunk in progress (1-based, as `promotion::verdict` counts).
    pub chunk: usize,
    /// Next table of the chunk.
    pub next_table: usize,
    /// This chunk so far.
    pub part: PairedSums,
    /// Completed chunks, pooled.
    pub so_far: Option<LedgerEntry>,
}

/// The stored run, if any; an unreadable one (an older format) is dropped with a warning.
pub fn load(store: &Store) -> Option<Run> {
    let json = store.get_kv(RUN_KEY).ok().flatten().filter(|s| !s.is_empty())?;
    match serde_json::from_str(&json) {
        Ok(run) => Some(run),
        Err(e) => {
            tracing::warn!("learner run not readable ({e}); starting over");
            clear(store);
            None
        }
    }
}

/// Store the run after a step.
pub fn save(store: &Store, run: &Run) -> anyhow::Result<()> {
    store.put_kv(RUN_KEY, &serde_json::to_string(run)?)
}

/// The run is finished (or abandoned).
pub fn clear(store: &Store) {
    if let Err(e) = store.put_kv(RUN_KEY, "") {
        tracing::warn!("learner run not cleared ({e})");
    }
}

/// Short digest of a JSON value, to notice a champion or population change between steps.
pub fn digest(json: &str) -> String {
    sv10_digest::hex(sv10_digest::Sha256::digest(json.as_bytes()))[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_slices_are_balanced_and_fit_the_budget() {
        // A 48-table confirmation chunk (two arms) with room for 72 runs: two slices of 24.
        assert_eq!(table_slice(0, 48, 2, 72), 0..24);
        assert_eq!(table_slice(24, 48, 2, 72), 24..48);
        // Everything fits: one slice.
        assert_eq!(table_slice(0, 8, 2, 72), 0..8);
        // A budget below one table per arm still plays a table.
        assert_eq!(table_slice(3, 10, 40, 8), 3..4);
        assert!(table_slice(10, 10, 2, 72).is_empty());
    }

    #[test]
    fn a_round_walks_candidate_batches_and_table_slices_to_the_end() {
        let walk = |pool: usize, tables: usize, budget: usize| {
            let mut c = RoundCursor::default();
            let mut played = vec![vec![0u32; tables]; pool];
            let mut slices = 0;
            while !c.done(pool) {
                let (cands, t) = c.slice(pool, tables, budget);
                assert!(!cands.is_empty() && !t.is_empty());
                assert!((cands.len() + 1) * t.len() <= budget.max(cands.len() + 1), "{cands:?} {t:?}");
                for k in cands.clone() {
                    for x in t.clone() {
                        played[k][x] += 1;
                    }
                }
                c.advance(&cands, &t, tables);
                slices += 1;
            }
            assert!(played.iter().flatten().all(|n| *n == 1), "every (candidate, table) exactly once");
            slices
        };
        // r0 of a 38-candidate field on one table fits one 72-run step.
        assert_eq!(walk(38, 1, 72), 1);
        // With four threads (36 runs) it splits into two balanced batches of 19.
        assert_eq!(walk(38, 1, 36), 2);
        assert_eq!(RoundCursor::default().slice(38, 1, 36).0, 0..19);
        // A late round: two candidates on 16 tables at 36 runs: two balanced slices of 8 tables.
        assert_eq!(walk(2, 16, 36), 2);
        assert_eq!(walk(1, 1, 1), 1);
    }

    #[test]
    fn a_round_covers_every_table_whatever_the_slice_size() {
        // The live loop re-plans the slice budget on every iteration (0343), so one round is walked
        // with a different budget each time; every (candidate, table) is still played exactly once,
        // which is what makes the cap free. `PairedSums` pools the slices exactly.
        for budget in [72, 40, 32, 10, 3, 1] {
            let (pool, tables) = (9usize, 7usize);
            let mut c = RoundCursor::default();
            let mut played = vec![vec![0u32; tables]; pool];
            let mut slices = 0;
            while !c.done(pool) {
                let (cands, t) = c.slice(pool, tables, budget);
                assert!(!cands.is_empty() && !t.is_empty(), "budget {budget}");
                for k in cands.clone() {
                    for x in t.clone() {
                        played[k][x] += 1;
                    }
                }
                c.advance(&cands, &t, tables);
                slices += 1;
                assert!(slices < 10_000, "budget {budget} walks");
            }
            assert!(played.iter().flatten().all(|n| *n == 1), "budget {budget}");
        }
    }

    #[test]
    fn a_slice_is_planned_from_the_live_rate_and_capped() {
        // Nothing in the slice budget is frozen at the top of a step (0343): it is what the rate
        // affords in the time the step has left, capped at SLICE_TARGET_SECS.
        assert_eq!(Rate(0.8).slice_runs(STEP_TARGET_SECS, SLICE_TARGET_SECS), 32);
        // The step's remaining time binds when it is below the cap.
        assert_eq!(Rate(0.8).slice_runs(12.0, SLICE_TARGET_SECS), 9);
        // The resume cap binds even with the whole step ahead.
        assert_eq!(Rate(0.8).slice_runs(STEP_TARGET_SECS, RESUME_SLICE_SECS), 8);
        // Every slice plays at least one table run, however stale the rate.
        assert_eq!(Rate(1e-9).slice_runs(STEP_TARGET_SECS, RESUME_SLICE_SECS), 1);
        // The cap is what bounds the damage a stale rate can do: a slice planned at
        // SLICE_TARGET_SECS costs 80 s when the rate has really fallen by half, against the 120 s
        // job budget — where the same fall against a 90 s plan cost 161 s.
        let stale = Rate(0.8);
        let runs = stale.slice_runs(STEP_TARGET_SECS, SLICE_TARGET_SECS);
        assert!((2.0 * stale.secs_for(runs) - 80.0).abs() < 1e-9);
        assert!(2.0 * Rate(0.8).secs_for(Rate(0.8).slice_runs(STEP_TARGET_SECS, STEP_TARGET_SECS)) > 120.0);
    }

    #[test]
    fn the_rate_learns_from_slices_and_sizes_steps() {
        let mut r = Rate::guess(8, 2_000);
        assert!((r.0 - 0.64).abs() < 1e-9);
        r.observe(96, 110.0);
        assert!(r.0 > 0.64 && r.0 < 0.873);
        assert_eq!(Rate(0.8).runs_in(STEP_TARGET_SECS), 72);
        assert_eq!(Rate(0.001).runs_in(STEP_TARGET_SECS), 1);
        assert!((Rate(0.8).secs_for(72) - 90.0).abs() < 1e-9);
    }

    #[test]
    fn a_run_survives_its_store_round_trip() {
        let run = Run::Search(Box::new(SearchRun {
            stacks: super::super::stacks::Fixture::default(),
            cycle: 7,
            start_rowid: 1,
            started: 2.0,
            lane: super::super::lane::Lane::bot("A"),
            transfers: vec![],
            champion_version: "sv10-ev-9".into(),
            refit_rowid: 3,
            champion_digest: digest("{}"),
            models_digest: digest("[]"),
            population_id: "pop".into(),
            evidence: 1.5,
            rate: Rate(0.7),
            steps: 2,
            players_refit_due: false,
            ledger_done: vec![("k|1|2".into(), LedgerEntry { hands: 5, mean_bb: 0.1, se_bb: 0.2, differing: 1 })],
            stage: Stage::Confirm(Box::new(Confirm {
                knob: "fold_scale".into(),
                old: 0.8,
                new: 0.9,
                params: Params::default(),
                search: LedgerEntry::default(),
                chunk: 2,
                next_table: 24,
                part: PairedSums { hands: 10, sum: 1.0, sum_sq: 2.0, differing: 3 },
                so_far: None,
            })),
        }));
        let back: Run = serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
        match back {
            Run::Search(s) => match s.stage {
                Stage::Confirm(c) => assert_eq!((s.cycle, c.chunk, c.next_table, c.part.differing), (7, 2, 24, 3)),
                _ => panic!("stage lost"),
            },
            _ => panic!("kind lost"),
        }
    }
}

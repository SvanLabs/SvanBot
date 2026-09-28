//! The champion search in steps (0334). [`begin`] trains (or reuses) the response model and
//! picks the first stage; every [`step`] then plays slices of the current stage (the response
//! model's poker gate, a successive-halving round, a fresh-deal confirmation chunk) until its
//! ~90 s are used, and stores where it stands. Deals, seeds, looks and verdicts are those of the
//! one-piece search: a slice covers a range of tables and [`PairedSums`] pools slices exactly.

mod conclude;
mod stages;
#[cfg(test)]
mod tests;

use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Instant;
use sv10_core::agents::{ProfileClone, live_pool};
use sv10_core::model::ModelStore;
use sv10_core::nn::Mlp;
use sv10_core::policy::Params;
use sv10_core::sim::{Arm, PairedSums, paired_sums_arms};
use sv10_store::store::Store;

use super::run::{self, Gate, Rate, SLICE_TARGET_SECS, STEP_TARGET_SECS, SearchRun, Stage, table_slice};
use super::{Ctx, MIN_OPPONENT_HANDS, NN_REUSE_SECS, POPULATION_MODELS_KEY, load_lineage, load_models, load_params, now, status};
use crate::promotion::CHUNK_SCALE;
use crate::{NN_CANDIDATE_KEY, NN_KEY, StoredNet, neural};

/// How a step ended.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// More steps to come.
    Continue,
    /// The search is over.
    Finished {
        /// A challenger was promoted.
        promoted: bool,
    },
    /// The champion or the population changed under the search; it starts over.
    Abandoned(&'static str),
}

/// The champion and population a search runs against, rebuilt every step and checked against
/// the digests taken when it began.
struct Scope {
    champion: Params,
    champion_json: String,
    models: ModelStore,
    models_json: String,
}

impl Scope {
    fn load(store: &Store) -> Scope {
        let models_json = store.get_kv(POPULATION_MODELS_KEY).ok().flatten().unwrap_or_default();
        let models = serde_json::from_str(&models_json).unwrap_or_else(|_| load_models(store));
        let mut champion = load_params(store);
        // Live fits correct live play only; challengers are compared without them (SPEC-learner).
        crate::livefits::LiveFits::NONE.apply(&mut champion);
        // Challengers inherit the showdown-fitted range model, so evaluations match live play.
        champion.range = crate::fitted_range_params(store.get_kv(crate::RANGE_PARAMS_KEY).ok().flatten().as_deref()).unwrap_or_default();
        let champion_json = serde_json::to_string(&champion).unwrap_or_default();
        Scope { champion, champion_json, models, models_json }
    }
}

/// What a search plays: the champion (search parameters), the population snapshot and the live
/// response net. `bench` freezes it into its fixture (0335).
pub fn inputs(store: &Store) -> (Params, ModelStore, Option<Arc<Mlp>>) {
    let sc = Scope::load(store);
    let nn = neural::active_response_net(store.get_kv(NN_KEY).ok().flatten().and_then(|j| serde_json::from_str::<StoredNet>(&j).ok()));
    (sc.champion, sc.models, nn)
}

/// Everything a stage needs within one step.
struct Env<'a> {
    ctx: &'a Ctx<'a>,
    sc: &'a Scope,
    clones: Vec<(ProfileClone, f64)>,
    /// The response model both arms price villains with (the live one).
    nn: Option<Arc<Mlp>>,
    /// The champion as the simulation plays it.
    eval_champion: Params,
    /// This cycle's one-knob proposals (candidates refer to them by index).
    proposals: Vec<(String, f64, f64, Params)>,
    lineage: Vec<String>,
}

impl Env<'_> {
    /// Parameters as the simulation plays them.
    fn eval(&self, p: &Params) -> Params {
        let mut e = p.clone();
        e.samples = self.ctx.decision_samples;
        e.deal_chunks = 1;
        e
    }

    fn status(&self, mut v: Value) {
        if let Some(o) = v.as_object_mut() {
            o.insert("lineage".into(), json!(self.lineage));
            o.insert("automatic".into(), json!(true));
        }
        status(self.ctx.store, v);
    }
}

/// What a stage did with the step's remaining time.
enum Flow {
    /// Played a slice.
    Played,
    /// Moved on without simulating (a round or chunk judged).
    Moved,
    /// Change stage.
    Next(Stage),
    /// Change stage and end the step: the live response model may have changed, and the next
    /// step prices with it (the one-piece search switched nets right after the gate too).
    Handover(Stage),
    /// No room left for the next slice in this step.
    Stop,
    /// The search is over.
    Done { promoted: bool },
}

/// Start a champion search: train (or reuse) the response model and set up the first stage.
/// `None` when fewer than four opponents have enough hands (the caller waits and retries).
pub fn begin(ctx: &Ctx, start_rowid: i64, started: f64, refit_rowid: i64) -> anyhow::Result<Option<SearchRun>> {
    let store = ctx.store;
    // What the ledger and the target queue are scoped to is the evidence epoch, not the refresh
    // watermark: refreshes run on the hands as they arrive (#314), and a scope that moved with each
    // one would retire the search's memory of what it has already measured (0285).
    let refit_rowid = crate::pacing::evidence_epoch(refit_rowid);
    let cycle = store.get_kv(crate::LEARNER_CYCLE_KEY).ok().flatten().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0) + 1;
    let _ = store.put_kv(crate::LEARNER_CYCLE_KEY, &cycle.to_string());
    // One evidence snapshot per search (0244): every step reads the same population.
    if store.get_kv(POPULATION_MODELS_KEY).ok().flatten().is_none() {
        store.put_kv(POPULATION_MODELS_KEY, &serde_json::to_string(&load_models(store))?)?;
    }
    let sc = Scope::load(store);
    let lineage = load_lineage(store);
    status(
        store,
        json!({"status": "training", "phase": "training neural response model", "automatic": true, "lineage": lineage,
        "progress": {"hands": 0, "target": ctx.tables * ctx.hands}}),
    );
    let stored_nn = store.get_kv(NN_KEY).ok().flatten().and_then(|j| serde_json::from_str::<StoredNet>(&j).ok());
    // A net trained minutes ago (a follow-up cycle after a promotion) has seen the same hands; reuse it.
    let fresh = stored_nn.as_ref().filter(|s| neural::has_current_training_contract(s) && now() - s.trained_at < NN_REUSE_SECS);
    let trained = if let Some(s) = fresh {
        tracing::info!("cycle {cycle}: neural response model trained {:.0} min ago; reusing it", (now() - s.trained_at) / 60.0);
        None
    } else {
        let t = Instant::now();
        let trained = neural::train_response_model(store, &ctx.root.join("artifacts"), &sc.models, cycle, &[]);
        tracing::info!("cycle {cycle}: neural response model trained in {:.0}s", t.elapsed().as_secs_f64());
        trained
    };
    // The live net keeps playing while a fresh candidate awaits the poker gate (0142).
    let candidate: Option<(StoredNet, &str)> = match trained {
        Some(t) => {
            let key = match neural::slot_for_trained(stored_nn.as_ref(), &t) {
                neural::NetSlot::Live => NN_KEY,
                neural::NetSlot::Candidate => NN_CANDIDATE_KEY,
            };
            store.put_kv(key, &serde_json::to_string(&t)?)?;
            Some((t, key))
        }
        None => stored_nn.map(|s| (s, NN_KEY)),
    };
    let mut pool: Vec<f32> = sc.models.players.values().filter(|s| s.hands >= MIN_OPPONENT_HANDS).map(|s| s.hands).collect();
    pool.sort_by(|a, b| b.total_cmp(a));
    pool.truncate(16);
    if pool.len() < 4 {
        status(
            store,
            json!({"status": "idle", "phase": "collecting", "automatic": true, "last_error": Value::Null, "next_run": now() + 300.0,
            "progress": {"hands": 0, "target": ctx.tables * ctx.hands}, "lineage": lineage,
            "next_candidate": {"family": "population fit", "reason": format!("waiting for at least 4 opponents with {MIN_OPPONENT_HANDS}+ observed hands ({} so far)", pool.len())}}),
        );
        tracing::info!("cycle {cycle}: only {} opponents with enough hands; waiting", pool.len());
        return Ok(None);
    }
    let mut run = SearchRun {
        cycle,
        start_rowid,
        started,
        champion_version: lineage.last().cloned().unwrap_or_default(),
        refit_rowid,
        champion_digest: run::digest(&sc.champion_json),
        models_digest: run::digest(&sc.models_json),
        population_id: format!("pop-c{cycle}-{}", pool.len()),
        evidence: pool.iter().sum(),
        rate: Rate::guess(ctx.threads, ctx.hands),
        steps: 0,
        players_refit_due: false,
        ledger_done: Vec::new(),
        stage: Stage::Gate(Gate { key: String::new(), trained_at: 0.0, next_table: 0, sums: PairedSums::default() }),
    };
    run.stage = match candidate.filter(|(c, _)| neural::awaits_poker_approval(c)) {
        Some((c, key)) => Stage::Gate(Gate { key: key.to_string(), trained_at: c.trained_at, next_table: 0, sums: PairedSums::default() }),
        None => {
            let env = env(ctx, &sc, &run, lineage);
            stages::start_halving(&env, &mut run)
        }
    };
    Ok(Some(run))
}

fn env<'a>(ctx: &'a Ctx<'a>, sc: &'a Scope, run: &SearchRun, lineage: Vec<String>) -> Env<'a> {
    // Clones reproduce each opponent's learned profile (0099); their rates are redrawn every cycle
    // within their sampling error, from the cycle's seed, so every step draws the same clones.
    let clones = live_pool(&sc.models, MIN_OPPONENT_HANDS, 16, 7_000 + run.cycle);
    let nn = neural::active_response_net(ctx.store.get_kv(NN_KEY).ok().flatten().and_then(|j| serde_json::from_str::<StoredNet>(&j).ok()));
    let mut env = Env {
        ctx,
        sc,
        clones,
        nn,
        eval_champion: sc.champion.clone(),
        proposals: super::pool::challengers(&sc.champion, run.cycle),
        lineage,
    };
    env.eval_champion = env.eval(&sc.champion);
    env
}

/// Advance the search by about [`STEP_TARGET_SECS`] of simulation and say where it stands. Each
/// slice is planned from the rate as it stands at that moment, capped at `first_cap` for the step's
/// first slice and at [`SLICE_TARGET_SECS`] after it (0343).
pub fn step(ctx: &Ctx, run: &mut SearchRun, first_cap: f64) -> anyhow::Result<Outcome> {
    let sc = Scope::load(ctx.store);
    if run::digest(&sc.champion_json) != run.champion_digest {
        return Ok(Outcome::Abandoned("the champion's parameters changed"));
    }
    if run::digest(&sc.models_json) != run.models_digest {
        return Ok(Outcome::Abandoned("the population models changed"));
    }
    let t0 = Instant::now();
    run.steps += 1;
    let env = env(ctx, &sc, run, load_lineage(ctx.store));
    let mut did = false;
    // The step plans each slice as it comes, never once at the top: the rate a slice is sized from
    // was measured before it, and load that arrives in between is invisible to a plan made earlier
    // (0343). The first slice of the step is capped at `first_cap`, which is smaller when this
    // process has not measured the rate yet; playing a slice widens it.
    let mut cap = first_cap;
    let outcome = loop {
        let left = STEP_TARGET_SECS - t0.elapsed().as_secs_f64();
        if run.players_refit_due {
            // The per-opponent correction is fitted against one network; refit it for the new one.
            if did && left < 30.0 {
                break Outcome::Continue;
            }
            crate::playerfits::refit(ctx.store, &ctx.root.join("artifacts"));
            run.players_refit_due = false;
            did = true;
            continue;
        }
        let flow = match &run.stage {
            Stage::Gate(_) => gate(&env, run, left, did, cap)?,
            Stage::Halving(_) => stages::halving(&env, run, left, did, cap)?,
            Stage::Confirm(_) => stages::confirm(&env, run, left, did, cap)?,
        };
        match flow {
            Flow::Played => {
                did = true;
                cap = SLICE_TARGET_SECS;
            }
            Flow::Moved => {}
            Flow::Next(stage) => run.stage = stage,
            Flow::Handover(stage) => {
                run.stage = stage;
                break Outcome::Continue;
            }
            Flow::Stop => break Outcome::Continue,
            Flow::Done { promoted } => break Outcome::Finished { promoted },
        }
    };
    super::log_step(&format!("learner cycle {} step {}", run.cycle, run.steps), t0.elapsed());
    Ok(outcome)
}

/// The response model's fresh-deal poker gate (`neural::poker_verdict`): the incumbent exposure
/// plays the candidate net on identical deals, and the candidate ships unless significantly worse.
fn gate(e: &Env, run: &mut SearchRun, left: f64, did: bool, cap: f64) -> anyhow::Result<Flow> {
    let total = e.ctx.tables * CHUNK_SCALE;
    let Stage::Gate(g) = &mut run.stage else { unreachable!("gate stage") };
    let cand = e
        .ctx
        .store
        .get_kv(&g.key)
        .ok()
        .flatten()
        .and_then(|j| serde_json::from_str::<StoredNet>(&j).ok())
        .filter(|c| c.trained_at == g.trained_at && neural::awaits_poker_approval(c));
    let Some(mut cand) = cand else {
        tracing::warn!("cycle {}: the response model awaiting the poker gate is gone; searching without it", run.cycle);
        return Ok(Flow::Next(stages::start_halving(e, run)));
    };
    if g.next_table >= total {
        let r = g.sums.result();
        let summary = format!(
            "{:+.2} bb/100 (95% {:+.2}..{:+.2}) over {} hands",
            r.mean_bb * 100.0,
            r.lower_95() * 100.0,
            r.upper_95() * 100.0,
            r.hands
        );
        match neural::poker_verdict(&r) {
            neural::PokerVerdict::Approve => {
                cand.paired_poker_approved = true;
                e.ctx.store.put_kv(NN_KEY, &serde_json::to_string(&cand)?)?;
                tracing::info!("cycle {}: neural model approved by the paired poker gate: {summary}", run.cycle);
                run.players_refit_due = true;
            }
            neural::PokerVerdict::Reject(why) => {
                tracing::warn!("cycle {}: neural model stays unexposed ({why}); the live net keeps playing: {summary}", run.cycle);
            }
        }
        return Ok(Flow::Handover(stages::start_halving(e, run)));
    }
    let t = table_slice(g.next_table, total, 2, run.rate.slice_runs(left, cap));
    if did && run.rate.secs_for(2 * t.len()) > left {
        return Ok(Flow::Stop);
    }
    e.status(json!({"status": "evaluating", "phase": "paired poker gate for the neural response model",
        "progress": {"hands": g.next_table * e.ctx.hands, "target": total * e.ctx.hands}}));
    let s = Instant::now();
    let net = Arc::new(cand.net);
    let part = paired_sums_arms(
        &Arm { params: &e.eval_champion, nn: e.nn.clone() },
        &[Arm { params: &e.eval_champion, nn: Some(net) }],
        &e.clones,
        &e.sc.models,
        t.clone(),
        e.ctx.hands,
        100,
        700_000 + run.cycle * 10_000,
    )
    .pop()
    .expect("one arm");
    g.sums.add(&part);
    g.next_table = t.end;
    run.rate.observe(2 * t.len(), s.elapsed().as_secs_f64());
    Ok(Flow::Played)
}

//! Successive halving and fresh-deal confirmation, one slice at a time.

use serde_json::json;
use std::time::Instant;
use sv10_core::sim::{Arm, PairedResult, PairedSums, paired_sums_arms};

use super::super::run::{Cand, Confirm, Halving, SearchRun, Stage, table_slice};
use super::super::{funnel, now, publish_targets, push_experiment};
use super::{Env, Flow};
use crate::experiment::target::{self as experiment_target, Source, TARGETS_KEY, Target, TargetQueue};
use crate::promotion::{self, CHUNK_SCALE, CONFIRM_CHUNKS, MIN_EDGE_BB, Verdict};
use crate::search_ledger::{self, LedgerEntry, transition_key};

/// Set up successive halving on common deals: every candidate gets a small screening budget, the
/// better half doubles its budget each round, candidates that never change an outcome or whose
/// upper bound is below +1 bb/100 are dropped. The survivor is then confirmed sequentially on
/// fresh deals (`crate::promotion`), which alone decides.
pub(super) fn start_halving(e: &Env, run: &mut SearchRun) -> Stage {
    let store = e.ctx.store;
    let total = e.proposals.len();
    // The rejection ledger (0285): repeats continue their stored interval instead of restarting
    // it, and decided-dead transitions are not proposed while this champion and evidence stand.
    let ledger = search_ledger::load(store, &run.champion_version, run.refit_rowid);
    let fresh = e.proposals.iter().map(|(k, o, n, p)| (k.clone(), *o, *n, p.clone(), None)).collect();
    let (kept, barred) = search_ledger::seed_and_filter(fresh, &ledger, e.ctx.tables, e.ctx.hands);
    if barred > 0 {
        tracing::info!(
            "cycle {}: rejection ledger bars {barred} of {total} decided-dead transitions ({} @ {})",
            run.cycle,
            run.champion_version,
            run.refit_rowid
        );
        funnel::note(store, funnel::BARRED, None, barred as u32);
    }
    // The denominator is the pool offered after the ledger filter, including when live support
    // bypasses halving. The ledger bar above is the other half of what the cycle was offered.
    funnel::note(store, funnel::PROPOSED, None, kept.len() as u32);
    // A challenger the experiment pair supported live (0291) goes straight to fresh-deal
    // confirmation: live evidence only prioritizes, the confirmation gate alone promotes.
    let published: Option<TargetQueue> = store.get_kv(TARGETS_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok());
    let verdicts: crate::experiment::Verdicts =
        store.get_kv(crate::experiment::VERDICTS_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    if let Some(t) = experiment_target::live_supported(published.as_ref(), &verdicts, &ledger) {
        tracing::info!("cycle {}: {} was supported live; confirming it on fresh deals instead of searching", run.cycle, t.label());
        let confirming = Target::new(
            (&run.champion_version, run.refit_rowid),
            &t.knob,
            t.old,
            t.new,
            t.challenger.clone(),
            t.sim.clone(),
            Source::Confirmation,
        );
        publish_targets(store, &ledger, &e.sc.champion, run.cycle, Some(confirming), (e.ctx.tables, e.ctx.hands));
        return Stage::Confirm(Box::new(Confirm {
            knob: t.knob,
            old: t.old,
            new: t.new,
            params: t.challenger,
            search: t.sim,
            chunk: 1,
            next_table: 0,
            part: PairedSums::default(),
            so_far: None,
        }));
    }
    let keys: Vec<String> = e.proposals.iter().map(|(k, o, n, _)| transition_key(k, *o, *n)).collect();
    let pool = kept
        .into_iter()
        .filter_map(|(knob, old, new, _, prior)| {
            let index = keys.iter().position(|k| *k == transition_key(&knob, old, new))?;
            Some(Cand { index, knob, old, new, prior: prior.as_ref().map(LedgerEntry::from_paired), round: PairedSums::default() })
        })
        .collect();
    // r0 is a liveness round: one table and a wide field, dropping only what never matters.
    // r1 carries the budget that can rank (0285).
    Stage::Halving(Halving { round: 0, round_tables: 1, spent: 0, pool, cursor: Default::default() })
}

/// Play the next slice of the current halving round, or judge the round once it is complete.
pub(super) fn halving(e: &Env, run: &mut SearchRun, left: f64, did: bool, cap: f64) -> anyhow::Result<Flow> {
    let Stage::Halving(h) = &mut run.stage else { unreachable!("halving stage") };
    if h.cursor.done(h.pool.len()) {
        return Ok(judge_round(e, run));
    }
    let (cands, tables) = h.cursor.slice(h.pool.len(), h.round_tables, run.rate.slice_runs(left, cap));
    let runs = (cands.len() + 1) * tables.len();
    if did && run.rate.secs_for(runs) > left {
        return Ok(Flow::Stop);
    }
    // One batch per slice: the champion plays each table once and every candidate's tables share
    // the whole thread pool.
    e.status(json!({"status": "evaluating", "stratum": "observed",
        "phase": format!("successive halving round {} ({} candidates, {} tables each; candidates {}..{})", h.round + 1, h.pool.len(), h.round_tables, cands.start + 1, cands.end),
        "progress": {"hands": h.spent, "target": e.ctx.tables * e.ctx.hands * 16},
        "next_candidate": {"family": "parameter search", "reason": format!("{} one-knob challengers vs {}", h.pool.len(), run.champion_version)}}));
    let batch: Vec<_> = cands.clone().map(|i| e.eval(&e.proposals[h.pool[i].index].3)).collect();
    let arms: Vec<Arm<'_>> = batch.iter().map(|p| Arm { params: p, nn: e.nn.clone() }).collect();
    let seed = 900_000 + run.cycle * 10_000 + h.round * 1_000;
    let s = Instant::now();
    let sums = paired_sums_arms(
        &Arm { params: &e.eval_champion, nn: e.nn.clone() },
        &arms,
        &e.clones,
        &e.sc.models,
        tables.clone(),
        e.ctx.hands,
        100,
        seed,
    );
    for (i, part) in cands.clone().zip(sums) {
        h.pool[i].round.add(&part);
    }
    h.spent += cands.len() * tables.len() * e.ctx.hands;
    h.cursor.advance(&cands, &tables, h.round_tables);
    run.rate.observe(runs, s.elapsed().as_secs_f64());
    Ok(Flow::Played)
}

/// Why a round dropped a candidate for cause. The code is what the dashboard's funnel counts deaths
/// by (#317); the message is what the log, the experiment card and the rationale say.
#[derive(Clone, Copy)]
enum Drop {
    /// No simulated outcome differed between the two policies: the knob does nothing here.
    NoEffect,
    /// Some outcome differed, but the 95% upper bound was still below the +1 bb/100 bar.
    BelowBar,
}

impl Drop {
    fn code(self) -> &'static str {
        match self {
            Drop::NoEffect => "no-effect",
            Drop::BelowBar => "below-bar",
        }
    }

    fn message(self) -> &'static str {
        match self {
            Drop::NoEffect => "no simulated outcome changed",
            Drop::BelowBar => "upper bound below +1 bb/100",
        }
    }
}

/// Drop, halve and double: the round's decisions, exactly as the one-piece search made them.
fn judge_round(e: &Env, run: &mut SearchRun) -> Flow {
    let cycle = run.cycle;
    let Stage::Halving(h) = &mut run.stage else { unreachable!("halving stage") };
    let round = h.round;
    let before = h.pool.len();
    let mut survivors = Vec::new();
    for mut c in h.pool.drain(..) {
        let this = c.round.result();
        let r = match c.prior.take() {
            Some(prev) => prev.to_paired().combine(&this),
            None => this,
        };
        let reason = if r.differing == 0 {
            Some(Drop::NoEffect)
        } else if r.upper_95() < MIN_EDGE_BB {
            Some(Drop::BelowBar)
        } else {
            None
        };
        let (knob, old, new) = (&c.knob, c.old, c.new);
        match reason {
            Some(why) => {
                tracing::info!(
                    "cycle {cycle} r{round}: drop {knob} {old:.3}->{new:.3}: {:+.2} bb/100 (95% {:+.2}..{:+.2}) — {}",
                    r.mean_bb * 100.0,
                    r.lower_95() * 100.0,
                    r.upper_95() * 100.0,
                    why.message()
                );
                funnel::note(e.ctx.store, &format!("search/{}", why.code()), Some(knob), 1);
                push_experiment(
                    e.ctx.store,
                    json!({
                        "id": format!("c{cycle}-{knob}-{new:.3}"), "status": "rejected", "ts": now(), "hands": r.hands, "target": r.hands,
                        "knob": knob, "old": (old * 1000.0).round() / 1000.0, "new": (new * 1000.0).round() / 1000.0,
                        "mean_bb": r.mean_bb, "lower_95": r.lower_95(), "upper_95": r.upper_95(),
                        "champion": run.champion_version, "challenger": format!("c{cycle}-{knob}-{new:.3}"), "candidate_kind": "parameter",
                        "stage": "search", "reason": why.code(),
                        "rationale": format!("Successive halving round {}: {}. Paired simulation on identical deals against clones of the live pool, all-in luck removed.", round + 1, why.message()),
                        "population": {"id": run.population_id, "opponent_count": e.clones.len(), "evidence": run.evidence},
                        "strata": {"observed": {"hands": r.hands, "mean_bb": r.mean_bb, "lower_95": r.lower_95(), "upper_95": r.upper_95()}}
                    }),
                );
                run.ledger_done.push((transition_key(knob, old, new), LedgerEntry::from_paired(&r)));
            }
            None => {
                c.prior = Some(LedgerEntry::from_paired(&r));
                c.round = PairedSums::default();
                survivors.push(c);
            }
        }
    }
    let mean = |c: &Cand| c.prior.as_ref().map_or(f64::NEG_INFINITY, |p| p.mean_bb);
    survivors.sort_by(|a, b| mean(b).partial_cmp(&mean(a)).unwrap_or(std::cmp::Ordering::Equal));
    let keep = if survivors.len() <= 1 { survivors.len() } else { survivors.len().div_ceil(2) };
    for c in survivors.drain(keep..) {
        tracing::info!("cycle {cycle} r{round}: halved out {} {:.3}->{:.3}: {:+.2} bb/100", c.knob, c.old, c.new, mean(&c) * 100.0);
        // Not a death by a bar: this candidate was pushed out by the ranking, and stays in the
        // ledger to be ranked again. Counted anyway — it is the stage most candidates end in, and
        // the panel cannot say where the search spends its budget without it (#317).
        funnel::note(e.ctx.store, funnel::HALVED_OUT, Some(&c.knob), 1);
        run.ledger_done.push((transition_key(&c.knob, c.old, c.new), c.prior.unwrap_or_default()));
    }
    tracing::info!("cycle {cycle} r{round}: {} of {before} candidates continue", survivors.len());
    h.pool = survivors;
    h.round += 1;
    h.round_tables *= 2;
    h.cursor = Default::default();
    let leader_hands = h.pool.first().and_then(|c| c.prior.as_ref()).map_or(0, |p| p.hands as usize);
    if h.pool.is_empty() || h.pool.len() <= 1 && leader_hands >= e.ctx.tables * e.ctx.hands || h.spent >= e.ctx.tables * e.ctx.hands * 16 {
        return end_halving(e, run);
    }
    Flow::Moved
}

/// The search phase is over: fold every measured transition back into the ledger so the next
/// cycle continues these intervals instead of re-simulating them, and nominate the survivor.
/// Confirmation runs on fresh deals and stays out of the ledger. A save failure only costs
/// simulations — the next cycle re-measures — so it warns instead of failing the search.
fn end_halving(e: &Env, run: &mut SearchRun) -> Flow {
    let Stage::Halving(h) = &mut run.stage else { unreachable!("halving stage") };
    let pool = std::mem::take(&mut h.pool);
    for c in &pool {
        if let Some(p) = &c.prior {
            run.ledger_done.push((transition_key(&c.knob, c.old, c.new), p.clone()));
        }
    }
    let mut ledger = search_ledger::load(e.ctx.store, &run.champion_version, run.refit_rowid);
    let done: Vec<(String, PairedResult)> = run.ledger_done.iter().map(|(k, v)| (k.clone(), v.to_paired())).collect();
    search_ledger::record(&mut ledger, &done);
    if let Err(err) = search_ledger::save(e.ctx.store, &ledger) {
        tracing::warn!("cycle {}: rejection ledger not saved ({err}); the next cycle re-simulates", run.cycle);
    }
    let best = pool.into_iter().next().and_then(|c| {
        let r = c.prior.as_ref()?.to_paired();
        tracing::info!(
            "cycle {}: survivor {} {:.3}->{:.3}: {:+.2} bb/100 (95% {:+.2}..{:+.2}) over {} hands",
            run.cycle,
            c.knob,
            c.old,
            c.new,
            r.mean_bb * 100.0,
            r.lower_95() * 100.0,
            r.upper_95() * 100.0,
            r.hands
        );
        promotion::worth_confirming(&r).then_some((c, r))
    });
    // The experiment pair's queue (0291): the survivor about to be confirmed, then every
    // transition the ledger leaves undecided.
    let confirming = best.as_ref().map(|(c, r)| {
        Target::new(
            (&run.champion_version, run.refit_rowid),
            &c.knob,
            c.old,
            c.new,
            e.proposals[c.index].3.clone(),
            LedgerEntry::from_paired(r),
            Source::Confirmation,
        )
    });
    publish_targets(e.ctx.store, &ledger, &e.sc.champion, run.cycle, confirming, (e.ctx.tables, e.ctx.hands));
    match best {
        Some((c, r)) => Flow::Next(Stage::Confirm(Box::new(Confirm {
            params: e.proposals[c.index].3.clone(),
            knob: c.knob,
            old: c.old,
            new: c.new,
            search: LedgerEntry::from_paired(&r),
            chunk: 1,
            next_table: 0,
            part: PairedSums::default(),
            so_far: None,
        }))),
        None => Flow::Done { promoted: false },
    }
}

/// Promotion rests on fresh deals alone: the search interval is selection-biased. Sequential
/// chunks stop early for futility (`crate::promotion`); a chunk is played in slices and judged
/// once complete, so the looks are the one-piece search's.
pub(super) fn confirm(e: &Env, run: &mut SearchRun, left: f64, did: bool, cap: f64) -> anyhow::Result<Flow> {
    let chunk_tables = e.ctx.tables * CHUNK_SCALE;
    let cycle = run.cycle;
    let Stage::Confirm(c) = &mut run.stage else { unreachable!("confirm stage") };
    if c.next_table >= chunk_tables {
        let part = c.part.result();
        let so_far = match c.so_far.take() {
            Some(prev) => prev.to_paired().combine(&part),
            None => part,
        };
        let outcome = promotion::verdict(&so_far, c.chunk);
        tracing::info!(
            "cycle {cycle}: confirm {}/{CONFIRM_CHUNKS} {} {:.3}->{:.3}: {:+.2} bb/100 (95% {:+.2}..{:+.2}) over {} hands -> {outcome:?}",
            c.chunk,
            c.knob,
            c.old,
            c.new,
            so_far.mean_bb * 100.0,
            so_far.lower_95() * 100.0,
            so_far.upper_95() * 100.0,
            so_far.hands
        );
        if outcome == Verdict::Continue {
            c.so_far = Some(LedgerEntry::from_paired(&so_far));
            c.chunk += 1;
            c.next_table = 0;
            c.part = PairedSums::default();
            return Ok(Flow::Moved);
        }
        let c = (**c).clone();
        return Ok(Flow::Done { promoted: super::conclude::conclude(e, run, &c, outcome, &so_far)? });
    }
    let t = table_slice(c.next_table, chunk_tables, 2, run.rate.slice_runs(left, cap));
    if did && run.rate.secs_for(2 * t.len()) > left {
        return Ok(Flow::Stop);
    }
    e.status(json!({"status": "evaluating", "stratum": "observed",
        "phase": format!("fresh-deal confirmation chunk {} of {CONFIRM_CHUNKS} (tables {}..{} of {chunk_tables})", c.chunk, t.start + 1, t.end),
        "progress": {"hands": c.so_far.as_ref().map_or(0, |s| s.hands) + c.part.hands, "target": CONFIRM_CHUNKS * chunk_tables * e.ctx.hands},
        "next_candidate": {"family": c.knob, "reason": format!("{:.3} -> {:.3} vs {}", c.old, c.new, run.champion_version)}}));
    // Stride 16 > CONFIRM_CHUNKS: no cycle's confirmation reuses another cycle's deals.
    const _: () = assert!(CONFIRM_CHUNKS < 16);
    let seed = 55_000_000 + cycle * 16 + c.chunk as u64;
    let challenger = e.eval(&c.params);
    let s = Instant::now();
    let part = paired_sums_arms(
        &Arm { params: &e.eval_champion, nn: e.nn.clone() },
        &[Arm { params: &challenger, nn: e.nn.clone() }],
        &e.clones,
        &e.sc.models,
        t.clone(),
        e.ctx.hands,
        100,
        seed,
    )
    .pop()
    .expect("one arm");
    c.part.add(&part);
    c.next_table = t.end;
    run.rate.observe(2 * t.len(), s.elapsed().as_secs_f64());
    Ok(Flow::Played)
}

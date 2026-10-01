//! The end of a fresh-deal confirmation: promote the challenger or record the rejection.

use serde_json::json;
use sv10_core::sim::PairedResult;

use super::super::run::{Confirm, SearchRun};
use super::super::{funnel, now, publish_targets, push_experiment};
use super::Env;
use crate::experiment::target::{TARGETS_KEY, TargetQueue};
use crate::promotion::{CHUNK_SCALE, CONFIRM_CHUNKS, Verdict};
use crate::search_ledger::{self, transition_key};

/// Act on the confirmation's final verdict; `true` when the challenger was promoted.
pub(super) fn conclude(e: &Env, run: &SearchRun, c: &Confirm, outcome: Verdict, confirm: &PairedResult) -> anyhow::Result<bool> {
    let store = e.ctx.store;
    let cycle = run.cycle;
    let (knob, old, new) = (&c.knob, c.old, c.new);
    let population =
        json!({"id": run.population_id, "opponent_count": e.clones.len(), "evidence": run.evidence, "evaluation": run.stacks.evidence()});
    if outcome == Verdict::Promote {
        let lineage = super::super::load_lineage(store);
        let version = format!("sv10-ev-{}", lineage.len() + 1);
        let params_json = serde_json::to_string(&c.params)?;
        let mut promoted = lineage;
        promoted.push(version.clone());
        let lineage_json = serde_json::to_string(&promoted)?;
        let promotion = json!({
            "id": format!("{version}-promotion"), "status": "promoted", "ts": now(), "hands": c.search.hands + confirm.hands,
            "knob": knob, "old": old, "new": new, "mean_bb": confirm.mean_bb, "lower_95": confirm.lower_95(), "upper_95": confirm.upper_95(),
            "champion": run.champion_version, "challenger": version,
            "stage": "confirmation", "reason": "promoted",
            "rationale": format!("Won search ({:+.2} bb/100) and fresh-deal confirmation ({:+.2} bb/100, lower bound {:+.2}).", c.search.mean_bb * 100.0, confirm.mean_bb * 100.0, confirm.lower_95() * 100.0),
            "population": population
        });
        super::super::progress::install(store, &params_json, &lineage_json, &promotion)?;
        push_experiment(store, promotion);
        funnel::note(store, funnel::PROMOTED, Some(knob), 1);
        tracing::info!("cycle {cycle}: PROMOTED {version} ({knob} {old:.3}->{new:.3})");
        // Every target was measured against the old champion; the next cycle publishes anew.
        let empty = TargetQueue { champion: version.clone(), refit_rowid: run.refit_rowid, updated: now(), ..Default::default() };
        if let Err(err) = serde_json::to_string(&empty).map_err(anyhow::Error::from).and_then(|j| store.put_kv(TARGETS_KEY, &j)) {
            tracing::warn!("cycle {cycle}: experiment targets not cleared ({err})");
        }
        return Ok(true);
    }
    let (code, why) = match outcome {
        Verdict::Reject(reason) => (reason.code(), reason.message()),
        _ => ("incomplete", "confirmation incomplete"),
    };
    tracing::info!("cycle {cycle}: best candidate failed confirmation ({:+.2} bb/100): {why}", confirm.mean_bb * 100.0);
    funnel::note(store, &format!("confirm/{code}"), Some(knob), 1);
    // A completed fresh-deal rejection is never offered to the experiment pair again.
    let mut ledger = search_ledger::load_evaluated(store, &run.champion_version, run.refit_rowid, &run.stacks.digest);
    ledger.confirm_rejected.insert(transition_key(knob, old, new));
    // The result is discarded, not stored: `confirm_rejected` holds only the key, so a key that
    // comes back costs the full confirmation again. Record the measurement beside the key so the
    // question "does a rejected candidate ever return?" has data behind it before anything is
    // built to answer it.
    tracing::info!(
        "cycle {cycle}: confirmation rejected {knob} {old:.3}->{new:.3} over {} hands ({:+.2} bb/100, 95% {:+.2}..{:+.2})",
        confirm.hands,
        confirm.mean_bb * 100.0,
        confirm.lower_95() * 100.0,
        confirm.upper_95() * 100.0
    );
    if let Err(err) = search_ledger::save(store, &ledger) {
        tracing::warn!("cycle {cycle}: rejection ledger not saved ({err})");
    }
    publish_targets(store, &ledger, &e.sc.champion, cycle, None, (e.ctx.tables, e.ctx.hands));
    push_experiment(
        store,
        json!({
            "id": format!("c{cycle}-{knob}-{new:.3}-confirm"), "status": "rejected", "ts": now(), "hands": confirm.hands,
            "target": CONFIRM_CHUNKS * e.ctx.tables * CHUNK_SCALE * e.ctx.hands,
            "knob": knob, "old": (old * 1000.0).round() / 1000.0, "new": (new * 1000.0).round() / 1000.0,
            "mean_bb": confirm.mean_bb, "lower_95": confirm.lower_95(), "upper_95": confirm.upper_95(),
            "champion": run.champion_version, "challenger": format!("c{cycle}-{knob}-{new:.3}"), "candidate_kind": "parameter",
            "stage": "confirmation", "reason": code,
            "rationale": format!("Search survivor ({:+.2} bb/100) failed fresh-deal confirmation: {why}.", c.search.mean_bb * 100.0),
            "population": population
        }),
    );
    Ok(false)
}

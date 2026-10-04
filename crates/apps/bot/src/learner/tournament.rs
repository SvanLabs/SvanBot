//! Tournament refresh (ADR 0002, stage 4). Every few days the lineages' champions play a paired
//! round-robin on identical deals. A lineage another beats at the 95% lower bound is replaced by a
//! copy of that winner, and its own parameters are archived: population-based training's exploit
//! step, taken through the same paired evaluation the search uses. Stored in steps like a search.

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Instant;
use sv10_core::agents::live_pool;
use sv10_core::policy::Params;
use sv10_core::sim::{Arm, PairedResult, PairedSums, paired_sums_arms_stacked};
use sv10_store::store::Store;

use super::lane::Lane;
use super::run::{Rate, SLICE_TARGET_SECS, STEP_TARGET_SECS, table_slice};
use super::{Ctx, MIN_OPPONENT_HANDS, now};
use crate::promotion::CHUNK_SCALE;

/// When the last round ran, and what it found.
pub const TOURNAMENT_KEY: &str = "learner.tournament.v1";
/// A lineage's latest replacement (`.slot.<bot>` suffix per lane).
pub const LAST_REFRESH_KEY: &str = "learner.last-refresh.v1";
/// Between rounds.
pub const INTERVAL_SECS: f64 = 3.0 * 86_400.0;
/// Retry after an abandoned round.
const RETRY_SECS: f64 = 3_600.0;

/// One lineage's champion as the round plays it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Parent {
    pub lane: String,
    pub params: Params,
}

/// A round in progress: sweep `i` plays parent `i` against every later parent on the same deals.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TournamentRun {
    pub started: f64,
    pub seed: u64,
    pub parents: Vec<Parent>,
    /// The champions as stored, to notice one changing under the round.
    pub digest: String,
    pub stacks: super::stacks::Fixture,
    pub rate: Rate,
    pub sweep: usize,
    pub next_table: usize,
    /// `sums[i][k]` is parent `i + 1 + k` minus parent `i`.
    pub sums: Vec<Vec<PairedSums>>,
}

/// A replacement the round decided: `loser` becomes a copy of `winner`.
#[derive(Debug, PartialEq)]
pub struct Replacement {
    pub loser: String,
    pub winner: String,
}

fn state(store: &Store) -> serde_json::Value {
    store.get_kv(TOURNAMENT_KEY).ok().flatten().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| json!({}))
}

fn mark(store: &Store, last: f64, extra: serde_json::Value) {
    let mut v = state(store);
    v["last"] = json!(last);
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    if let Err(e) = store.put_kv(TOURNAMENT_KEY, &v.to_string()) {
        tracing::warn!("tournament state not stored ({e})");
    }
}

/// Start a round when two or more lineages exist and the last one is old enough. `None` otherwise,
/// including when every champion is still the same copy (nothing to compare).
pub fn begin_if_due(ctx: &Ctx, lanes: &[Lane], at: f64) -> anyhow::Result<Option<TournamentRun>> {
    let store = ctx.store;
    let bots: Vec<&str> = lanes.iter().filter_map(|l| l.0.as_deref()).collect();
    if bots.len() < 2 || at - state(store)["last"].as_f64().unwrap_or(0.0) < INTERVAL_SECS {
        return Ok(None);
    }
    let parents: Vec<Parent> =
        bots.iter().map(|b| Parent { lane: b.to_string(), params: super::search::lane_inputs(store, &Lane::bot(b)).0 }).collect();
    let digest = super::run::digest(&serde_json::to_string(&parents)?);
    let distinct = parents.iter().any(|p| serde_json::to_string(&p.params).ok() != serde_json::to_string(&parents[0].params).ok());
    if !distinct {
        mark(store, at, json!({"skipped": "every lineage is still a copy of the same champion"}));
        return Ok(None);
    }
    let n = parents.len();
    Ok(Some(TournamentRun {
        started: at,
        seed: 800_000 + state(store)["count"].as_u64().unwrap_or(0) * 10_000,
        parents,
        digest,
        stacks: super::stacks::Fixture::capture(store, store.max_hand_rowid().unwrap_or(0))?,
        rate: Rate::guess(ctx.threads, ctx.hands),
        sweep: 0,
        next_table: 0,
        sums: (0..n - 1).map(|i| vec![PairedSums::default(); n - 1 - i]).collect(),
    }))
}

/// Play slices for about [`STEP_TARGET_SECS`]; `true` when the round is finished and applied.
pub fn step(ctx: &Ctx, t: &mut TournamentRun, first_cap: f64) -> anyhow::Result<bool> {
    let store = ctx.store;
    let current: Vec<Parent> = t
        .parents
        .iter()
        .map(|p| Parent { lane: p.lane.clone(), params: super::search::lane_inputs(store, &Lane::bot(&p.lane)).0 })
        .collect();
    if super::run::digest(&serde_json::to_string(&current)?) != t.digest {
        tracing::warn!("tournament: a lineage's champion changed under the round; it starts over later");
        mark(store, now() - INTERVAL_SECS + RETRY_SECS, json!({"abandoned": now()}));
        return Ok(true);
    }
    let (_, models, nn) = super::search::lane_inputs(store, &Lane::default());
    let clones = live_pool(&models, MIN_OPPONENT_HANDS, 16, 7_000 + t.seed);
    let eval = |p: &Params| Params { samples: ctx.decision_samples, deal_chunks: 1, ..p.clone() };
    let total = ctx.tables * CHUNK_SCALE;
    let t0 = Instant::now();
    let mut cap = first_cap;
    while t.sweep + 1 < t.parents.len() {
        let left = STEP_TARGET_SECS - t0.elapsed().as_secs_f64();
        let arms = t.parents.len() - 1 - t.sweep;
        let slice = table_slice(t.next_table, total, arms + 1, t.rate.slice_runs(left, cap));
        if left <= 0.0 || (cap == SLICE_TARGET_SECS && t.rate.secs_for((arms + 1) * slice.len()) > left) {
            return Ok(false);
        }
        let base = eval(&t.parents[t.sweep].params);
        let others: Vec<Params> = t.parents[t.sweep + 1..].iter().map(|p| eval(&p.params)).collect();
        let started = Instant::now();
        let parts = paired_sums_arms_stacked(
            &Arm { params: &base, nn: nn.clone() },
            &others.iter().map(|p| Arm { params: p, nn: nn.clone() }).collect::<Vec<_>>(),
            &clones,
            &models,
            slice.clone(),
            ctx.hands,
            &t.stacks.layouts,
            t.seed + t.sweep as u64,
        );
        for (sum, part) in t.sums[t.sweep].iter_mut().zip(&parts) {
            sum.add(part);
        }
        t.rate.observe((arms + 1) * slice.len(), started.elapsed().as_secs_f64());
        cap = SLICE_TARGET_SECS;
        t.next_table = slice.end;
        if t.next_table >= total {
            t.sweep += 1;
            t.next_table = 0;
        }
    }
    finish(store, t)?;
    Ok(true)
}

/// Who replaces whom: lineage `i` is dominated by `j` when `j` beats it at the 95% lower bound; a
/// dominated lineage is replaced by its best undominated conqueror. `results[a][b]` is `b - a`.
pub fn decide(names: &[String], results: &[(usize, usize, PairedResult)]) -> Vec<Replacement> {
    let mut dominated = vec![false; names.len()];
    for (i, j, r) in results {
        if r.lower_95() > 0.0 {
            dominated[*i] = true;
        }
        if r.upper_95() < 0.0 {
            dominated[*j] = true;
        }
    }
    let mut out = Vec::new();
    for i in (0..names.len()).filter(|i| dominated[*i]) {
        // The undominated lineage that beats `i` decisively by the most.
        let margin = |&(a, b, ref r): &(usize, usize, PairedResult)| match (a == i, b == i) {
            (true, _) if r.lower_95() > 0.0 => Some((b, r.mean_bb)),
            (_, true) if r.upper_95() < 0.0 => Some((a, -r.mean_bb)),
            _ => None,
        };
        let best = results.iter().filter_map(margin).filter(|(j, _)| !dominated[*j]).max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((j, _)) = best {
            out.push(Replacement { loser: names[i].clone(), winner: names[j].clone() });
        }
    }
    out
}

fn finish(store: &Store, t: &TournamentRun) -> anyhow::Result<()> {
    let names: Vec<String> = t.parents.iter().map(|p| p.lane.clone()).collect();
    let mut results = Vec::new();
    for (i, row) in t.sums.iter().enumerate() {
        for (k, sums) in row.iter().enumerate() {
            results.push((i, i + 1 + k, sums.result()));
        }
    }
    let replaced = decide(&names, &results);
    for r in &replaced {
        replace(store, r, t.started)?;
    }
    let report: Vec<_> = results
        .iter()
        .map(|(i, j, r)| json!({"a": names[*i], "b": names[*j], "mean_bb": r.mean_bb, "lower_95": r.lower_95(), "upper_95": r.upper_95(), "hands": r.hands}))
        .collect();
    let count = state(store)["count"].as_u64().unwrap_or(0) + 1;
    mark(
        store,
        t.started,
        json!({"count": count, "pairs": report, "replaced": replaced.iter().map(|r| json!({"loser": r.loser, "winner": r.winner})).collect::<Vec<_>>()}),
    );
    tracing::info!("tournament: {} replacement(s) among {} lineages", replaced.len(), names.len());
    Ok(())
}

/// `loser` becomes a copy of `winner`; what it played is kept under `archive.params.<loser>.<time>`.
fn replace(store: &Store, r: &Replacement, at: f64) -> anyhow::Result<()> {
    let (loser, winner) = (Lane::bot(&r.loser), Lane::bot(&r.winner));
    let archived = format!("archive.params.{}.{}", r.loser, at as i64);
    let own = serde_json::to_string(&loser.params(store))?;
    let record = json!({"ts": at, "from": r.winner, "archived": archived}).to_string();
    store.put_kv_batch(&[
        (&archived, &own),
        (&loser.params_key(), &serde_json::to_string(&winner.params(store))?),
        (&loser.key(crate::LEARNER_LINEAGE_KEY), &serde_json::to_string(&winner.lineage(store))?),
        (&loser.key(LAST_REFRESH_KEY), &record),
    ])?;
    tracing::info!("tournament: {} replaced by a copy of {} (kept as {archived})", r.loser, r.winner);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(mean_bb: f64, se_bb: f64) -> PairedResult {
        PairedResult { hands: 100_000, mean_bb, se_bb, differing: 50_000 }
    }

    #[test]
    fn a_decisively_beaten_lineage_is_replaced_by_the_best_undominated_winner() {
        let names: Vec<String> = ["A", "B", "C"].map(String::from).to_vec();
        // B beats A clearly, C beats A by more, C and B are level: A takes C.
        let results = [(0, 1, result(0.02, 0.005)), (0, 2, result(0.05, 0.005)), (1, 2, result(0.01, 0.02))];
        assert_eq!(decide(&names, &results), [Replacement { loser: "A".into(), winner: "C".into() }]);
        // Nothing decisive: nobody is replaced.
        assert!(decide(&names, &[(0, 1, result(0.01, 0.02)), (0, 2, result(-0.01, 0.02)), (1, 2, result(0.0, 0.02))]).is_empty());
        // A winner that is itself dominated does not replace anyone this round.
        let chain = [(0, 1, result(0.02, 0.005)), (1, 2, result(0.02, 0.005)), (0, 2, result(0.04, 0.005))];
        assert_eq!(
            decide(&names, &chain),
            [Replacement { loser: "A".into(), winner: "C".into() }, Replacement { loser: "B".into(), winner: "C".into() }]
        );
    }

    #[test]
    fn a_replacement_archives_the_loser_and_copies_the_winner() {
        let shared = crate::live::Shared::for_test("tournament-replace", &["A"]);
        let store = &shared.store;
        let (a, b) = (Lane::bot("A"), Lane::bot("B"));
        store.put_kv(&a.params_key(), &serde_json::to_string(&Params { call_margin: 0.1, ..Default::default() }).unwrap()).unwrap();
        store.put_kv(&b.params_key(), &serde_json::to_string(&Params { call_margin: 0.2, ..Default::default() }).unwrap()).unwrap();
        store.put_kv(&b.key(crate::LEARNER_LINEAGE_KEY), r#"["sv10-ev-1","B-ev-2"]"#).unwrap();
        replace(store, &Replacement { loser: "A".into(), winner: "B".into() }, 1_000.0).unwrap();
        assert_eq!(a.params(store).call_margin, 0.2);
        assert_eq!(a.lineage(store), ["sv10-ev-1", "B-ev-2"]);
        let kept: Params = serde_json::from_str(&store.get_kv("archive.params.A.1000").unwrap().unwrap()).unwrap();
        assert_eq!(kept.call_margin, 0.1);
        assert_eq!(b.params(store).call_margin, 0.2, "the winner is untouched");
    }

    #[test]
    fn a_round_plays_every_pair_on_identical_deals_and_records_the_report() {
        use sv10_core::model::{ModelStore, PlayerStats};
        let dir = std::env::temp_dir().join(format!("sv10-tournament-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::open(&dir.join("svanbot10.db")).unwrap();
        let mut models = ModelStore::default();
        for (i, hands) in [60.0, 55.0, 50.0, 45.0, 40.0].into_iter().enumerate() {
            models.players.insert(format!("villain{i}"), PlayerStats { hands, ..Default::default() });
        }
        store.put_kv(super::super::POPULATION_MODELS_KEY, &serde_json::to_string(&models).unwrap()).unwrap();
        for (bot, margin) in [("A", 0.0), ("B", 0.05), ("C", -0.05)] {
            store
                .put_kv(
                    &Lane::bot(bot).params_key(),
                    &serde_json::to_string(&Params { call_margin: margin, ..Default::default() }).unwrap(),
                )
                .unwrap();
        }
        let ctx = Ctx { store: &store, root: &dir, threads: 2, tables: 1, hands: 20, decision_samples: 40 };
        let lanes = Lane::rotation(Some("A,B,C"));
        assert!(begin_if_due(&ctx, &lanes[..1], 1e9).unwrap().is_none(), "one lineage has no one to play");
        let mut t = begin_if_due(&ctx, &lanes, 1e9).unwrap().expect("three distinct champions are due a round");
        assert!(begin_if_due(&ctx, &lanes, 1e9 + 1.0).unwrap().is_some(), "due until a round is recorded");
        let mut steps = 0;
        while !step(&ctx, &mut t, 60.0).unwrap() {
            steps += 1;
            assert!(steps < 50, "the round must end");
        }
        let report = state(&store);
        assert_eq!(report["count"], 1);
        assert_eq!(report["pairs"].as_array().unwrap().len(), 3);
        assert!(report["pairs"][0]["hands"].as_u64().unwrap() > 0);
        assert!(begin_if_due(&ctx, &lanes, 1e9 + 2.0).unwrap().is_none(), "not again for days");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

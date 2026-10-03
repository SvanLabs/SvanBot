//! The post-promotion drift instrument (0128, 0366): the population it judges, the basis it re-solves
//! on, the row it stores under `analyst.drift`, and the render of that row (`review drift`).
//!
//! The analyst runs the check when its audit queue is empty ([`drift_check`], called from the bin);
//! `review drift` prints the row it left. `review_wiring` is the same shape: the measurement lives in
//! the library and both surfaces call it.

use anyhow::Result;
use serde_json::json;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use sv10_core::nn::Mlp;
use sv10_core::policy::Params;
use sv10_store::store::{ReplayRow, Store};

use crate::replay::{REPLAY_VERSION, ReplayRecord, audit, rerun};

/// The basis a stored drift row is measured on (0358's convention; 0366): today's champion knobs
/// (`params.v1`) re-solved at the recorded decision's own prices, at the analyst's budget.
///
/// A different basis is a different number, so the writer stamps this tag into the row and treats a
/// row whose tag is missing or older as due for a re-check rather than read as current (the analyst
/// bin; 0120 reads that row against a standing threshold). Changing what the run varies — the knob set
/// it grades, the price basis it grades them from, or the budget it solves at — means a new version
/// here, never a silent re-reading of the old number.
pub const DRIFT_BASIS: &str = "champion-knobs@recorded-prices+analyst-budget/v1";

/// Big-spot replays re-run per drift check (each also gets one deep audit).
const DRIFT_REPLAYS: usize = 48;
/// Replays read to fill one drift sample, newest first. The sample is pinned to the current replay
/// version (0366), so the read has to cover the rows that filter will drop; each row read past the
/// sample costs a record decode, which is why the read is bounded rather than every row stored.
const DRIFT_SCAN: usize = DRIFT_REPLAYS * 4;
/// Longest slice of a drift check: under the two-minute budget for live-system work, with room for
/// the one deep re-solve that may start just before it ends (about 14 s at 8 threads).
pub const DRIFT_STEP: Duration = Duration::from_secs(100);

/// Replay-network cache shared by the caller's audit path and the drift check: digest to network,
/// `None` once looked up and missing.
pub type NetCache = HashMap<String, Option<Arc<Mlp>>>;

/// Response network from the replay store, cached across replays and audits.
pub fn cached_net(store: &Store, nets: &mut NetCache, digest: &str) -> Option<Arc<Mlp>> {
    nets.entry(digest.to_string())
        .or_insert_with(|| {
            let net = store.replay_net(digest).ok().flatten().and_then(|j| serde_json::from_str::<Mlp>(&j).ok());
            if net.is_none() {
                tracing::warn!("response network {digest} not found; auditing with the stat model");
            }
            net.map(Arc::new)
        })
        .clone()
}

/// Short digest identifying one champion's parameters.
pub fn params_digest(params_json: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    params_json.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn now() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64()
}

/// The parameters both of one drift record's re-solves run with (0366): today's champion knobs
/// (`params.v1`) with the recorded decision's own local set — the prices the action was actually
/// made at, through the one definition shared with `adopt_promoted` (`Params::adopt_recorded_local`,
/// 0358's convention) — so the only strategy knob the re-solve varies against the recorded action is
/// the champion's.
///
/// The record's budget and deal decomposition come along with its local set and are replaced by the
/// analyst's own (`samples` over `threads`): the deep search is the instrument's depth, and it is the
/// one deliberate difference from the recorded action, which the row and the log name. This is the
/// shape the analyst's audit branch uses for `deep`, differing only in which knobs it grades — the
/// audit grades the record's, this grades the champion's.
fn drift_basis(champion: &Params, rec: &ReplayRecord, samples: usize, threads: usize) -> Params {
    let mut params = champion.clone();
    params.adopt_recorded_local(&rec.params);
    params.samples = samples;
    params.deal_chunks = threads;
    params
}

/// The population one drift run was judged on: which replays, out of what was read, and what the
/// version filter left out (0366). `review audit-by` states its window's mix the same way.
struct Population {
    /// Replay row id of the oldest judged record.
    oldest_id: i64,
    /// Replay row id of the newest judged record.
    newest_id: i64,
    /// Decision time of the oldest judged record.
    from: String,
    /// Decision time of the newest judged record.
    to: String,
    /// Rows read from the store's newest, before the version filter.
    scanned: usize,
    /// Record versions among the scanned rows, largest share first (`v3 48, v2 90`).
    versions: String,
    /// How many of the scanned rows the version filter dropped: not the current replay version. Rows
    /// read past it are not counted here — they are of the right version and simply older than the
    /// sample, which `ids` and `scanned` already say.
    other_version: usize,
}

impl Population {
    /// The row's `population` object.
    fn json(&self) -> serde_json::Value {
        json!({"ids": format!("{}-{}", self.oldest_id, self.newest_id), "from": self.from, "to": self.to,
            "scanned": self.scanned, "other_version": self.other_version, "version": REPLAY_VERSION,
            "versions": self.versions})
    }

    /// The population in words, for the log line.
    fn describe(&self) -> String {
        format!(
            "replays {}..{} ({}..{}), {} rows read, {} of them not replay v{REPLAY_VERSION} ({})",
            self.oldest_id, self.newest_id, self.from, self.to, self.scanned, self.other_version, self.versions
        )
    }
}

/// The newest [`DRIFT_REPLAYS`] scanned rows written by the current replay version, with the
/// population they are drawn from: the version mix of everything read and how many of those rows the
/// filter dropped.
///
/// A pre-v3 record carries no per-opponent corrections (0316; `replay.rs`), so re-solving it compares
/// the live action with a model that saw less than the live action did — the same family of
/// unattributed disagreement this check removes from its prices (0366). The version lives inside the
/// record's JSON, so the filter cannot be a SQL predicate: `Store::replays` takes an id.
fn drift_sample(rows: Vec<ReplayRow>) -> (Vec<ReplayRow>, Population) {
    let scanned = rows.len();
    let mut counts: BTreeMap<u32, usize> = Default::default();
    let mut kept = Vec::new();
    for row in rows {
        let version = serde_json::from_str::<ReplayRecord>(&row.record).map_or(0, |rec| rec.version);
        *counts.entry(version).or_default() += 1;
        if version == REPLAY_VERSION && kept.len() < DRIFT_REPLAYS {
            kept.push(row);
        }
    }
    let population = Population {
        oldest_id: kept.last().map_or(0, |r| r.id),
        newest_id: kept.first().map_or(0, |r| r.id),
        from: kept.last().map(|r| r.ts.clone()).unwrap_or_default(),
        to: kept.first().map(|r| r.ts.clone()).unwrap_or_default(),
        scanned,
        versions: version_mix(&counts),
        other_version: counts.iter().filter(|(v, _)| **v != REPLAY_VERSION).map(|(_, n)| *n).sum(),
    };
    (kept, population)
}

/// Record-version counts, largest share first (`v3 48, v2 90, v0 54`), as `review audit-by` states
/// its own window's mix. Version zero is the legacy default a record without the field reads back
/// as, so it is the pre-v3 group.
fn version_mix(counts: &BTreeMap<u32, usize>) -> String {
    let mut named: Vec<(u32, usize)> = counts.iter().map(|(v, n)| (*v, *n)).collect();
    named.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    named.iter().map(|(v, n)| format!("v{v} {n}")).collect::<Vec<_>>().join(", ")
}

/// A drift check under way (0334): its replays are re-solved a slice at a time, each slice within
/// [`DRIFT_STEP`], so the analyst never holds the cores for minutes (48 deep re-solves took up to
/// 11 min in one go); live audits are served between slices.
pub struct DriftRun {
    digest: String,
    current: Params,
    rows: Vec<ReplayRow>,
    next: usize,
    n: u64,
    flips: u64,
    gap_sum: f64,
    gap_max: f64,
    /// The replays this run was drawn from and what the version filter left out, carried to the row
    /// and the log the check ends with (0366).
    population: Population,
}

/// Start a drift check when the champion changed, or continue the one under way, for at most
/// `budget` (always at least one replay). Returns whether a check is still under way.
///
/// Both re-solves of a record run on one [`drift_basis`] parameter set (0366): the flip test re-runs
/// the policy on it and the gap is an [`audit`] on the same set, so the two numbers in the row are on
/// one basis, and the row names it.
pub fn drift_check(
    store: &Store,
    nets: &mut NetCache,
    threads: usize,
    samples: usize,
    run: &mut Option<DriftRun>,
    budget: Duration,
) -> bool {
    if run.is_none() {
        *run = start_drift(store);
    }
    let Some(r) = run.as_mut() else { return false };
    let started = Instant::now();
    let mut judged = 0;
    while r.next < r.rows.len() {
        // At least one replay per slice, then stop once the budget is spent.
        if judged > 0 && started.elapsed() >= budget {
            break;
        }
        let row = &r.rows[r.next];
        r.next += 1;
        judged += 1;
        let Ok(rec) = serde_json::from_str::<ReplayRecord>(&row.record) else { continue };
        let nn = rec.net_digest.as_ref().and_then(|d| cached_net(store, nets, d));
        let deep = drift_basis(&r.current, &rec, samples, threads);
        let now_d = rerun(&rec, &deep, nn.as_deref());
        r.n += 1;
        if (now_d.action_name.as_str(), now_d.amount) != (rec.action.0.as_str(), rec.action.1) {
            r.flips += 1;
        }
        let a = audit(&rec, &deep, nn.as_deref(), &row.bot, &row.hand_id);
        r.gap_sum += a.gap_bb;
        r.gap_max = r.gap_max.max(a.gap_bb);
    }
    if r.next < r.rows.len() {
        return true;
    }
    let done = run.take().expect("checked above");
    if done.n > 0 {
        let mean_gap = done.gap_sum / done.n as f64;
        // The row says what the number is measured on, not only which champion it names (LESSONS 39):
        // the basis tag, the budget, the population with its version mix, and where the prices came
        // from. A row without the tag is due for a re-check ([`start_drift`]).
        let summary = json!({"basis": DRIFT_BASIS, "digest": done.digest, "ts": now(), "replays": done.n,
            "flips": done.flips, "flip_rate": done.flips as f64 / done.n as f64, "mean_gap_bb": mean_gap,
            "max_gap_bb": done.gap_max, "samples": samples, "deal_chunks": threads,
            "fits": "rec.params (the recorded decision's own prices)", "population": done.population.json()});
        // A row that does not land is reported, but it is not lost work: `start_drift` reads the row
        // back and starts the check again when the stored one is not this champion, so the next idle
        // pass redoes it (issue #326).
        if let Err(e) = store.put_kv(crate::ANALYST_DRIFT_KEY, &summary.to_string()) {
            tracing::warn!("the drift row was not stored, so this check reruns on the next idle pass ({e})");
        }
        tracing::info!(
            "drift [{DRIFT_BASIS}] vs champion {}: {}/{} big-spot actions flip, mean deep gap {mean_gap:.2} bb, \
             max {:.2} bb; {} at {samples} samples x {threads} chunks, the record's own prices",
            done.digest,
            done.flips,
            done.n,
            done.gap_max,
            done.population.describe()
        );
    }
    false
}

/// A new drift check when the champion changed since the last one and there are replays to judge.
fn start_drift(store: &Store) -> Option<DriftRun> {
    let params_json = store.get_kv(crate::PARAMS_KEY).ok().flatten()?;
    let digest = params_digest(&params_json);
    // A stored row is current only when it names both this champion and this basis: the digest says
    // which knobs were graded, not what they were graded against, so a row from a superseded basis is
    // due for a re-check instead of being read as current by 0120 (0366). Without this, changing the
    // basis alone would leave the older reading in the kv indefinitely, since the check reruns only
    // when the digest moves.
    let seen = store.get_kv(crate::ANALYST_DRIFT_KEY).ok().flatten().and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok());
    let checked = seen.as_ref().is_some_and(|v| {
        v.get("digest").and_then(|d| d.as_str()) == Some(digest.as_str()) && v.get("basis").and_then(|b| b.as_str()) == Some(DRIFT_BASIS)
    });
    if checked {
        return None;
    }
    let current: Params = match serde_json::from_str(&params_json) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("champion params unreadable, skipping drift check: {e}");
            return None;
        }
    };
    let rows = match store.replays(DRIFT_SCAN, None) {
        Ok(r) if !r.is_empty() => r,
        Ok(_) => return None,
        Err(e) => {
            tracing::warn!("reading replays for the drift check failed: {e}");
            return None;
        }
    };
    let (rows, population) = drift_sample(rows);
    if rows.is_empty() {
        // Not stored, so the check stays due: a version bump makes the drift row unavailable until
        // records of the new version exist, and that must be visible rather than a stale number.
        tracing::warn!("no replay v{REPLAY_VERSION} records among the newest {DRIFT_SCAN} ({}); drift check skipped", population.versions);
        return None;
    }
    Some(DriftRun { digest, current, rows, next: 0, n: 0, flips: 0, gap_sum: 0.0, gap_max: 0.0, population })
}

/// The stored drift row as one line, with what it was measured on and whether it is the row the
/// analyst would store now — the only surface that prints it, so a basis named only inside the kv
/// is not left as visible as a hand query (0366).
///
/// `json` (#723) prints the stored row itself — the object the analyst wrote, exactly as stored —
/// with the same verdict beside it, instead of the line the row is read into.
pub fn drift(store: &Store, json: bool) -> Result<String> {
    let row = store.get_kv(crate::ANALYST_DRIFT_KEY)?;
    let champion = store.get_kv(crate::PARAMS_KEY)?;
    if json {
        return Ok(render_json(row.as_deref(), champion.as_deref()));
    }
    Ok(render(row.as_deref(), champion.as_deref()))
}

/// [`drift`] on the two values it reads, so the render is testable without a store.
fn render(row: Option<&str>, champion: Option<&str>) -> String {
    let Some(value) = row.and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok()) else {
        return "drift: no row stored — the analyst measures one when idle, once the champion or the basis changes\n".into();
    };
    let num = |k: &str| value.get(k).and_then(serde_json::Value::as_f64).unwrap_or(0.0);
    let basis = value.get("basis").and_then(|v| v.as_str()).unwrap_or("");
    let pop = value.get("population").cloned().unwrap_or_default();
    let field = |k: &str| pop.get(k).and_then(|v| v.as_str()).unwrap_or("?").to_string();
    let count = |k: &str| pop.get(k).and_then(serde_json::Value::as_u64).unwrap_or(0);
    // The row is the number plus everything the number is conditioned on (LESSONS 39): which
    // champion, on which basis, at which budget, over which replays, out of what was left out. A row
    // written before 0366 has none of the last three and prints them as `?`/0, which is what makes it
    // legible as the older basis it is.
    let line = format!(
        "drift [{basis}]: {:.0}/{:.0} big-spot actions flip ({:.2}%), deep gap mean {:.2} bb / max {:.2} bb | \
         {} samples x {} chunks | replays {} ({}..{}) | {} rows read, {} of them not replay v{} ({}) | prices {}",
        num("flips"),
        num("replays"),
        num("flip_rate") * 100.0,
        num("mean_gap_bb"),
        num("max_gap_bb"),
        num("samples"),
        num("deal_chunks"),
        field("ids"),
        short_ts(&field("from")),
        short_ts(&field("to")),
        count("scanned"),
        count("other_version"),
        count("version"),
        field("versions"),
        value.get("fits").and_then(|v| v.as_str()).unwrap_or("?"),
    );
    format!("{line}\n   {}\n", verdict(&value, champion))
}

/// The words that end the drift line: current, or why the row is due for a re-check (0358, 0366).
/// Shared by the text line and the JSON render, so the two cannot disagree about a row's status.
fn verdict(value: &serde_json::Value, champion: Option<&str>) -> String {
    let basis = value.get("basis").and_then(|v| v.as_str()).unwrap_or("");
    let stored = value.get("digest").and_then(|v| v.as_str()).unwrap_or("");
    if basis != DRIFT_BASIS {
        format!(
            "due for a re-check: measured on a superseded basis ({}), the analyst measures on {DRIFT_BASIS}",
            if basis.is_empty() { "none stored".to_string() } else { format!("{basis:?}") }
        )
    } else {
        match champion.map(params_digest) {
            Some(now) if now == stored => format!("current: champion {stored}"),
            Some(now) => format!("due for a re-check: the champion changed since it was measured ({stored} -> {now})"),
            None => "no champion (params.v1) stored to compare the row's digest with".to_string(),
        }
    }
}

/// [`drift`] as JSON (#723): the stored row exactly as the analyst wrote it (so every field it
/// carries survives), the basis a row has to name to be current, and the verdict.
fn render_json(row: Option<&str>, champion: Option<&str>) -> String {
    const NO_ROW: &str = "no row stored — the analyst measures one when idle, once the champion or the basis changes";
    match row.and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok()) {
        Some(value) => {
            let v = verdict(&value, champion);
            json!({"basis": DRIFT_BASIS, "verdict": v, "row": value}).to_string()
        }
        None => json!({"basis": DRIFT_BASIS, "verdict": NO_ROW, "row": serde_json::Value::Null}).to_string(),
    }
}

/// A stored RFC 3339 time cut to the minute, for a row that prints a window.
fn short_ts(ts: &str) -> &str {
    ts.get(..16).unwrap_or(ts)
}

#[cfg(test)]
mod tests;

//! Experiment mode (0266, 0267, 0291): while the fleet holds #1–#4 of the active season, the bot at
//! #4 and the fifth bot run one controlled live test of a learner challenger — one plays it
//! (treatment) while the other plays the champion (control), swapping every block of hands. The
//! protected trio always plays the champion.
//!
//! Three parts, one per module: [`mode`] decides when the mode is on and who is in the pair,
//! [`target`] which challenger they test, [`evidence`] what the live hands say about it. This file
//! wires them to live play: the poll loop (one process), the refresh every process runs, and the
//! per-hand latch that fixes a bot's policy when a hand starts and never changes it mid-hand.
//!
//! Treatment hands carry their provenance into the store and stay out of every production fit;
//! live evidence never promotes (only `sv10_bot::promotion` does) and never relaxes its gate.

pub mod evidence;
pub mod mode;
pub mod target;

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};
use sv10_core::policy::Params;
use sv10_store::store::HandTag;

use crate::live::Shared;
pub use evidence::{Arm, Estimate, VERDICTS_KEY, Verdict, VerdictRecord, Verdicts, arm_for};
pub use mode::{ModeState, Reading, Status};
pub use target::{TARGETS_KEY, Target, TargetQueue};

/// KV key of the persisted [`ModeState`], written by the polling process.
pub const MODE_KEY: &str = "experiment.mode.v1";

/// What every process knows about the experiment, refreshed from the store.
#[derive(Clone, Debug, Default)]
pub struct Live {
    /// The mode as last written by the polling process.
    pub mode: ModeState,
    /// The target the pair runs, when one is safe.
    pub target: Option<Arc<Target>>,
    /// Why no target runs, when none does.
    pub no_target: Option<String>,
    /// The learner's published queue.
    pub queue: Option<TargetQueue>,
    /// Live verdicts by target id.
    pub verdicts: Verdicts,
    /// Hands each pair bot has played on the current target (stored, plus latched since).
    pub hands_on_target: HashMap<String, u64>,
    /// The current estimate of the running target.
    pub estimate: Option<Estimate>,
    /// Last arm change per bot: (unix seconds, arm).
    pub last_assignment: HashMap<String, (f64, Arm)>,
    /// A store failure that stopped the experiment (fail closed until the next good refresh).
    pub store_error: Option<String>,
}

/// The policy a bot fixed for one hand.
#[derive(Clone, Debug)]
pub struct HandPolicy {
    /// The hand it belongs to.
    pub hand_id: String,
    /// The experiment arm, or `None` for ordinary champion play.
    pub arm: Option<Arm>,
    /// The target under test (with `arm`).
    pub target: Option<Arc<Target>>,
    /// The provenance record stored with the hand and its decisions.
    pub record: Option<Value>,
}

impl HandPolicy {
    fn ordinary(hand_id: &str) -> Self {
        HandPolicy { hand_id: hand_id.to_string(), arm: None, target: None, record: None }
    }

    /// The parameters for a decision in this hand: the live champion, or for the treatment arm the
    /// live champion with the challenger's knobs (local fits and budget stay the live ones).
    pub fn params(&self, live: &Params) -> Params {
        match (&self.arm, &self.target) {
            (Some(Arm::Treatment), Some(t)) => {
                let mut p = live.clone();
                p.adopt_promoted(t.challenger.clone());
                p
            }
            _ => live.clone(),
        }
    }

    /// The store tag for this hand, if it is an experiment hand.
    pub fn tag(&self) -> Option<HandTag> {
        let (arm, target, record) = (self.arm?, self.target.as_ref()?, self.record.as_ref()?);
        Some(HandTag { target: target.id.clone(), arm: arm.as_str().to_string(), record: record.to_string() })
    }

    /// Whether the production fits must skip this hand.
    pub fn is_treatment(&self) -> bool {
        self.arm == Some(Arm::Treatment)
    }
}

fn now_secs() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// The policy of `slot` for `hand_id`, fixed at the hand's first call and kept for the whole hand.
/// Ordinary unless the mode is active and fresh, the bot is in the pair and a target is safe.
pub fn latch(shared: &Shared, slot: usize, hand_id: &str) -> HandPolicy {
    if let Some(p) = shared.bots[slot].read().hand_policy.as_ref().filter(|p| p.hand_id == hand_id) {
        return p.clone();
    }
    let name = shared.bots[slot].read().name.clone();
    let now = now_secs();
    // One champion is what the pair tests against; with bots on lineages of their own there is none (ADR 0002).
    let lineages = shared.bots.iter().any(|b| b.read().slot_params.is_some());
    let mut policy = if lineages {
        HandPolicy::ordinary(hand_id)
    } else {
        let mut live = shared.experiment.write();
        decide_policy(&mut live, &name, hand_id, now, &shared.champion_version.read(), shared.season().and_then(|s| s.id))
    };
    if let Some(record) = policy.record.as_mut() {
        let net = shared.nn.read().as_deref().and_then(crate::replay::net_digest).map(|(d, _)| d);
        record["models"] = model_digests(&shared.params.read(), net.as_deref());
    }
    let changed = {
        let mut live = shared.experiment.write();
        match policy.arm {
            Some(arm) if live.last_assignment.get(&name).map(|(_, a)| *a) != Some(arm) => {
                live.last_assignment.insert(name.clone(), (now, arm));
                Some(arm)
            }
            None => {
                live.last_assignment.remove(&name);
                None
            }
            _ => None,
        }
    };
    if let (Some(arm), Some(t)) = (changed, &policy.target) {
        shared.log(&name, "info", format!("experiment: {} arm for {} (from this hand)", arm.provenance(), t.label()));
    }
    shared.bots[slot].write().hand_policy = Some(policy.clone());
    policy
}

/// The pure part of [`latch`]: the arm for `name`'s next hand, counting it on the target.
pub fn decide_policy(live: &mut Live, name: &str, hand_id: &str, now: f64, champion: &str, season: Option<String>) -> HandPolicy {
    if !live.mode.active_at(now) || live.store_error.is_some() {
        return HandPolicy::ordinary(hand_id);
    }
    let (Some(index), Some(target)) = (live.mode.pair.iter().position(|p| p == name), live.target.clone()) else {
        return HandPolicy::ordinary(hand_id);
    };
    let played = live.hands_on_target.entry(name.to_string()).or_insert(0);
    let arm = arm_for(index, *played);
    *played += 1;
    let record = json!({
        "provenance": arm.provenance(),
        "target": target.id,
        "hypothesis": format!("{} beats the champion live", target.label()),
        "arm": arm.as_str(),
        "assignment": {"rule": "alternating blocks", "block_hands": evidence::BLOCK_HANDS, "probability": 0.5, "pair_index": index},
        "bot": name,
        "season": season.or_else(|| live.mode.season.clone()),
        "champion": champion,
        "challenger": target.label(),
        "population_watermark": live.queue.as_ref().map(|q| q.refit_rowid),
        "ts": now,
    });
    HandPolicy { hand_id: hand_id.to_string(), arm: Some(arm), target: Some(target), record: Some(record) }
}

/// Digests of the models a decision used, for the provenance record: the response net, the range
/// model and the self-calibration table.
pub fn model_digests(params: &Params, net: Option<&str>) -> Value {
    let digest = |v: &Value| sv10_digest::hex(sv10_digest::Sha256::digest(v.to_string().as_bytes()))[..16].to_string();
    let calibration: std::collections::BTreeMap<&String, &f64> = params.ev_bias.iter().collect();
    json!({
        "net": net,
        "range": digest(&serde_json::to_value(params.range).unwrap_or(Value::Null)),
        "calibration": digest(&json!(calibration)),
    })
}

/// Re-read the mode, the learner's queue and the verdicts, pick the target and recount the
/// pair's hands on it. A store failure stops the experiment until a later refresh succeeds.
pub fn refresh(shared: &Shared) {
    let read = || -> anyhow::Result<(Option<ModeState>, Option<TargetQueue>, Verdicts)> {
        let parse = |key: &str| -> anyhow::Result<Option<String>> { shared.store.get_kv(key) };
        let mode = parse(MODE_KEY)?.and_then(|s| serde_json::from_str(&s).ok());
        let queue = parse(TARGETS_KEY)?.and_then(|s| serde_json::from_str(&s).ok());
        let verdicts = parse(VERDICTS_KEY)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Ok((mode, queue, verdicts))
    };
    let (mode, queue, verdicts) = match read() {
        Ok(v) => v,
        Err(e) => {
            fail_closed(shared, format!("experiment state unreadable: {e}"));
            return;
        }
    };
    let champion = shared.champion_version.read().clone();
    let selected = target::select(queue.as_ref(), &champion, |id| verdicts.contains_key(id)).cloned();
    let (target, no_target) = match selected {
        Ok(t) => (Some(t), None),
        Err(why) => (None, Some(why)),
    };
    let mut counts = HashMap::new();
    let mut estimate = None;
    if let Some(t) = &target {
        match shared.store.target_hands(&t.id) {
            Ok(hands) => {
                for h in &hands {
                    *counts.entry(h.bot.clone()).or_insert(0u64) += 1;
                }
                estimate = Some(Estimate::of(&hands));
            }
            Err(e) => {
                fail_closed(shared, format!("experiment hands unreadable: {e}"));
                return;
            }
        }
    }
    let mut live = shared.experiment.write();
    let same_target = live.target.as_ref().map(|t| &t.id) == target.as_ref().map(|t| &t.id);
    if let Some(mode) = mode {
        live.mode = mode;
    }
    // Latched hands not yet stored still count, so a bot does not replay a block it just played.
    if same_target {
        for (bot, stored) in counts.iter_mut() {
            *stored = (*stored).max(live.hands_on_target.get(bot).copied().unwrap_or(0));
        }
        for (bot, n) in &live.hands_on_target {
            counts.entry(bot.clone()).or_insert(*n);
        }
    }
    live.hands_on_target = counts;
    live.target = target.map(Arc::new);
    live.no_target = no_target;
    live.queue = queue;
    live.verdicts = verdicts;
    live.estimate = estimate;
    live.store_error = None;
}

fn fail_closed(shared: &Shared, why: String) {
    let mut live = shared.experiment.write();
    if live.store_error.is_none() {
        shared.log("fleet", "warn", format!("{why}; experiment bots play the champion"));
    }
    live.store_error = Some(why);
}

/// One poll: read the official board, advance the mode, store it, and check the running target's
/// live evidence against its gates. Run by the fleet's head (or its only process).
pub async fn poll_once(shared: &Arc<Shared>, http: &reqwest::Client, mode: &mut ModeState) {
    let fleet: Vec<String> = shared.bots.iter().map(|b| b.read().name.clone()).collect();
    let base = &shared.config.rest_base;
    let fetch = async {
        let get = |url: String| async move {
            http.get(&url)
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|e| format!("{} ({e})", url.rsplit('/').next().unwrap_or("")))?
                .json::<Value>()
                .await
                .map_err(|_| "unreadable JSON".to_string())
        };
        let season = get(format!("{base}/season/current")).await?;
        let board = get(format!("{base}/season/leaderboard?sort_by=score&min_hands=10&limit=1000")).await?;
        Ok::<_, String>((season, board))
    }
    .await;
    let now = now_secs();
    let reading = fetch.and_then(|(season, board)| Reading::parse(&season, &board, &fleet, now));
    for why in advance(mode, reading, now) {
        shared.log("fleet", "info", format!("experiment mode {}: {why}", if mode.status == Status::Active { "on" } else { "off" }));
    }
    store_mode(shared, mode);
    check_evidence(shared).await;
}

/// Advance the mode by one poll attempt at `now`, returning the reasons it changed.
///
/// The staleness check runs *first* (0327). A poll that fails already ends the mode inside
/// `observe`, so the stored mode could only stay `Active` on a reading hours old when no poll
/// happened at all — a stalled loop — and that is exactly the state nobody was recording: `expire`
/// had no caller outside its own test, so `review experiment` kept printing `Active` and no log
/// line ever said the experiment had ended, while play had already fallen back to the champion.
fn advance(mode: &mut ModeState, reading: Result<Reading, String>, now: f64) -> Vec<String> {
    let mut said = Vec::new();
    if let Some(why) = mode.expire(now) {
        said.push(why);
    }
    // The reading is applied either way: a poll that answers after a stall is how the mode
    // requalifies, and `observe` clears the qualifying run itself when it cannot be used.
    if let Some(why) = mode.observe(reading, now) {
        said.push(why);
    }
    said
}

/// Persist the mode for every process; a failed write ends the mode (a stale reading fails closed).
pub fn store_mode(shared: &Shared, mode: &ModeState) {
    match serde_json::to_string(mode).map_err(anyhow::Error::from).and_then(|s| shared.store.put_kv(MODE_KEY, &s)) {
        Ok(()) => shared.experiment.write().mode = mode.clone(),
        Err(e) => fail_closed(shared, format!("experiment mode not stored: {e}")),
    }
}

/// Record a verdict for the running target when its estimate reached a gate.
async fn check_evidence(shared: &Arc<Shared>) {
    let s = shared.clone();
    crate::jobs::blocking("experiment evidence", move || {
        refresh(&s);
        let (target, estimate, mut verdicts) = {
            let live = s.experiment.read();
            (live.target.clone(), live.estimate.clone(), live.verdicts.clone())
        };
        let (Some(target), Some(estimate)) = (target, estimate) else { return };
        let Some(verdict) = estimate.verdict() else { return };
        verdicts.insert(target.id.clone(), VerdictRecord { verdict, at: now_secs(), label: target.label(), estimate: estimate.clone() });
        match serde_json::to_string(&verdicts).map_err(anyhow::Error::from).and_then(|j| s.store.put_kv(VERDICTS_KEY, &j)) {
            Ok(()) => {
                s.log(
                    "fleet",
                    "info",
                    format!(
                        "experiment verdict {}: {} ({:+.1} bb/100, 95% {:+.1}..{:+.1}, {} hands per arm)",
                        serde_json::to_value(verdict).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default(),
                        target.label(),
                        estimate.diff_bb100,
                        estimate.lower_bb100,
                        estimate.upper_bb100,
                        estimate.effective_hands
                    ),
                );
                refresh(&s);
            }
            Err(e) => fail_closed(&s, format!("experiment verdict not stored: {e}")),
        }
    })
    .await;
}

/// The dashboard's view of the experiment (`GET /api/experiment`).
pub fn view(shared: &Shared, now: f64) -> Value {
    let live = shared.experiment.read().clone();
    let m = &live.mode;
    let running = m.active_at(now) && live.store_error.is_none() && live.target.is_some();
    let status = if running {
        "running"
    } else if m.active_at(now) {
        "active_no_target"
    } else {
        "champion"
    };
    let bots: Vec<Value> = m
        .pair
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let played = live.hands_on_target.get(name).copied().unwrap_or(0);
            let arm = running.then(|| live.last_assignment.get(name).map(|(_, a)| *a).unwrap_or_else(|| arm_for(i, played)));
            json!({
                "name": name,
                "role": arm.map(|a| if a == Arm::Treatment { "treatment" } else { "champion control" }),
                "hands_on_target": played,
                "last_assignment_change": live.last_assignment.get(name).map(|(t, _)| *t),
            })
        })
        .collect();
    let target = live.target.as_ref().filter(|_| running).map(|t| {
        json!({
            "id": t.id, "label": t.label(), "knob": t.knob, "old": t.old, "new": t.new,
            "hypothesis": format!("{} beats the champion live", t.label()),
            "source": t.source,
            "sim": {"hands": t.sim.hands, "mean_bb100": t.sim.mean_bb * 100.0, "upper_bb100": t.sim.upper_95() * 100.0},
            "champion": live.queue.as_ref().map(|q| q.champion.clone()),
            "population_watermark": live.queue.as_ref().map(|q| q.refit_rowid),
            "estimate": live.estimate,
            "required_hands": evidence::BOUNDARY_HANDS,
            "min_hands": evidence::MIN_EFFECTIVE_HANDS,
            "next_gate": live.estimate.as_ref().map(Estimate::next_gate),
            "safety_bound": format!("retire when the 95% upper bound is below 0 bb/100 after {} hands per arm", evidence::MIN_EFFECTIVE_HANDS),
            "review": format!("review experiment {}", t.id),
        })
    });
    let verdicts: Vec<Value> = live
        .verdicts
        .iter()
        .rev()
        .take(10)
        .map(|(id, v)| json!({"id": id, "label": v.label, "verdict": v.verdict, "at": v.at, "estimate": v.estimate}))
        .collect();
    let reason = if running {
        None
    } else if m.active_at(now) {
        live.store_error.clone().or(live.no_target.clone())
    } else {
        m.unavailable().or_else(|| m.last_transition.as_ref().map(|t| t.reason.clone()))
    };
    json!({
        "status": status,
        "season": m.season,
        "qualifying": {"readings": m.qualifying.len(), "needed": mode::QUALIFYING_READINGS},
        "protected": m.protected,
        "pair": m.pair,
        "ranks": m.ranks,
        "last_reading_at": m.last_reading_at,
        "last_attempt_at": m.last_attempt_at,
        "reading_age_secs": m.last_reading_at.map(|t| now - t),
        "stale_after_secs": mode::STALE_AFTER_SECS,
        "last_error": m.last_error,
        "last_transition": m.last_transition,
        "reason": reason,
        "bots": bots,
        "target": target,
        "verdicts": verdicts,
    })
}

/// `review experiment [TARGET]`: without a target, every target with live hands and the stored
/// mode; with one, its estimate and gate, then every hand with its arm, result, decision ids and
/// replay ids (`review replay id=N` re-runs a recorded decision bit for bit).
///
/// `json` (#723) prints one JSON object instead of the lines: the stored `ModeState`, the `Verdicts`
/// and each target's `Estimate` are the structs the text is formatted from, so an agent reads the
/// same numbers.
pub fn review(store: &sv10_store::store::Store, target: Option<&str>, out: &mut impl std::io::Write, json: bool) -> anyhow::Result<()> {
    let Some(target) = target else {
        let mode: Option<ModeState> = store.get_kv(MODE_KEY)?.and_then(|s| serde_json::from_str(&s).ok());
        if json {
            let verdicts = store.get_kv(VERDICTS_KEY)?.and_then(|s| serde_json::from_str::<Verdicts>(&s).ok()).unwrap_or_default();
            writeln!(out, "{}", json!({"mode": mode, "verdicts": verdicts, "targets": store.experiment_targets()?}))?;
            return Ok(());
        }
        match mode {
            Some(m) => writeln!(
                out,
                "mode {:?} · season {} · pair {} · protected {} · last reading {} · {}",
                m.status,
                m.season.as_deref().unwrap_or("?"),
                m.pair.join(", "),
                m.protected.join(", "),
                m.last_reading_at.map_or("never".into(), |t| format!("{t:.0}")),
                m.last_transition.map_or(String::new(), |t| t.reason)
            )?,
            None => writeln!(out, "mode: never polled")?,
        }
        let verdicts: Verdicts = store.get_kv(VERDICTS_KEY)?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        for (target, arm, hands, first, last) in store.experiment_targets()? {
            let verdict = verdicts.get(&target).map_or("running or open".to_string(), |v| format!("{:?}", v.verdict));
            writeln!(out, "{target}  {arm:9} {hands:6} hands  {first} .. {last}  {verdict}")?;
        }
        return Ok(());
    };
    let hands = store.target_hands(target)?;
    let e = Estimate::of(&hands);
    if json {
        let mut rows = Vec::new();
        for h in &hands {
            let (decisions, replays) = store.hand_audit_ids(&h.bot, &h.hand_id)?;
            rows.push(json!({"ts": h.ts, "bot": h.bot, "hand_id": h.hand_id, "arm": h.arm, "net": h.net,
                "decisions": decisions, "replays": replays}));
        }
        writeln!(out, "{}", json!({"target": target, "estimate": e, "next_gate": e.next_gate(), "verdict": e.verdict(), "hands": rows}))?;
        return Ok(());
    }
    writeln!(
        out,
        "{target}: treatment − control {:+.1} bb/100 (95% {:+.1}..{:+.1}); {} treatment / {} control hands; {}; verdict {:?}",
        e.diff_bb100,
        e.lower_bb100,
        e.upper_bb100,
        e.treatment.hands,
        e.control.hands,
        e.next_gate(),
        e.verdict()
    )?;
    for h in &hands {
        let (decisions, replays) = store.hand_audit_ids(&h.bot, &h.hand_id)?;
        writeln!(
            out,
            "{} {:12} {:9} {} net {:>7} decisions {:?} replays {:?}",
            h.ts,
            h.bot,
            h.arm,
            h.hand_id,
            h.net.map_or("?".into(), |n| n.to_string()),
            decisions,
            replays
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;

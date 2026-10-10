//! Turning a `your_turn` into a legal action: the policy call with its timeout, legalization
//! against the server's offer, and the decision record.

use super::*;

/// Map the policy's action onto what the server actually offers this turn.
pub fn legalize(action: Action, legal: &LegalActions) -> (String, Option<i64>) {
    let passive = || {
        if legal.can_check { ("check".to_string(), None) } else { ("fold".to_string(), None) }
    };
    let call = || {
        if legal.can_check {
            ("check".to_string(), None)
        } else if legal.call.is_some() {
            ("call".to_string(), None)
        } else {
            ("fold".to_string(), None)
        }
    };
    match action {
        Action::Fold => passive(),
        Action::Check => passive(),
        Action::Call => call(),
        Action::RaiseTo(t) => match (legal.raise_min, legal.raise_max) {
            (Some(lo), Some(hi)) => {
                // Never panic on a malformed offer (min above max): clamp into what is valid.
                let t = t.clamp(lo.min(hi), hi);
                if t >= hi && legal.all_in.is_some() { ("all_in".into(), None) } else { ("raise".into(), Some(t)) }
            }
            _ if legal.all_in.is_some() => ("all_in".into(), None),
            _ => call(),
        },
        Action::AllIn => {
            if legal.all_in.is_some() {
                ("all_in".into(), None)
            } else if let Some(hi) = legal.raise_max {
                ("raise".into(), Some(hi))
            } else {
                call()
            }
        }
    }
}

/// Whether this authority is a duplicate or older than the newest turn already sent for the hand.
fn already_answered(
    last_hand: Option<&str>,
    last_token: Option<&str>,
    last_seq: Option<i64>,
    hand_id: &str,
    token: &str,
    seq: Option<i64>,
) -> bool {
    if last_hand != Some(hand_id) {
        return false;
    }
    (!token.is_empty() && last_token == Some(token)) || matches!((seq, last_seq), (Some(current), Some(last)) if current < last)
}

/// The sent-action authority gate, also used before delaying a missing-state turn.
pub(super) fn turn_answered(shared: &Shared, slot: usize, tracker: &TableTracker, msg: &Value) -> bool {
    let hand_id = msg["hand_id"].as_str().or(tracker.hand_id.as_deref()).unwrap_or("");
    let token = msg["turn_token"].as_str().unwrap_or("");
    let seq = msg["table_seq"].as_i64();
    let prior = shared.bots[slot].read();
    already_answered(
        prior.last_acted_hand_id.as_deref(),
        prior.last_acted_turn_token.as_deref(),
        prior.last_acted_turn_seq,
        hand_id,
        token,
        seq,
    )
}

fn calibration_candidate<'a>(
    decision: &'a sv10_core::policy::Decision,
    action: &str,
    amount: Option<i64>,
) -> Option<&'a sv10_core::policy::Candidate> {
    decision.candidates.iter().find(|candidate| candidate.action == action && (action != "raise" || candidate.amount == amount))
}

/// The server auto-folds (or auto-checks) 45 s after it sends `your_turn` in public play and never
/// extends that clock, not for a resync nor a disconnect (spec, Timeouts). Decisions take ~0.1-0.4 s
/// (1.6M samples, 0161); one still running at this cap is stuck, so the safe action goes out with
/// over 35 s of the deadline left for the network and a rejection resync (0163).
const DECISION_CAP: Duration = Duration::from_secs(8);

/// Waits before each retry of a store write that failed on a locked database (0322): the first
/// attempt's 10 s busy timeout has just expired, so the holder is already long; a quarter second and
/// then seconds clear the stalls seen in the logs (one 55 s hold, 2026-09-27 07:33).
const LOCKED_WRITE_WAITS: [Duration; 3] = [Duration::from_millis(250), Duration::from_secs(2), Duration::from_secs(10)];

/// Run `write` again while it fails, waiting before each attempt, and return the last error (0322).
/// A write that gives up on `database is locked` loses its row for good otherwise: hands have a
/// retry queue (0247), the decision record and the analyst's audit do not, and 18 audits were lost
/// that way in ten days. Bounded by the waits it is given, and never on the decision path — callers
/// spawn it.
pub(crate) async fn retry_locked_write(waits: &[Duration], mut write: impl FnMut() -> anyhow::Result<()>) -> anyhow::Result<()> {
    let mut last = anyhow::anyhow!("no attempt made");
    for wait in waits {
        tokio::time::sleep(*wait).await;
        match write() {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// The blocking twin of [`retry_locked_write`], for callers already on the blocking pool (the
/// calibration round): same waits with `std::thread::sleep` so no runtime is needed (#744).
pub(crate) fn retry_locked_write_blocking(mut write: impl FnMut() -> anyhow::Result<()>) -> anyhow::Result<()> {
    let mut last = anyhow::anyhow!("no attempt made");
    for wait in LOCKED_WRITE_WAITS {
        std::thread::sleep(wait);
        match write() {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Retry a store write that failed on a locked database, off the frame loop, and report the outcome.
pub(crate) fn spawn_locked_write_retry(
    shared: &Arc<Shared>,
    bot: &str,
    what: &str,
    write: impl FnMut() -> anyhow::Result<()> + Send + 'static,
) {
    let (shared, bot, what) = (shared.clone(), bot.to_string(), what.to_string());
    tokio::spawn(async move {
        match retry_locked_write(&LOCKED_WRITE_WAITS, write).await {
            Ok(()) => shared.log(&bot, "info", format!("{what} stored on retry")),
            Err(e) => shared.log(&bot, "warn", format!("{what} lost after {} retries: {e}", LOCKED_WRITE_WAITS.len())),
        }
    });
}

pub(super) async fn act(
    shared: &Arc<Shared>,
    slot: usize,
    bot: &BotConfig,
    tracker: &mut TableTracker,
    conn: &Conn,
    msg: &Value,
    rng: &mut SmallRng,
) {
    let legal = LegalActions::parse(&msg["valid_actions"]);
    let token = msg["turn_token"].as_str().unwrap_or("").to_string();
    let hand_id = msg["hand_id"].as_str().map(String::from).or_else(|| tracker.hand_id.clone()).unwrap_or_default();
    let seq = msg["table_seq"].as_i64();
    // A (hand, token) is answered at most once, and a late older authority in the same hand cannot
    // act after a newer sequenced turn. Resync snapshots without a sequence still deduplicate by pair.
    let answered = turn_answered(shared, slot, tracker, msg);
    if answered {
        shared.log(&bot.name, "info", format!("turn token {token} re-delivered; skipping duplicate action"));
        return;
    }
    let started = Instant::now();
    let sit = tracker.situation(msg, &legal);
    let missing_state = sit.is_none();
    let sit_for_view = sit.clone();
    // The hand's policy was fixed at hand start (or here, for a hand joined by resync).
    let policy = crate::experiment::latch(shared, slot, &hand_id);
    let (name, amount, view, pending_calibration, replay) = match sit {
        Some(sit) => {
            let shared2 = shared.clone();
            let policy2 = policy.clone();
            let sit2 = sit.clone();
            let seed = sv10_rng::RngExt::random::<u64>(rng);
            let gate = shared.decision_gate.clone();
            // Set by the timeout arm below; the search's parallel chunk loops read it and stop (#750).
            let abandon = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let abandon_in = abandon.clone();
            // The permit moves into the search itself: a `spawn_blocking` task cannot be cancelled, so a search
            // that outlives `DECISION_CAP` keeps its permit until it really ends instead of freeing it for another.
            let job = async move {
                let permit = gate.acquire_owned().await.ok();
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    let _abandon = sv10_core::abandon::install(abandon_in);
                    // Snapshot, do not borrow: a `spawn_blocking` task cannot be cancelled, so a decision
                    // that outlives `DECISION_CAP` would keep the models' read lock for the rest of its
                    // search and block the frame loop's `models.write()` (and with it every hand's
                    // insert). A clone is ~305 small structs with the heavy per-opponent maps behind
                    // `Arc`, so it costs far less than the 45 ms search it replaces (0252).
                    let models = shared2.models.read().clone();
                    let mut live = shared2.params.read().clone();
                    if let Some(own) = shared2.bots[slot].read().slot_params.clone() {
                        live.adopt_promoted(own);
                    }
                    let params = policy2.params(&live);
                    let version = shared2.bots[slot].read().slot_version.clone().unwrap_or_else(|| shared2.champion_version.read().clone());
                    let nn = shared2.nn.read().clone();
                    let mut r = SmallRng::seed_from_u64(seed);
                    let d = decide_with(&sit2, &models, &params, nn.as_deref(), &mut r);
                    // Every decision keeps its full inputs: the analyst process re-solves it with a deep search,
                    // and big spots are also kept for bit-exact replay (`review replay`).
                    let net = nn.as_deref().and_then(crate::replay::net_digest);
                    let rec = crate::replay::record(&sit2, seed, &params, &models, net.as_ref().map(|n| n.0.clone()), &d);
                    let replay = (serde_json::to_string(&rec).unwrap_or_default(), net, crate::replay::is_big_spot(&sit2, &d));
                    let offsets: Vec<(String, f32)> =
                        sit2.live_opponents().filter_map(|p| Some((p.name.clone(), *models.fold_offsets.get(&p.name)?))).collect();
                    (d, version, replay, (params.fold_logit_shift, params.preflop_fold_logit_shift, offsets))
                })
                .await
            };
            match tokio::time::timeout(DECISION_CAP, job).await {
                Ok(Ok((d, version, replay, (fold_shift, preflop_fold_shift, fold_offsets)))) => {
                    let (n, a) = legalize(d.action, &legal);
                    let pending_calibration = calibration_candidate(&d, &n, a).and_then(|candidate| {
                        let cat = candidate.category.as_ref()?;
                        let predicted = candidate.ev - candidate.bias;
                        let scale = sit.pot + sit.call_amount.max(0);
                        if predicted.is_finite() && scale > 0 {
                            Some(sv10_venue::tracker::PendingCalibration {
                                category: cat.clone(),
                                predicted_incremental_chips: predicted,
                                hero_stack_before_action: sit.hero().stack,
                                scale_chips: scale,
                            })
                        } else {
                            None
                        }
                    });
                    let mut view =
                        DecisionView::for_decision(&hand_id, &sit, &d, n.clone(), a, started.elapsed().as_secs_f64() * 1000.0, version);
                    view.fold_shift = fold_shift;
                    view.preflop_fold_shift = preflop_fold_shift;
                    view.fold_offsets = fold_offsets;
                    view.experiment = policy.record.clone();
                    (n, a, Some(view), pending_calibration, Some(replay))
                }
                _ => {
                    abandon.store(true, std::sync::atomic::Ordering::Relaxed);
                    shared.log(&bot.name, "warn", "decision timed out or panicked; taking the safe action");
                    shared.update(slot, |b| b.decision_timeouts += 1);
                    let (n, a) = legalize(Action::Check, &legal);
                    (n, a, None, None, None)
                }
            }
        }
        None => {
            shared.log(&bot.name, "warn", "no situation (missing state); taking the safe action");
            let (n, a) = legalize(Action::Check, &legal);
            (n, a, None, None, None)
        }
    };
    if !msg["valid_actions"].as_array().is_some_and(|offer| offer.iter().any(|entry| entry["action"].as_str() == Some(name.as_str()))) {
        shared.log(&bot.name, "warn", format!("action {name} is absent from valid_actions; awaiting fresh authority"));
        return;
    }
    let mut out = json!({
        "type": "action",
        "action": name,
        "hand_id": hand_id,
        "turn_token": token,
        "client_action_id": sv10_rt::uuid_v4(),
    });
    if let Some(a) = amount {
        out["amount"] = json!(a);
    }
    let sent = conn.send(out);
    if sent {
        if let Some(pending) = pending_calibration {
            tracker.pending_calibration.push(pending);
        }
        if !token.is_empty() {
            shared.update(slot, |b| {
                b.last_acted_hand_id = Some(hand_id.clone());
                b.last_acted_turn_token = Some(token);
                if let Some(seq) = seq {
                    b.last_acted_turn_seq = Some(seq);
                }
            });
        }
    } else {
        // Not recorded as answered: after the reconnect the resynced turn is answered again.
        shared.log(&bot.name, "warn", format!("action {name} for hand {hand_id} not sent: writer closed"));
    }
    if sent && missing_state {
        recover::resync_missing_state(shared, slot, tracker, conn, &hand_id);
    }
    if sent
        && let Some((record, net, big)) = replay
        && !record.is_empty()
    {
        let net_ref = net.as_ref().map(|(digest, json)| (digest.as_str(), json.as_str()));
        if big && let Err(e) = shared.store.insert_replay(&bot.name, &hand_id, &record, net_ref) {
            shared.log(&bot.name, "warn", format!("replay record not stored: {e}"));
            let (name, hand, rec) = (bot.name.clone(), hand_id.clone(), record.clone());
            let net_owned = net_ref.map(|(digest, json)| (digest.to_string(), json.to_string()));
            let shared2 = shared.clone();
            spawn_locked_write_retry(shared, &bot.name, "the replay record", move || {
                shared2.store.insert_replay(&name, &hand, &rec, net_owned.as_ref().map(|(d, j)| (d.as_str(), j.as_str())))
            });
        }
        if let Err(e) = shared.store.insert_audit(&bot.name, &hand_id, &record, net_ref) {
            shared.log(&bot.name, "warn", format!("decision not queued for the analyst: {e}"));
            let (name, hand, rec) = (bot.name.clone(), hand_id.clone(), record.clone());
            let net_owned = net_ref.map(|(digest, json)| (digest.to_string(), json.to_string()));
            let shared2 = shared.clone();
            spawn_locked_write_retry(shared, &bot.name, "the analyst audit", move || {
                shared2.store.insert_audit(&name, &hand, &rec, net_owned.as_ref().map(|(d, j)| (d.as_str(), j.as_str())))
            });
        }
    }
    if let Some(v) = view.filter(|_| sent) {
        let detail = json!({"reason": v.reason, "candidates": v.candidates, "hole": v.hole, "board": v.board, "opponents": v.opponents, "pot_odds": v.pot_odds, "version": v.version, "fold_shift": v.fold_shift, "preflop_fold_shift": v.preflop_fold_shift, "fold_offsets": v.fold_offsets, "experiment": v.experiment});
        if let Err(e) = shared.store.insert_decision(
            &bot.name,
            &v.hand_id,
            &v.street,
            &v.action,
            v.amount,
            v.equity,
            v.pot,
            v.to_call,
            v.latency_ms,
            &detail.to_string(),
        ) {
            shared.log(&bot.name, "warn", format!("decision record not stored: {e}"));
            let (name, hand, street, action, detail) =
                (bot.name.clone(), v.hand_id.clone(), v.street.clone(), v.action.clone(), detail.to_string());
            let (amount, equity, pot, to_call, latency) = (v.amount, v.equity, v.pot, v.to_call, v.latency_ms);
            let shared2 = shared.clone();
            spawn_locked_write_retry(shared, &bot.name, "the decision record", move || {
                shared2.store.insert_decision(&name, &hand, &street, &action, amount, equity, pot, to_call, latency, &detail)
            });
        }
        let latency = v.latency_ms;
        shared.emit("decision", json!({"slot": slot, "bot": bot.name, "action": v.action, "amount": v.amount, "equity": v.equity, "street": v.street, "latency_ms": v.latency_ms}));
        shared.update(slot, |b| {
            b.decisions += 1;
            b.latencies_ms.push_back(latency);
            while b.latencies_ms.len() > 200 {
                b.latencies_ms.pop_front();
            }
            b.last_decision = Some(v);
            b.last_situation = sit_for_view.map(Arc::new);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0252: a `spawn_blocking` decision cannot be cancelled, so it used to keep the models' read
    /// lock for the rest of a search that outlived the 8 s cap — blocking the frame loop's
    /// `models.write()` and with it every hand insert. The job now snapshots what it needs, so a
    /// writer is never held up by a decision.
    #[test]
    fn a_decision_snapshots_the_models_instead_of_holding_their_read_lock() {
        let source = include_str!("decide.rs");
        let body = source.split("#[cfg(test)]").next().unwrap();
        assert!(body.contains("shared2.models.read().clone()"), "the job must snapshot the models");
        assert!(
            !body.lines().any(|l| l.trim_start().starts_with("let models =") && l.contains(".read()") && !l.contains("clone")),
            "a bare models.read() guard in the job outlives the timeout and blocks the frame loop's write"
        );
    }

    /// 0322: a store write that gave up on `database is locked` is tried again (a quarter second
    /// then seconds later), and only a write that fails every attempt is reported lost.
    #[tokio::test]
    async fn a_locked_write_is_retried_and_a_persistent_failure_is_reported() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let tries = Arc::new(AtomicUsize::new(0));
        let count = tries.clone();
        let waits = [Duration::ZERO; LOCKED_WRITE_WAITS.len()];
        let landed =
            retry_locked_write(
                &waits,
                move || {
                    if count.fetch_add(1, Ordering::SeqCst) < 2 { anyhow::bail!("database is locked") } else { Ok(()) }
                },
            )
            .await;
        assert!(landed.is_ok(), "the third attempt lands: {landed:?}");
        assert_eq!(tries.load(Ordering::SeqCst), 3);
        let lost = retry_locked_write(&waits, || anyhow::bail!("database is locked")).await.unwrap_err();
        assert!(lost.to_string().contains("database is locked"), "{lost}");
    }

    /// 0322 wiring: both rows that are otherwise lost for good hand the failed write to the retry
    /// (the loop above is tested; a real `database is locked` costs the 10 s busy timeout before it
    /// fails, too slow for the gate).
    #[test]
    fn a_locked_decision_record_or_audit_is_handed_to_the_retry() {
        let source = include_str!("decide.rs");
        let body = source.split("#[cfg(test)]").next().unwrap();
        for (lost, what) in [
            ("decision not queued for the analyst", "the analyst audit"),
            ("decision record not stored", "the decision record"),
            ("replay record not stored", "the replay record"),
        ] {
            let at = body.find(lost).unwrap_or_else(|| panic!("{lost} must be logged"));
            let branch = &body[at..body.len().min(at + 800)];
            assert!(branch.contains(&format!("spawn_locked_write_retry(shared, &bot.name, \"{what}\"")), "{lost} must be retried");
        }
    }

    fn legal(check: bool, call: Option<i64>, raise: Option<(i64, i64)>, all_in: Option<i64>) -> LegalActions {
        LegalActions { can_fold: true, can_check: check, call, raise_min: raise.map(|r| r.0), raise_max: raise.map(|r| r.1), all_in }
    }

    #[test]
    fn legalize_maps_to_offered_actions() {
        let l = legal(false, Some(20), Some((40, 1000)), Some(1000));
        assert_eq!(legalize(Action::RaiseTo(10), &l), ("raise".into(), Some(40)));
        assert_eq!(legalize(Action::RaiseTo(5000), &l), ("all_in".into(), None));
        assert_eq!(legalize(Action::Check, &l), ("fold".into(), None));
        let free = legal(true, None, Some((20, 500)), None);
        assert_eq!(legalize(Action::Fold, &free), ("check".into(), None));
        assert_eq!(legalize(Action::Call, &free), ("check".into(), None));
        assert_eq!(legalize(Action::AllIn, &free), ("raise".into(), Some(500)));
        // Malformed offer with min above max must not panic.
        let bad = legal(false, Some(20), Some((500, 300)), None);
        assert_eq!(legalize(Action::RaiseTo(400), &bad), ("raise".into(), Some(300)));
        let short = legal(false, Some(300), None, None);
        assert_eq!(legalize(Action::RaiseTo(900), &short), ("call".into(), None));
        assert_eq!(legalize(Action::AllIn, &short), ("call".into(), None));
    }

    #[test]
    fn calibration_prediction_and_realization_share_the_pre_action_frontier() {
        let before = 1_000;
        let call = 200.0;
        let predicted_losing_call = 0.0 * (500.0 + call) - call;
        let realized_losing_call = sv10_venue::tracker::realized_incremental_chips(800, before) as f64;
        assert_eq!(predicted_losing_call, -200.0);
        assert_eq!(realized_losing_call, predicted_losing_call);
    }

    /// Invariant "never send an action absent from valid_actions", over random offers (including
    /// malformed ones with min above max) and every intended action.
    #[test]
    fn legalize_only_returns_offered_actions() {
        use sv10_rng::rngs::SmallRng;
        use sv10_rng::{RngExt, SeedableRng};
        let cases: u64 = std::env::var("SV10_PROP_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(20_000);
        let mut rng = SmallRng::seed_from_u64(0x1e6a1);
        for case in 0..cases {
            let can_check = rng.random_bool(0.4);
            let call = (!can_check && rng.random_bool(0.8)).then(|| rng.random_range(1..=5_000));
            let raise = rng.random_bool(0.7).then(|| (rng.random_range(1..=10_000), rng.random_range(1..=10_000)));
            let all_in = rng.random_bool(0.6).then(|| rng.random_range(1..=10_000));
            let l = legal(can_check, call, raise, all_in);
            let intended = match rng.random_range(0..5) {
                0 => Action::Fold,
                1 => Action::Check,
                2 => Action::Call,
                3 => Action::RaiseTo(rng.random_range(-100..=20_000)),
                _ => Action::AllIn,
            };
            let (name, amount) = legalize(intended, &l);
            let ok = match name.as_str() {
                "check" => l.can_check && amount.is_none(),
                "fold" => !l.can_check && l.can_fold && amount.is_none(),
                "call" => l.call.is_some() && amount.is_none(),
                "all_in" => l.all_in.is_some() && amount.is_none(),
                "raise" => match (l.raise_min, l.raise_max, amount) {
                    (Some(lo), Some(hi), Some(t)) => t >= lo.min(hi) && t <= hi,
                    _ => false,
                },
                _ => false,
            };
            assert!(ok, "case {case}: {intended:?} on {l:?} became {name} {amount:?}");
        }
    }

    #[test]
    fn only_a_nonempty_matching_token_counts_as_answered() {
        assert!(already_answered(Some("h1"), Some("t1"), Some(10), "h1", "t1", Some(10)));
        assert!(already_answered(Some("h1"), Some("t2"), Some(12), "h1", "old", Some(11)));
        assert!(!already_answered(Some("h1"), Some("t1"), Some(10), "h1", "t2", Some(11)));
        assert!(!already_answered(Some("h1"), Some("t1"), Some(10), "h2", "t1", Some(9)));
        assert!(!already_answered(None, None, None, "h1", "t1", None));
    }
}

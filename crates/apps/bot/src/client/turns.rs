//! Let missing state recover while the session keeps reading frames; never extend turn authority.
use super::*;

/// One turn waiting for a snapshot, owned by this connection's seat lifecycle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PendingTurn {
    table: String,
    frame: Value,
    pub(super) deadline: Instant,
}

const RECOVERY_WAIT: Duration = Duration::from_secs(2);

fn hand<'a>(frame: &'a Value, tracker: &'a TableTracker) -> &'a str {
    frame["hand_id"].as_str().or(tracker.hand_id.as_deref()).unwrap_or("")
}

/// A definitive end of authority cancels the wait, including a snapshot with no active token.
pub(super) fn observe(pending: &mut Option<PendingTurn>, tracker: &TableTracker, msg: &Value) {
    let Some(turn) = pending.as_ref() else { return };
    let same_hand = hand(msg, tracker) == hand(&turn.frame, tracker);
    let ended = match msg["type"].as_str().unwrap_or("") {
        "table_closed" | "left_table" | "table_joined" => true,
        "hand_start" => !same_hand,
        "hand_result" => same_hand,
        "player_action" => same_hand && msg["seat"].as_u64().map(|s| s as usize) == tracker.hero_seat,
        "resync_response" => msg["role"] == "spectator" || !msg["snapshot"]["hero"]["turn_token"].is_string(),
        _ => false,
    };
    if ended {
        *pending = None;
    }
}

/// Duplicate frames preserve the original deadline. A newer authority supersedes the old wait.
// These are the session's existing borrows; bundling them would only rename the same state.
#[allow(clippy::too_many_arguments)]
pub(super) async fn receive(
    shared: &Arc<Shared>,
    slot: usize,
    bot: &BotConfig,
    tracker: &mut TableTracker,
    conn: &Conn,
    msg: &Value,
    rng: &mut SmallRng,
    pending: &mut Option<PendingTurn>,
) {
    if decide::turn_answered(shared, slot, tracker, msg) {
        return;
    }
    let previous = pending.as_ref().filter(|old| tracker.table_id.as_deref() == Some(&old.table));
    if previous
        .is_some_and(|old| matches!((msg["table_seq"].as_i64(), old.frame["table_seq"].as_i64()), (Some(new), Some(prior)) if new < prior))
    {
        return;
    }
    let legal = LegalActions::parse(&msg["valid_actions"]);
    if !legal.can_fold
        && !legal.can_check
        && legal.call.is_none()
        && legal.all_in.is_none()
        && legal.raise_min.zip(legal.raise_max).is_none()
    {
        *pending = None;
        shared.log(&bot.name, "warn", "turn has no usable legal offer; awaiting fresh authority");
        return;
    }
    if let Some(old) = previous
        && hand(msg, tracker) == hand(&old.frame, tracker)
        && msg["turn_token"] == old.frame["turn_token"]
    {
        let sequence = old.frame["table_seq"].clone();
        let old = pending.as_mut().expect("pending authority was checked");
        old.frame = msg.clone();
        if old.frame["table_seq"].is_null() {
            old.frame["table_seq"] = sequence;
        }
        return;
    }
    *pending = None;
    if tracker.situation(msg, &legal).is_none()
        && !hand(msg, tracker).is_empty()
        && msg["turn_token"].as_str().is_some_and(|s| !s.is_empty())
        && recover::resync_missing_state(shared, slot, tracker, conn, hand(msg, tracker))
    {
        let mut frame = msg.clone();
        frame["hand_id"] = json!(hand(msg, tracker));
        *pending = Some(PendingTurn {
            table: tracker.table_id.clone().expect("queued resync has a table"),
            frame,
            deadline: Instant::now() + RECOVERY_WAIT,
        });
        shared.log(&bot.name, "warn", "missing decision state; waiting up to 2 seconds for resync before fallback");
    } else {
        act(shared, slot, bot, tracker, conn, msg, rng).await;
    }
}

/// Complete on recovered state or the original deadline, never on an expired table or hand.
// These are the session's existing borrows; bundling them would only rename the same state.
#[allow(clippy::too_many_arguments)]
pub(super) async fn finish(
    shared: &Arc<Shared>,
    slot: usize,
    bot: &BotConfig,
    tracker: &mut TableTracker,
    conn: &Conn,
    rng: &mut SmallRng,
    pending: &mut Option<PendingTurn>,
    now: Instant,
) {
    let Some(turn) = pending.as_ref() else { return };
    if tracker.table_id.as_deref() != Some(&turn.table) || tracker.hand_id.as_deref().is_some_and(|id| id != hand(&turn.frame, tracker)) {
        *pending = None;
        return;
    }
    let legal = LegalActions::parse(&turn.frame["valid_actions"]);
    if now >= turn.deadline || tracker.situation(&turn.frame, &legal).is_some() {
        let turn = pending.take().expect("pending turn was checked");
        act(shared, slot, bot, tracker, conn, &turn.frame, rng).await;
    }
}

/// No pending turn leaves this select branch asleep; an active wait has its own timer.
pub(super) async fn wait(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await,
        None => std::future::pending::<()>().await,
    }
}

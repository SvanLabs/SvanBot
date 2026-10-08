//! Storing a finished hand, live or recovered (0315). A hand whose `hand_result` the bot missed — the
//! process restarted mid-hand at a hot swap, or the socket dropped before the result — arrives again in
//! the resync replay; it is stored then, once, unless the store already has it.

use super::*;

/// Recover impossible local decision state after the legal fallback has been queued. Keep the
/// token answered, and request at most once per table/hand: resync can restore private state for
/// later turns, but an incomplete snapshot must not feed a request loop.
pub(super) fn resync_missing_state(shared: &Shared, slot: usize, tracker: &TableTracker, conn: &Conn, hand: &str) {
    let Some(table) = &tracker.table_id else { return };
    let key = (table.clone(), hand.to_string());
    let mut bot = shared.bots[slot].write();
    if bot.missing_state_resync.as_ref() != Some(&key)
        && conn.send(json!({"type": "resync_request", "table_id": table, "last_table_seq": tracker.last_table_seq.max(0)}))
    {
        bot.missing_state_resync = Some(key);
    }
}

/// Store a finished hand and feed it to the opponent models and the fleet's image of our own play
/// (0321); a treatment-arm experiment hand is stored with its provenance and kept out of both and out
/// of the self-calibration table (0291). Returns the stored row.
pub(super) fn store_finished(
    shared: &Shared,
    slot: usize,
    bot: &BotConfig,
    tracker: &mut TableTracker,
    f: &sv10_venue::tracker::FinishedHand,
) -> HandRow {
    let row = HandRow {
        bot: bot.name.clone(),
        hand_id: f.hand_id.clone(),
        table_id: tracker.table_id.clone().unwrap_or_default(),
        ended_at: chrono::Utc::now().to_rfc3339(),
        hero_seat: f.hero_seat.map(|s| s as i64),
        hole: f.hero_hole.map(|h| format!("{}{}", h[0], h[1])).unwrap_or_default(),
        board: f.board.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(""),
        pot: f.pot,
        net: f.hero_net,
        winners: f.winners.join(","),
        summary: serde_json::to_string(&f.summary).unwrap_or_default(),
        showdown: f.hero_seat.map(|h| f.summary.shown.iter().any(|(s, _)| *s == h)).unwrap_or(false),
    };
    let policy = shared.bots[slot].read().hand_policy.clone().filter(|p| p.hand_id == f.hand_id);
    let tag = policy.as_ref().and_then(|p| p.tag());
    let treatment = policy.as_ref().is_some_and(|p| p.is_treatment());
    // Persist first; the model watermark then lets a restart replay anything unsaved.
    match shared.store.insert_hand_tagged(&row, tag.as_ref()) {
        Ok(rowid) => {
            let mut models = shared.models.write();
            if !treatment {
                models.observe_own_hand(&f.summary, &bot.name, 1.0);
            }
            models.watermark = Some(models.watermark.unwrap_or(0).max(rowid));
        }
        Err(e) => {
            shared.log(&bot.name, "error", format!("store hand failed: {e}; queued for retry"));
            if let Some(tag) = &tag
                && let Err(e) = shared.store.tag_hand(&row.bot, &row.hand_id, tag)
            {
                shared.log(&bot.name, "warn", format!("experiment provenance of hand {} not stored: {e}", row.hand_id));
            }
            if !treatment {
                shared.models.write().observe_own_hand(&f.summary, &bot.name, 1.0);
            }
            shared.unstored_hands.lock().push((row.clone(), 0));
        }
    }
    if treatment {
        tracker.pending_calibration.clear();
    }
    row
}

/// Give the tracker the start of a hand a *live* `hand_result` names, when it cannot price it itself
/// (0323). A process that restarted into the hand, or resynced into it past preflop, never received
/// its `table_state` — only a preflop one fills `start_stacks` (tracker/mod.rs) — so `hero_net`
/// stayed `None`, the row was stored unpriced and the hand vanished from `review`, the leak finder
/// and the experiment pair's evidence, silently (18–40 hands a day). The hand saved before the swap
/// holds exactly that start, and `resume_hand` fills only what is missing.
pub(super) fn resume_open_hand(shared: &Shared, bot: &BotConfig, tracker: &mut TableTracker, hand_id: &str) {
    if tracker.knows_hand(hand_id) || tracker.hand_id.as_deref().is_some_and(|id| id != hand_id) {
        return; // priced already, or a result for a hand this tracker is not on
    }
    let mut resumable = shared.resumable.lock();
    if !resumable.get(&bot.name).is_some_and(|h| h.hand_id == hand_id) {
        return; // nothing saved for this hand: `replayed_result` reports the loss if it is replayed
    }
    let Some(open) = resumable.remove(&bot.name) else { return };
    drop(resumable);
    tracker.resume_hand(&open);
}

/// After a resync's replay: a hand saved before the restart that is over (the snapshot is another hand)
/// and that the replay did not settle is a lost row. Across four hot swaps on 2026-09-27 that was every
/// saved hand, and nothing said so (0315). Named once, with what the replay held, and dropped so a later
/// resync does not repeat it. A saved hand the snapshot still shows is kept: its result comes live.
pub(super) fn report_unsettled(shared: &Shared, bot: &BotConfig, msg: &Value) {
    let current = msg["snapshot"]["hand_id"].as_str();
    let mut resumable = shared.resumable.lock();
    let Some(saved) = resumable.get(&bot.name).map(|h| h.hand_id.clone()) else { return };
    if current == Some(saved.as_str()) {
        return;
    }
    resumable.remove(&bot.name);
    drop(resumable);
    let replayed = msg["replayed_events"].as_array().map_or(&[][..], Vec::as_slice);
    let results: Vec<&str> = replayed.iter().filter(|e| e["type"] == "hand_result").filter_map(|e| e["hand_id"].as_str()).collect();
    shared.log(
        &bot.name,
        "warn",
        format!(
            "hand {saved} was in progress at the restart and ended before the resync; the replay (seq {}..{}, {} events, results for {results:?}) does not hold its result, so its row is lost",
            msg["from_table_seq"],
            msg["to_table_seq"],
            replayed.len()
        ),
    );
}

/// Whether a replayed `hand_result` names `seat` as one that was dealt into the hand: it has a final
/// stack, or it won a pot. A result that does not is another table's or another seat's business.
fn played_seat(ev: &Value, seat: Option<usize>) -> bool {
    let Some(seat) = seat else { return false };
    let key = seat.to_string();
    !ev["final_stacks"][key.as_str()].is_null()
        || ev["winners"].as_array().is_some_and(|winners| winners.iter().any(|w| w["seat"].as_u64() == Some(seat as u64)))
}

/// A `hand_result` from a resync replay: settle and store it when it is a hand we played, know the
/// start of (still tracked, or saved by the process before a restart) and have not stored yet.
pub(super) fn replayed_result(shared: &Shared, slot: usize, bot: &BotConfig, tracker: &mut TableTracker, ev: &Value) {
    let Some(hand_id) = ev["hand_id"].as_str().map(String::from).or_else(|| tracker.hand_id.clone()) else { return };
    if !matches!(shared.store.hand(&bot.name, &hand_id), Ok(None)) {
        return; // stored already (or the store cannot say: never risk a second copy)
    }
    if !tracker.knows_hand(&hand_id) {
        // Earlier unknown results can precede the saved hand in the replay window. Only the
        // matching result consumes its start; filtering after removal loses a different hand.
        let saved = {
            let mut resumable = shared.resumable.lock();
            if resumable.get(&bot.name).is_some_and(|h| h.hand_id == hand_id) { resumable.remove(&bot.name) } else { None }
        };
        match saved {
            Some(open) => tracker.resume_hand(&open),
            None => {
                // A hand our seat played that we cannot settle is a lost row, and it is otherwise
                // silent: the replay is the only place it is mentioned again (0315).
                if played_seat(ev, tracker.hero_seat) {
                    shared.log(
                        &bot.name,
                        "warn",
                        format!("a replayed result for hand {hand_id} cannot be settled: its start was not saved"),
                    );
                }
                return;
            }
        }
    }
    // Settled pending calibration samples belong to the live path's final-stack frontier; a recovered
    // hand drops them rather than pricing them against a replayed stack.
    tracker.pending_calibration.clear();
    if let Some(f) = tracker.hand_result(ev) {
        let row = store_finished(shared, slot, bot, tracker, &f);
        shared.log(&bot.name, "info", format!("recovered hand {} from the resync replay (net {:?})", row.hand_id, row.net));
        // Both counters, as the live path does (0325): the row's net is real, and `status.sh` prints
        // the count beside the net, so counting one without the other drifts them apart.
        shared.update(slot, |b| {
            b.session_hands += 1;
            if let Some(net) = row.net {
                b.session_net += net;
            }
        });
    }
}

/// The resync replay window in the order the reducer contract requires: sorted by `table_seq` and
/// deduplicated. `apply_event` is not idempotent — a repeated `player_action` appends the same bet to
/// the hand history twice — and the window is not ours to trust, so a server that returns it
/// shuffled, or a resync we apply twice, would put a doubled bet into the hand we are about to decide
/// and into every stored summary of it (0268).
///
/// Events without a sequence keep their relative order after the sequenced ones: a sequence is the
/// only ordering the envelope promises, and an unsequenced event is not evidence of a later one.
pub(super) fn replay_window(msg: &Value) -> Vec<&Value> {
    let events = msg["replayed_events"].as_array().into_iter().flatten().collect::<Vec<_>>();
    let mut sequenced: Vec<(i64, usize, &Value)> =
        events.iter().enumerate().filter_map(|(i, ev)| Some((ev["table_seq"].as_i64()?, i, *ev))).collect();
    sequenced.sort_by_key(|(seq, i, _)| (*seq, *i));
    let mut out: Vec<&Value> = sequenced.iter().map(|(_, _, ev)| *ev).collect();
    let mut last_seq: Option<i64> = None;
    out.retain(|ev| match (ev["table_seq"].as_i64(), last_seq) {
        (Some(seq), Some(previous)) if seq == previous => false,
        (Some(seq), _) => {
            last_seq = Some(seq);
            true
        }
        (None, _) => true,
    });
    for ev in events.iter().filter(|ev| ev["table_seq"].as_i64().is_none()) {
        out.push(ev);
    }
    out
}

/// A resync answered as a spectator: the seat we were recovering is gone (an outage past the 120 s seat
/// window). The spec's recovery loop guard ends recovery only on a *player* resync, so the table is let go
/// and the bot rejoins the lobby instead of watching a table it no longer plays at.
pub(super) fn seat_gone(shared: &Shared, bot: &BotConfig, tracker: &mut TableTracker, msg: &Value) -> bool {
    if msg["role"].as_str() != Some("spectator") {
        return false;
    }
    let table = tracker.table_id.clone().unwrap_or_else(|| "?".into());
    shared.log(&bot.name, "warn", format!("resync of table {table} answered as a spectator: our seat there is gone; rejoining the lobby"));
    tracker.reset_table();
    true
}

/// One log line for a `resync_response` that was not a spectator: its role and what the snapshot
/// holds (#933), so a stall after a seat the server may have given up can be read from the log.
pub(super) fn resync_shape(msg: &Value) -> String {
    let shape = match &msg["snapshot"] {
        Value::Null => "null",
        s if s["hero"].is_object() => "object with a hero block",
        s if s.is_object() => "object without a hero block",
        _ => "other",
    };
    format!("resync answered as {}: snapshot {shape}", msg["role"].as_str().unwrap_or("?"))
}

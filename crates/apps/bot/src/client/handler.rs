//! Dispatch of every server message during a session: table state, hand lifecycle, results,
//! table selection, errors.

use super::*;

// The handler is one dispatch over the whole session's state; bundling it into a struct would only
// rename the same borrows.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle(
    shared: &Arc<Shared>,
    slot: usize,
    bot: &BotConfig,
    http: &reqwest::Client,
    tracker: &mut TableTracker,
    conn: &Conn,
    msg: &Value,
    rng: &mut SmallRng,
    seat: &mut Seat,
) -> Option<SessionEnd> {
    let t = msg["type"].as_str().unwrap_or("");
    // The watermark before this frame: `accept_seq` advances it, and a state-hash verdict needs the
    // sequence we had already applied to tell a replayed frame from a divergent snapshot (0301).
    let applied_seq = tracker.last_table_seq;
    if !tracker.accept_seq(msg) && t != "your_turn" {
        return None;
    }
    match t {
        "connected" => {
            shared.log(&bot.name, "info", format!("connected as {}", msg["name"].as_str().unwrap_or("?")));
            if let Some(table) = tracker.table_id.clone() {
                // Warm restart asks from our watermark; a cold one asks for the whole retained window.
                conn.send(json!({"type": "resync_request", "table_id": table, "last_table_seq": tracker.last_table_seq.max(0)}));
            } else if shared.bots[slot].read().desired == "run" {
                seat.pending_join = true;
            }
        }
        "lobby_joined" => shared.update(slot, |b| b.mode = "lobby".into()),
        "table_joined" => {
            tracker.table_joined(msg);
            seat.joined();
            shared.log(&bot.name, "info", format!("seated at table {} seat {}", msg["table_id"].as_str().unwrap_or("?"), msg["seat"]));
            sync_live(shared, slot, tracker);
        }
        "table_state" => {
            verify_state_hash(shared, slot, bot, tracker, applied_seq, conn, msg);
            tracker.table_state(msg);
            sync_live(shared, slot, tracker);
            shared.update(slot, |b| b.actor_seat = msg["actor_seat"].as_u64().map(|s| s as usize));
        }
        "resync_response" if recover::seat_gone(shared, bot, tracker, msg) => seat.pending_join = true,
        "resync_response" => {
            tracker.clear_event_clock();
            if let Some(seat) = msg["snapshot"]["hero"]["seat"].as_u64() {
                tracker.hero_seat = tracker.hero_seat.or(Some(seat as usize));
            }
            for ev in recover::replay_window(msg) {
                if ev["type"] == "hand_result" {
                    recover::replayed_result(shared, slot, bot, tracker, ev);
                } else {
                    apply_event(tracker, ev);
                }
            }
            recover::report_unsettled(shared, bot, msg);
            if msg["snapshot"].is_object() {
                tracker.table_state(&msg["snapshot"]);
                let hero = &msg["snapshot"]["hero"];
                if let Some(token) = hero["turn_token"].as_str() {
                    let mut turn = json!({
                        "type": "your_turn",
                        "valid_actions": hero["valid_actions"].clone(),
                        "pot": msg["snapshot"]["pot"].clone(),
                        "community_cards": msg["snapshot"]["board"].clone(),
                        "turn_token": token,
                        "seat": hero["seat"].clone(),
                    });
                    turn["hand_id"] = msg["snapshot"]["hand_id"].clone();
                    act(shared, slot, bot, tracker, conn, &turn, rng).await;
                }
            } else if tracker.table_id.is_some() && msg["snapshot"].is_null() {
                // Table gone: go back to the lobby.
                tracker.reset_table();
                seat.pending_join = true;
            }
            sync_live(shared, slot, tracker);
            shared.update(slot, |b| b.mode = "playing".into());
        }
        "hand_start" => {
            tracker.hand_start(msg);
            // The policy for this hand is fixed here, before any card is seen (0291).
            if let Some(hand) = tracker.hand_id.clone() {
                crate::experiment::latch(shared, slot, &hand);
            }
            shared.update(slot, |b| b.last_actions.clear());
            shared.update(slot, |b| b.mode = if seat.leaving() { "paused".into() } else { "playing".into() });
        }
        "hole_cards" | "player_action" | "community_cards" => {
            let street_before = tracker.street.map(|s| s.name()).unwrap_or("preflop");
            apply_event(tracker, msg);
            sync_live(shared, slot, tracker);
            if t == "player_action" {
                let seat = msg["seat"].as_u64().unwrap_or(0) as usize;
                let action = msg["action"].as_str().unwrap_or("").to_string();
                let to = tracker.history.last().map(|r| r.to).unwrap_or(0);
                let label = if to > 0 && action != "fold" && action != "check" { format!("{action} {to}") } else { action.clone() };
                shared.update(slot, |b| {
                    b.last_actions.insert(seat, label);
                    if Some(seat) == b.seat {
                        b.turn_started = None;
                    }
                });
                shared.emit("action", json!({"slot": slot, "bot": bot.name, "seat": seat, "name": msg["name"], "action": action, "amount": if to > 0 { json!(to) } else { Value::Null }, "street": street_before, "pot": msg["pot_after"].as_i64().or(msg["pot"].as_i64())}));
            } else if t == "community_cards" {
                shared.update(slot, |b| b.last_actions.clear());
                shared.emit("board", json!({"slot": slot, "bot": bot.name, "street": msg["street"], "cards": msg["cards"]}));
            }
        }
        "your_turn" => {
            shared.update(slot, |b| b.turn_started = Some(chrono::Utc::now().timestamp_millis() as f64 / 1000.0));
            act(shared, slot, bot, tracker, conn, msg, rng).await;
        }
        "action_ack" => {}
        "action_rejected" => {
            let code = msg["code"].as_str().unwrap_or("");
            let reason = msg["reason"].as_str().unwrap_or("");
            // Protocol codes mean our action frame is wrong: an error to fix, not a turn to recover.
            let protocol = matches!(code, "missing_action_id" | "action_id_conflict" | "legacy_action_protocol");
            shared.log(&bot.name, if protocol { "error" } else { "warn" }, format!("action rejected: {code} {reason}"));
            // A rejected action did not consume the turn: let the turn be answered again.
            shared.update(slot, |b| {
                b.rejections += 1;
                b.last_acted_turn_token = None;
                b.last_acted_hand_id = None;
            });
            // Private your_turn messages are never replayed and the 45 s deadline keeps running, so the
            // only way back to the turn is a resync: its snapshot restores the pending token.
            let recoverable =
                matches!(code, "stale_turn_token" | "stale_hand_action" | "invalid_action" | "not_your_turn" | "no_hand_in_progress");
            if recoverable && let Some(table) = tracker.table_id.clone() {
                let hand = tracker.hand_id.clone();
                let allowed = {
                    let mut b = shared.bots[slot].write();
                    if b.reject_resyncs.0 != hand {
                        b.reject_resyncs = (hand, 0);
                    }
                    b.reject_resyncs.1 += 1;
                    b.reject_resyncs.1 <= MAX_REJECT_RESYNCS
                };
                if allowed {
                    conn.send(json!({"type": "resync_request", "table_id": table, "last_table_seq": tracker.last_table_seq.max(0)}));
                }
            }
        }
        "hand_result" => {
            // The result can arrive live for a hand this process never saw start (0323): a hot swap
            // left it in progress and the resync landed past preflop, so the row would be stored with
            // no net. The hand saved at the swap holds its start.
            if let Some(hand_id) = msg["hand_id"].as_str().map(String::from).or_else(|| tracker.hand_id.clone()) {
                recover::resume_open_hand(shared, bot, tracker, &hand_id);
            }
            if let Some(f) = tracker.hand_result(msg) {
                let row = recover::store_finished(shared, slot, bot, tracker, &f);
                tracker.hands_at_table += 1;
                // Table moves (seat::between_hands): seek top bots, leave tough tables, bank a very deep
                // stack, top up a short one. Near the season end a re-queue can only lose the seat.
                if !seat.leaving() {
                    let quality = table_quality(shared, tracker, &bot.name);
                    let stack = tracker.hero_seat.and_then(|h| tracker.seats.get(&h)).map(|s| s.stack).unwrap_or(0);
                    let (since_switch, since_topup) = {
                        let b = shared.bots[slot].read();
                        (b.last_table_switch.map(|t| t.elapsed()), b.last_topup.map(|t| t.elapsed()))
                    };
                    let chosen = between_hands(&HandEnd {
                        moves_ok: shared.season_clock.read().table_moves_allowed(Instant::now()),
                        hands_at_table: tracker.hands_at_table,
                        seek_top_rank: shared.config.seek_top_rank,
                        quality: &quality,
                        since_switch,
                        since_topup,
                        stack,
                        bb: tracker.bb,
                        bank_stack_bb: shared.config.bank_stack_bb,
                        max_buy_in: shared.config.max_buy_in,
                    });
                    if let Some(chosen) = chosen {
                        move_tables(shared, slot, bot, http, tracker, conn, seat, chosen, stack).await;
                    }
                }
                if let Some(final_stack) = f.hero_final_stack
                    && tracker.bb > 0
                {
                    let bb = tracker.bb as f64;
                    for pending in tracker.pending_calibration.drain(..) {
                        let realized =
                            sv10_venue::tracker::realized_incremental_chips(final_stack, pending.hero_stack_before_action) as f64;
                        if let Err(e) = shared.store.insert_calibration(
                            &bot.name,
                            &f.hand_id,
                            &pending.category,
                            pending.predicted_incremental_chips / bb,
                            realized / bb,
                            pending.scale_chips as f64 / bb,
                        ) {
                            tracing::warn!(bot = %bot.name, "calibration sample not stored: {e}");
                        }
                    }
                } else if !tracker.pending_calibration.is_empty() {
                    // Without a final stack or a positive blind the units are unknown; samples are
                    // dropped rather than inventing a stack result or one-chip blind.
                    tracing::info!(bot = %bot.name, "hand {}: {} calibration samples dropped (final stack or positive bb unavailable)", f.hand_id, tracker.pending_calibration.len());
                    tracker.pending_calibration.clear();
                }
                let net = f.hero_net.unwrap_or(0);
                shared.emit("result", json!({"slot": slot, "bot": bot.name, "hand_id": f.hand_id, "net": f.hero_net, "pot": f.pot, "winners": f.winners, "board": row.board, "hole": row.hole}));
                shared.update(slot, |b| {
                    b.session_hands += 1;
                    b.session_net += net;
                });
                let _ = shared.events.send(json!({"type": "hand", "slot": slot, "net": net}).to_string());
                if shared.bots[slot].read().session_hands.is_multiple_of(25) {
                    let shared2 = shared.clone();
                    let bot2 = bot.clone();
                    let http2 = http.clone();
                    tokio::spawn(async move {
                        if let Some(m) = rest_get(&http2, &shared2, &bot2, "/season/me").await {
                            shared2.update(slot, |b| b.season = Some(m));
                        }
                    });
                }
            }
            sync_live(shared, slot, tracker);
        }
        "busted" => {
            shared.log(&bot.name, "info", "busted");
        }
        "auto_rebuy_scheduled" => {
            shared.log(&bot.name, "info", format!("auto-rebuy scheduled in {}s", msg["cooldown_seconds"]));
            let due = Instant::now() + Duration::from_secs(msg["cooldown_seconds"].as_u64().unwrap_or(0));
            shared.update(slot, |b| b.auto_rebuy_at = Some(due));
        }
        "rebuy_confirmed" => {
            shared.log(&bot.name, "info", format!("rebuy confirmed, balance {}", msg["chip_balance"]));
            shared.update(slot, |b| b.auto_rebuy_at = None);
            if tracker.seats.get(&tracker.hero_seat.unwrap_or(99)).map(|s| s.stack == 0).unwrap_or(true) {
                tracker.reset_table();
                seat.pending_join = true;
            }
        }
        "table_closed" => {
            shared.log(&bot.name, "info", format!("table closed: {}", msg["reason"].as_str().unwrap_or("")));
            // The spec (revision 2026-09-02): if a leave was in flight and `table_closed` arrives
            // before our own `player_left`, send one final `leave_table` while the socket is still
            // open, and treat a following `not_at_table` as a clean exit. Without it the seat is
            // left half-released: the next `join_lobby` can come back `already_seated` and point at a
            // table we are not at (0264).
            let leaving = seat.leaving();
            tracker.reset_table();
            sync_live(shared, slot, tracker);
            if leaving {
                conn.send(json!({"type": "leave_table"}));
            }
            // A table closing completes a leave to rejoin (rejoin now); a pause stays paused.
            if seat.left() {
                shared.update(slot, |b| b.mode = "paused".into());
            }
        }
        "season_ended" => {
            shared.log(&bot.name, "info", format!("season ended: {msg}"));
            tracker.reset_table();
            sync_live(shared, slot, tracker);
            seat.season_ended();
        }
        "player_left" => {
            if msg["name"].as_str() == Some(bot.name.as_str()) {
                shared.log(&bot.name, "info", format!("left table ({})", msg["reason"].as_str().unwrap_or("")));
                tracker.reset_table();
                sync_live(shared, slot, tracker);
                if seat.left() {
                    shared.update(slot, |b| b.mode = "paused".into());
                }
            }
        }
        "player_joined" => {}
        "chips_skimmed" => shared.log(&bot.name, "info", format!("chips skimmed: {msg}")),
        "error" => {
            let code = msg["code"].as_str().unwrap_or("");
            match code {
                "auth_failed" => return Some(SessionEnd::Fatal("auth_failed".into())),
                "already_seated" => {
                    let mut tid = msg["table_id"].as_str().or(msg["details"]["table_id"].as_str()).map(String::from);
                    if tid.is_none()
                        && let Some(a) = rest_get(http, shared, bot, "/me/active-game").await
                    {
                        tid = a["table_id"].as_str().map(String::from);
                    }
                    if let Some(tid) = tid {
                        tracker.table_id = Some(tid.clone());
                        conn.send(json!({"type": "resync_request", "table_id": tid, "last_table_seq": 0}));
                    }
                }
                "already_in_lobby" | "leave_pending" => {}
                "not_at_table" => {
                    tracker.reset_table();
                    seat.left();
                }
                "insufficient_funds" | "not_registered_for_season" => {
                    seat.pending_join = true;
                }
                // The spec documents `rate_limited` as a dropped message (20/s), but the server also
                // sends it to refuse a connection ("Too many connection attempts for this play
                // pool"); that one ends the session so the reconnect backs off.
                "rate_limited" if msg["message"].as_str().is_some_and(|m| m.to_ascii_lowercase().contains("connection attempt")) => {
                    shared.log(&bot.name, "warn", format!("server error {code}: {}; backing off", msg["message"].as_str().unwrap_or("")));
                    return Some(SessionEnd::Throttled);
                }
                "rate_limited" | "flood_warning" => {}
                "flood_kick" => {
                    tracker.reset_table();
                    seat.pending_join = true;
                }
                _ => {}
            }
            shared.log(&bot.name, "warn", format!("server error {code}: {}", msg["message"].as_str().unwrap_or("")));
        }
        _ => {}
    }
    None
}

/// Count `state_hash` results and resync on a mismatch. A mismatch triggers a resync only once our
/// serializer has matched a snapshot on this bot (a serializer bug must never loop resyncs), and at
/// most once a minute.
///
/// `applied_seq` is the watermark *before* this frame: `accept_seq` has already advanced it, so
/// comparing the frame's sequence against the tracker's own would file every mismatch as `STALE`
/// (a replay) and a genuinely divergent serializer would never read as `DIVERGED` (0301).
fn verify_state_hash(
    shared: &Arc<Shared>,
    slot: usize,
    bot: &BotConfig,
    tracker: &TableTracker,
    applied_seq: i64,
    conn: &Conn,
    msg: &Value,
) {
    let Some(ok) = sv10_venue::statehash::verify(msg) else { return };
    if ok {
        crate::live::count_state_hash(&mut shared.bots[slot].write(), true, None);
        return;
    }
    // Diagnose before anything moves on: the verdict (stale replay vs real divergence) plus a
    // field census in the log, the full snapshot in the incident table (0265). The dashboard keeps
    // the same verdict, table and census, so a later reader does not have to open the store (0301).
    let report = sv10_venue::statehash::mismatch_report(msg, applied_seq);
    let stale = sv10_venue::statehash::is_stale(msg, applied_seq);
    let seq = msg.get("table_seq").and_then(|v| v.as_i64()).unwrap_or(-1);
    let table_name = tracker.table_id.clone().unwrap_or_else(|| "?".into());
    let verdict = if stale { "STALE" } else { "DIVERGED" };
    let mismatch = crate::live::StateHashMismatch {
        at: chrono::Utc::now().timestamp_millis() as f64 / 1000.0,
        table: table_name.clone(),
        verdict: verdict.to_string(),
        summary: report.clone(),
    };
    let mut b = shared.bots[slot].write();
    crate::live::count_state_hash(&mut b, false, Some(mismatch));
    let due = b.last_hash_resync.is_none_or(|t| t.elapsed() >= Duration::from_secs(60));
    let resync = b.state_hash_ok > 0 && due && tracker.table_id.is_some();
    drop(b);
    shared.store.record_hash_incident(&bot.name, &table_name, seq, verdict, &report, &msg.to_string());
    shared.log(&bot.name, "warn", format!("state_hash mismatch on table {table_name}; {report}"));
    if resync && let Some(table) = tracker.table_id.clone() {
        shared.bots[slot].write().last_hash_resync = Some(Instant::now());
        conn.send(json!({"type": "resync_request", "table_id": table, "last_table_seq": tracker.last_table_seq.max(0)}));
    }
}

/// Carry out a table move chosen between hands: send `leave_table` to rejoin, with its log line
/// and dashboard event. A top-up first confirms the off-table balance with a fresh read (the
/// cached one can be minutes old) and is checked at most every ten minutes either way.
#[allow(clippy::too_many_arguments)]
async fn move_tables(
    shared: &Arc<Shared>,
    slot: usize,
    bot: &BotConfig,
    http: &reqwest::Client,
    tracker: &TableTracker,
    conn: &Conn,
    seat: &mut Seat,
    chosen: Move,
    stack: i64,
) {
    let hands = tracker.hands_at_table;
    // (log line, dashboard reason, counts as a table switch for the seek/tough cooldowns)
    let (line, event, switch) = match chosen {
        Move::Seek(reason) => (format!("table seeking: re-queueing after {hands} hands — {reason}"), Some(reason), true),
        Move::Tough(summary) => (format!("table selection: leaving a tough table after {hands} hands ({summary})"), Some(summary), true),
        Move::Bank => (
            format!("banking winnings: table stack {stack} ({} bb); leaving to rejoin with a fresh buy-in", stack / tracker.bb.max(1)),
            Some("banking winnings".to_string()),
            false,
        ),
        Move::TopUp => {
            let fresh = rest_get(http, shared, bot, "/season/me").await;
            let stored = shared.bots[slot].read().season.clone();
            if let Some(m) = &fresh {
                let m = m.clone();
                shared.update(slot, |b| b.season = Some(m));
            }
            // A failed read is unknown, not a measured 0 (0324): fall back to the last balance on
            // record, and only stamp the cooldown once a funding decision was actually made — one
            // flaky call used to park a short stack for the full ten minutes.
            let Some((balance, _)) = balance_from(fresh.as_ref(), stored.as_ref()) else {
                shared.log(&bot.name, "warn", "top-up skipped: /season/me unreadable and no balance on record");
                return;
            };
            // Checked either way: a short stack without the balance to deepen it is not re-checked every hand.
            shared.update(slot, |b| b.last_topup = Some(Instant::now()));
            if !top_up_funded(stack, balance, shared.config.max_buy_in) {
                return;
            }
            (format!("topping up: stack {stack}, off-table balance {balance}; leaving to rejoin deeper"), None, false)
        }
    };
    if !seat.leave(Leave::Rejoin) {
        return;
    }
    conn.send(json!({"type": "leave_table"}));
    if switch {
        shared.update(slot, |b| b.last_table_switch = Some(Instant::now()));
    }
    shared.log(&bot.name, "info", line);
    if let Some(reason) = event {
        shared.emit("table_select", json!({"slot": slot, "bot": bot.name, "reason": reason}));
    }
}

pub(super) fn apply_event(tracker: &mut TableTracker, ev: &Value) {
    match ev["type"].as_str().unwrap_or("") {
        "hand_start" => tracker.hand_start(ev),
        "hole_cards" => tracker.hole_cards(ev),
        "player_action" => tracker.player_action(ev),
        "community_cards" => tracker.community_cards(ev),
        "table_state" => tracker.table_state(ev),
        _ => {}
    }
}

pub(super) fn sync_live(shared: &Shared, slot: usize, t: &TableTracker) {
    let seen: Vec<(String, String)> = t.seats.values().filter_map(|s| Some((s.name.clone(), s.avatar_url.clone()?))).collect();
    if !seen.is_empty() {
        let mut avatars = shared.avatars.write();
        for (name, url) in seen {
            avatars.insert(name, url);
        }
    }
    shared.update(slot, |b| {
        b.table_id = t.table_id.clone();
        b.seat = t.hero_seat;
        b.hand_id = t.hand_id.clone();
        b.street = t.street.map(|s| s.name().to_string());
        b.dealer = t.dealer;
        b.pot = t.pot;
        b.big_blind = t.bb;
        b.hole = t.hole.map(|h| vec![h[0].to_string(), h[1].to_string()]).unwrap_or_default();
        b.board = t.board.iter().map(|c| c.to_string()).collect();
        b.seats = t.seats.values().cloned().collect();
        b.stack = t.hero_seat.and_then(|h| t.seats.get(&h)).map(|s| s.stack).unwrap_or(0);
        b.open_hand = t.open_hand();
    });
}

#[cfg(test)]
mod tests;

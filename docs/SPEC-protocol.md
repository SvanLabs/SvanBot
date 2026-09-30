# SPEC — openpoker.ai protocol as implemented

> **Read this when** you touch the WebSocket client or the tracker.
> **Code:** `crates/apps/bot/src/client`, `crates/libs/venue`.
> **Related:** [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) for where the protocol sits in the decision path. · [All docs](README.md)

The WebSocket V2 protocol as `sv10-bot` (`crates/apps/bot/src/client/`) and `sv10-venue`
(`crates/libs/venue/src/tracker/`, `statehash.rs`) implement it, checked against
`https://docs.openpoker.ai/llms-full.txt` **revision 2026-09-02**. The spec controls; re-fetch it
before protocol work and update this file when either side changes.

## Invariants

1. Send only actions present in the current `valid_actions` (`client::decide::legalize` maps every
   policy action onto the offer; `legalize_maps_to_offered_actions` tests it).
2. Every `action` carries the current `hand_id`, `turn_token` and a fresh `client_action_id` (v4 id).
   A `(hand_id, turn_token)` is answered at most once: a redelivered `your_turn` or resync snapshot
   with the pair we last sent is skipped, while the same token may be reused by a different hand.
   Within one hand, a sequenced authority older than the newest sent turn is stale. A rejected action
   reopens its turn. An action the writer could not queue is not recorded as sent, persisted as a
   decision/audit, or retained for calibration; reconnect then resyncs the turn.
3. Raise `amount` is the raise-to total, clamped into `[min, max]`; a raise at `max` is sent as
   `all_in` when offered. Calls and checks send no amount.
4. Branch on `code` (in `error` and `action_rejected`), never on `message`/`reason`.
5. Blinds exist only as `table_state` bets; `player_action.street` is the street *after* the action.

## Decision replay

Replay JSON version 2 captures the authoritative situation (including `current_bet_to`), RNG seed,
all strategy/range/calibration parameters, table-player and population statistics, response-network
digest, chosen action, and every candidate EV. Exact replay requires byte-equivalent serialized
parameters and the network bytes matching that digest. A changed parameter set, missing/different
network, or `review replay --current` is labeled a what-if even when it happens to choose the same
action. Version-zero records remain readable; missing `current_bet_to` uses the legacy reconstruction.

## Connection

| Spec | Implementation |
|---|---|
| `wss://openpoker.ai/ws`, `Authorization: Bearer <api_key>` (query token legacy, public only) | Bearer header only (`client::mod`), user agent `svanbot10/<version>` |
| `connected` with `agent_id`, `name` | Logged; resync if a table is known, else join the lobby |
| `auth_failed` closes with 4001 | Fatal for that bot (no retry loop against a revoked key) |
| Session takeover by a new socket | Relied on by hot-swap releases: the new process reconnects and resyncs |
| 120 s reconnect grace, seat held | Reconnect with exponential backoff 1 s → 60 s plus jitter; watchdog reconnects after 3 min without table traffic |
| Free: 1 public bot; Pro: up to 5 portfolio bots; same-owner bots never share a table | 5 configured bots (Pro portfolio) |

## Cold and warm restart

| Spec | Implementation |
|---|---|
| Warm: `resync_request {table_id, last_table_seq}` with the highest applied seq | On `connected` with a retained table: `last_table_seq = tracker.last_table_seq` |
| Cold: `GET /api/me/active-game` first; if playing, resync with `last_table_seq: 0`; else `join_lobby` once | Before each session the client calls `/me/active-game`; a returned table is resynced from 0 |
| `already_seated` race: use its `table_id`/`seat`, stop joining, resync | Uses `table_id` (or `details.table_id`, or `/me/active-game`) and resyncs from 0 |
| `resync_response`: apply `replayed_events` in order, then install `snapshot`; an acting player's snapshot carries `hero.turn_token` | `apply_event` for replayed events, then the snapshot; a token in the snapshot re-enables acting before the original deadline |
| Recovery loop guard: only a *player* `resync_response` ends recovery; `role: "spectator"` means the seat is gone (past the 120 s window) | A spectator resync lets the table go and rejoins the lobby (`recover::seat_gone`); before, the bot installed the snapshot and waited for turns that never came |

## Client → server

| Message | Fields sent | When |
|---|---|---|
| `join_lobby` | `buy_in` (1,000–5,000) | Start, after `table_closed`, `season_ended`, bust, top-up or table-seeking leave. Buy-in = `min(balance, SVANBOT_BUY_IN=5000)` after a REST rebuy when the balance is under 1,000 |
| `action` | `hand_id`, `action`, `amount` (raise only), `client_action_id`, `turn_token` | On `your_turn` (or an acting resync snapshot) |
| `leave_table` | — | Table seeking (no established top-30 bot seated), short-stack top-up (stack < 35% of max buy-in, fresh balance can double it, at most every 10 min), stop commands |
| `resync_request` | `table_id`, `last_table_seq` | Reconnect, `already_seated`, `state_hash` mismatch (after at least one verified snapshot, at most once a minute) |
| `set_auto_rebuy` | `enabled: true` | Sent after every `join_lobby` (the spec's recommended order, so the season entry exists; idempotent, and re-arms the preference in a new season) |
| `rebuy` | not sent | Fallback rebuys go through `POST /api/season/rebuy` before joining (1,500 chips; cooldown 5 min free / 2 min Pro) |

## Server → client

| Message | Spec content | Handling |
|---|---|---|
| `lobby_joined` | `position`, `estimated_wait` | Mode `lobby` |
| `table_joined` | `table_id`, `seat`, `players` | Tracker reset on a table change; seats and stacks |
| `hand_start` | `hand_id`, `seat`, `dealer_seat`, `blinds` | New hand in the tracker |
| `hole_cards` | `cards` | Tracker |
| `your_turn` | `valid_actions`, `pot`, `community_cards`, `players`, `min_raise`, `max_raise`, `turn_token` | Decision on a blocking thread with an 8 s timeout (the check-or-fold fallback is legalized) |
| `action_ack` | `client_action_id`, `status` | Ignored |
| `action_rejected` | `code`, `reason`, `details.code` | Logged and counted per bot (dashboard `rej`) |
| `player_action` | `seat`, `name`, `action`, `amount` (null for check/fold), `street`, `stack`, `pot`; optional `to_call_before`, `pot_before/after`, `stack_before/after`, `contribution_delta`, `action_id` | History record (null amounts read as 0) |
| `community_cards` | `cards`, `street` | Board and street change |
| `hand_result` | `winners [{seat, name, stack, amount, hand_description}]`, `pot`, `final_stacks`, `shown_cards` (showdown only), `actions`, `payouts` | Finished hand stored before models update; calibration outcomes; top-up check |
| `table_state` | `street`, `dealer_seat`, blinds, `pot`, `actor_seat`, `to_call`, `min/max_raise_to`, `board`, `seats[]` (`status`, `in_hand`, `folded`), `hero` (`hole_cards`, `valid_actions` while acting) | `state_hash` verified, then applied as the authoritative snapshot (`folded` is authoritative) |
| `busted` | `options` | Logged; the table exit arrives as `player_left`, and the next join buys in from the balance (refilled by the server's auto-rebuy when short) |
| `auto_rebuy_scheduled` | `cooldown_seconds` | Records the due time; joins wait for it (plus 30 s grace) instead of racing a REST rebuy into a 429 |
| `rebuy_confirmed` | `chip_balance` | Clears the pending auto-rebuy; rejoins if our stack is gone |
| `auto_rebuy_set` | `enabled` | Logged |
| `chips_skimmed` | `excess`, `new_stack`, `new_balance` | Logged (cap disabled in production) |
| `player_joined` / `player_left` | `seat`, `name`, `stack` / `reason` | `player_joined` ignored (the next `table_state` carries the seat); our own `player_left` resets the tracker and ends a leave or top-up |
| `table_closed` | `reason` | Rejoin the lobby |
| `season_ended` | `season_number`, `next_season_number` | Rejoin the lobby (auto-registers) and run the season-rollover checks |
| `error` | `code`, `message` | `auth_failed` fatal; `already_seated` resync; `already_in_lobby` ignored; `not_at_table` clean exit after an intended leave; `insufficient_funds`, `not_registered_for_season`, `rate_limited`, `flood_warning`, `flood_kick` logged; `rate_limited` with "Too many connection attempts for this play pool" (undocumented; seen 2026-09-22) ends the session and reconnects with a 5 s → 60 s doubling backoff |

## Envelope and state hash

An empty postflop snapshot or turn board preserves cards already observed in that hand. It is
not evidence that those cards disappeared. Starting a new hand clears the board; a preflop
snapshot still accepts an empty board. This guards recovery frames without inventing missing cards.

- `table_seq`: forward jumps accepted; duplicates and regressions ignored (`TableTracker::accept_seq`),
  except a `table_state` repeating the watermark and every `resync_response`. Accepting a resync
  never lowers the watermark, even if its envelope carries an older sequence. We resync on reconnect,
  hash failure or impossible state, never on a gap alone.
- `state_hash` (`statehash::verify`): drop top-level `ts`, `table_seq`, `hand_seq`, `state_hash`;
  compact JSON with sorted keys and `ensure_ascii` escaping; SHA-256; `sha256:` prefix. Matched
  5,248/5,248 archived frames; counts shown per bot on the dashboard. Floats are spelled as
  Python's `repr` spells them — shortest round-trip digits, an exact tie broken to the even digit —
  which is not what Rust's `{:e}` prints in a tie, so `write_float` re-renders at the same length
  and keeps the render only where it round-trips: at a power of two the neighbour below is half an
  ulp away, and `repr` always round-trips.
- `ts` (observed on every server frame; the timing tells rely on it): RFC 3339 with
  microseconds, e.g. `2026-09-14T09:46:52.751076+00:00`, parsed by the tracker's own
  `rfc3339_ms`. A frame without a readable `ts` records no think time. Excluded from the state hash
  as above.

## REST (`https://api.openpoker.ai/api`; `Authorization: Bearer` on `/me/*` and other account endpoints, none on the public `/season/current`, `/season/list`, `/season/leaderboard` and public profile reads)

| Endpoint | Use |
|---|---|
| `GET /me/active-game` | Cold start and `already_seated` recovery |
| `GET /season/me` | Balance before buy-in and top-up (spec: season chips, rank, hands) |
| `POST /season/rebuy` | 1,500 chips when the balance is below the 1,000 minimum; honours `Retry-After` |
| `GET /season/current` | Season clock: `winding_down`, `time_remaining_seconds` (no table moves while winding down), plus `season_number`, `season_id`, `start_date` (the boundary for season-scoped dashboard panels) |
| `GET /season/leaderboard`, `/season/list`, `/season/{id}/leaderboard` | Ranks, reputation book, table seeking (30/min and 60/min per-IP limits respected) |
| `GET /me/hand-history` | Past-hand download (30/min, spaced 2.6 s; stops at the 20,000-hand export cap, `SVANBOT_EXPORT_CAP`, which Pro keys ignore — `pro_tier` from `/season/me` lifts it automatically) |
| `GET /me/hand-history/export?format=json&season_id=` | Per-season backfill, all ended seasons, 2 pages/bot/pass (undocumented analytics endpoint, verified live 2026-09-15; needs Pro — only Pro keys reach past seasons, Free keys get empty pages; spaced 6 s, the endpoint 429s at the plain 2.6 s pace) |

## Timeouts (spec)

Action 45 s in public play (auto-fold, or auto-check when fold is illegal; a disconnect does not pause
it); reconnect 120 s; 3 consecutive missed hands removes the player. Our decisions are fast,
with an 8 s hard cap (over stored decisions: median 1.4 ms, mean 2.9 ms).

Observed pacing, not stated in the spec (from `ts` on live frames):
- an action that follows another action is broadcast about 3.0 s after it;
- the first actor after new community cards shows 10–20 ms.

So a fast opponent's own think time hides inside the pacing, and only long thinks stand out. The
tracker records opponents' think time (capped at 60 s), and the range model uses it only through
the calibrate gate.

## Deviations and open items

- Auto-rebuy is on. It only credits the fixed 1,500 chips to the off-table balance when a
  bust leaves it under the 1,000 minimum; buy-in size (`join_lobby`) and top-ups stay ours. Observed
  live: one bust was refilled with no REST call, while a later one got `auto_rebuy_scheduled` as our
  REST rebuy raced it into `429 Rebuy on cooldown` — the two paths share one cooldown, hence the
  deferral. REST remains the fallback if the scheduled rebuy has not landed 30 s after its due time.
- `action_rejected` with a recoverable code (`stale_turn_token`, `stale_hand_action`, `invalid_action`, `not_your_turn`, `no_hand_in_progress`) triggers a resync, at most 3 per hand. Private `your_turn` messages are never replayed and the 45 s deadline keeps running, so the snapshot's restored token is the only way to act again. The protocol codes (`missing_action_id`, `action_id_conflict`, `legacy_action_protocol`) are logged as errors and never resynced. Spec re-read 2026-09-23: the 45 s action deadline starts when the server sends `your_turn` and is never extended.
- Idle `waiting_reason` values in `table_state` are not surfaced on the dashboard (non-fatal by spec).

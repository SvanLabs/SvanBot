# Dashboard API contract

> **Read this when** you touch the dashboard API or anything under `web/`.
> **Code:** `crates/apps/bot/src/api`, `web/src` (the contract is `web/src/types.ts`).
> **Related:** [`docs/GUIDE.md`](GUIDE.md) section 10 for the panels as an operator sees them. · [All docs](README.md)

The dashboard is an operator surface. It reports observed state and does not treat rank, score movement, or short-run results as proof that the poker strategy improved.

## Season leaderboard

`GET /api/leaderboard` returns:

- `entries`: upstream leaderboard rows in upstream order;
- `season`: the last successfully fetched current-season object;
- `updated`: Unix timestamp of the last successful paired season/leaderboard fetch, or `null` before one succeeds;
- `stale`: `false` only for a successful refresh and `true` after a failed refresh;
- `error`: `null` on success or a bounded, non-secret operator message on failure.

Each entry includes nullable upstream `rank`, `score`, `hands`, `win_rate`, and `pro` values plus `name` and `ours`. Derived nullable fields are:

- `rank_delta`: previous successful rank minus current rank for the same bot identity;
- `gap_to_first`: rank-one score minus this score;
- `gap_to_next`: the closest available better rank's score minus this score;
- `gap_to_four`: zero at ranks 1–4, otherwise rank-four score minus this score;
- `score_delta`: current score minus the same bot's previous successful score;
- `score_velocity_per_hour`: `score_delta` divided by elapsed hours, emitted only when successful snapshots are at least five minutes apart.

Missing, malformed, or not-yet-measurable values remain JSON `null`; they are never coerced to zero. Rank gaps use actual rank values rather than array positions, so a skipped rank does not shift the target.

The server caches refresh attempts for 60 seconds. A failed refresh does not advance movement history or `updated`; it preserves the last successful entries and season while setting `stale:true`. An initial failure returns an empty entry list with `updated:null`.

## Errors and stale data

- A request the server cannot answer returns a non-2xx status with `{"detail": "<reason>"}`: 404 for an
  unknown bot slot, hand or player, 500 when the store cannot be read or the handler failed, 401 without
  the operator session, 403 for a change sent to a non-loopback host when no operator token is set.
  An empty list with 200 always means "nothing recorded", never "could not read".
- The web client goes through `web/src/api.tsx`: `request` throws the server's `detail`; `usePoll`
  keeps the last good answer, reports the latest error and its age, and panels render `StaleNote`
  ("Update failed (...); showing data from HH:MM") while polls fail.
- Shared formatting lives in `web/src/format.ts` (`fmt`, `sgn`, `pct`, `SUITS`). Every browser-storage
  read goes through `readLocal` in `web/src/storage.ts`, which answers `null` where the accessor
  raises instead of returning — a browser with site data blocked renders the dashboard without its
  remembered preferences rather than not at all.
- Views: a tab bar (Live, Opponents, Learning, Results, System, All; `widgets.tsx` `VIEWS`)
  shows a subset of the widget board in the user's own arrangement; the page opens on Live
  (2,507 px tall on a 1440-wide screen against 6,743 for All), the choice is remembered in the
  browser, arrow keys move between tabs, and "Arrange widgets" always shows the whole board. Views that
  leave a column empty get their own grid template; at phone width everything stacks.
- Panel collapse is a reading preference and stays in the browser: the whole panel bar toggles it
  (the info button and a link in the heading do not), the chevron is the explicit affordance, and the
  workspace's collapse-all/expand-all acts on the current view's panels. The key is the widget id
  (`svan-panel-collapsed:<id>`), with the pre-#729 title key read as a fallback; the store keeps it
  in one place so the workspace control can reach every panel on screen.
- Web modules: `main.tsx` holds the app shell and routing; `ui.tsx` the shared primitives
  (formatters, seat read, table themes, card, panel, empty state); `table.tsx` the live table, TV mode
  (the dashboard's and the public listener's) and decision strip; `training.tsx` the season, performance and learner panels; `panels.tsx` replay,
  the starting-hand guide and the opponent profile. Each opponent seat's `read` carries `size_tell`
  when a per-opponent river sizing tell is installed, shown in the seat tooltip.

## Board layout (`GET`/`POST /api/layout`)

`GET /api/layout` returns the saved arrangement — `{left, center, right, hidden}`, widget ids in
order (`web/src/types.ts` `DashboardLayout`) — or `null` when none is stored. `POST /api/layout`
stores the document the Arrange mode produced and answers with what was stored; a `null` body stores
none, and a body that is not an arrangement is refused with 400 and changes nothing. The document is
one JSON value in the kv store (`dashboard.layout.v1`), so an arrangement survives a browser change
and a second operator device.

The client reconciles a stored document against the widgets the running build has: unknown ids are
dropped and new widgets land in their default column. It writes through the endpoint on every Arrange
mutation, and keeps the `svan-layout:v1` browser copy as what renders before the answer arrives and
what stays when the endpoint cannot be reached. The public TV renders its own fixed table view and
never reads the layout, and never asks the endpoint.

## Operator notes (`GET`/`POST /api/notes`)

`GET /api/notes` returns the operator's scratchpad — `{text}`, one plain-text note
(`web/src/types.ts` `DashboardNotes`) — or `null` when none is stored. `POST /api/notes` stores the
note and answers with what was stored; a body that is not `{text: string}`, or text past the 20,000
character cap, is refused with 400 and changes nothing, and empty text clears the stored note (the
store's convention for "nothing stored"). The document is one JSON value in the kv store
(`dashboard.notes.v1`), so a note survives a browser change. Panel: `web/src/notes.tsx`, on the board
as the `notes` widget.

The client saves a second after the last keystroke, and at once on blur and on `pagehide` (a
`keepalive` request, which a plain fetch would lose with the page). A save that fails stays on screen
as failed and retries with doubling backoff rather than looking saved (LESSONS 24). Browser-local
keys `svan-notes:v1` and `svan-notes:unsaved` hold the note and whether it reached the server: the
local copy renders before the answer arrives, and the flag keeps a later load from adopting an older
stored note over keystrokes that never got through. With nothing unsent the stored document is the
truth, so a browser still holding an old copy drops it when the server answers `null` — a cleared
note stays cleared — and text the flag marks unsent is pushed on load even when the read failed. The
public TV never mounts the endpoint.

## Per-opponent reads (`GET /api/intel`)

`{fits: [{id: "fold"|"response"|"sizing", title, reads, stored, active, evidence: {gain_mnats,
half_width_mnats, n} | null, installed}], corrected_opponents, opponents: [{name, hands, fold_offset,
response_ratio, size_tell}]}`: the three per-opponent fits with the held-out evidence
of the stored fit and how many opponents each corrects in live play right now, and the 25 most-observed
opponents with any correction. A store read error answers 500. Panel: `web/src/intel.tsx`.

## Releases and one-click updates

- `GET /api/releases`: `installed` (commit, time, subject from `artifacts/releases.log`), `head`,
  `behind` (commits an update would install: `installed..<remote>/<branch>` once the branch has been
  fetched, else `installed..HEAD`), `dirty` (uncommitted build inputs, the definition `release.sh`
  refuses on), `update_available`, `build` (this process's commit and version), `changelog`
  (`{commit, subject, group}` by commit prefix) and `remote`: the last update check
  `{source, commit, behind, checked_at, error}` or `null` before the first one.
- `POST /api/releases/check`: fetch the update branch now (`scripts/update.sh --check`) and return the
  check. The fleet also checks every 30 minutes; a failed check is logged once per change of outcome.
- `POST /api/releases/update`: 409 while a run holds the lock or with uncommitted build inputs;
  otherwise starts `scripts/update.sh` detached in its own process group (it outlives the fleet head,
  which exits during the hot swap): fetch, fast-forward (never a merge or rewrite; refused when the
  checkout has commits the branch lacks), `scripts/release.sh` (tests before the release build), install.
  A failed release moves the checkout back to the installed commit; the fleet keeps playing throughout.
- `GET /api/releases/progress`: `{state: idle|running|failed|installed, running, stages: [{name,
  state: pending|running|done|failed, seconds, expected}], percent, elapsed, eta, from, commit, message,
  swap: {target, fleet, fleet_done, learner, analyst, workers: [{bot, commit}]}, bots_playing,
  bots_total, log}`. Stages: fetch, snapshot, lint, test, build, dashboard, install (a release run by
  hand has no fetch); `elapsed` is the time since the run started while it runs, and the run's own
  duration once it has ended (`installed`/`failed`, from the progress file's `updated`);
  `percent` and `eta` weigh them by the last successful run's times
  (`artifacts/release-timings.json`), else defaults for the i7-4770K. The swap is confirmed by each
  process reporting its build: this process (`fleet`), the learner and analyst status, worker heartbeats.
- `GET /api/releases/log`: the 64 KB tail of `artifacts/release.log` (kept for tools).
- `GET /api/host`: `{checks: [{key, label, value, status: ok|warn|info, advice}], checked_at}` —
  CPU and AVX2, microcode (judged against 0x28 on the i7-4770K only), frequency scaling, transparent
  huge pages, memory and swap, free space on the databases' disk and the archive disk (or a warning
  when the archive shares the SSD), `fstrim.timer`, kernel. Read fresh per request from `/proc`,
  `/sys` and systemd's timer links; unreadable facts are left out; `advice` is the operator's
  command. The System view's Host check panel polls it every 60 s. Nothing is ever changed.
- Panel: `web/src/updates.tsx` (`UpdatesPanel`, `UpdateProgress`); browser tests
  `web/tests/updates.spec.ts`.

## Public TV listener (off by default)

`SVANBOT_TV_PORT` (default `0`, off; `SVANBOT_TV_HOST` defaults to `127.0.0.1`) starts a second
listener whose audience has no operator token (`crates/apps/bot/src/api/tv.rs`). It carries the table
view and nothing else:

| Method | Path | Answers |
|---|---|---|
| GET | `/api/health` | `{"ok": true, "public": true, "version", "commit"}` — the discriminator the client branches on |
| GET | `/api/tv` | `{"public": true, "bots": [<public table>, …]}`, the same shape as one entry of `/api/state`'s `bots` with the fields below removed |
| GET | `/api/tv/events` | SSE `table` events: `{"slot": n, "bot": <public table>}`, throttled at `state::TABLE_EVERY` like the dashboard's |
| any | `/api/{*rest}` | 404 `{"detail": "not on the public TV; the dashboard is elsewhere"}` |
| GET | anything else | the built page and its assets |

- The dashboard's own `/api/health` answers `{"ok": true, "public": false, …}`. It is the only
  reliable discriminator between the two listeners: the fallback service answers an unknown path with
  200 and the built `index.html` on both, so a status code says nothing.
- A public table is an **allow-list** of the dashboard's table payload: `slot`, `name`, `mode`,
  `status`, `connected`, `table_id`, `hand_id`, `board`, `seats`, `hero_seat`, `dealer_seat`,
  `actor_seat`, `pot`, `big_blind`; each seat keeps `seat`, `name`, `stack`, `bet`, `folded`,
  `status`, `last_action`, `avatar_url`. Absent by design: `hole` (a live hand shown to the opponents
  it is played against), `decision` and `version` (the policy's working), `turn`/`turn_started` (the
  think clock, which is a tell the project's own response model is fitted on), `last_error` and
  `season` (operator internals), and the per-seat `read` (the model's opinion of a named person).
  Unknown keys are dropped rather than passed through, so the dashboard's payload can grow without
  widening this one.
- The stream emits `table` events only: no `decision` (its equity is what the commentary is built
  from), no `result` and no `state` snapshot, which carries the metrics, the logs and the training
  state.
- Framing is the one header that differs from the dashboard's: `Content-Security-Policy:
  frame-ancestors *` instead of `X-Frame-Options: DENY`, because a public table view is meant to be
  embedded. The cache, `nosniff` and referrer rules are the dashboard's.
- The client (`web/src/main.tsx`) probes `/api/health` before anything else and renders nothing until
  it answers: the panels below mount with their own timers, so a first paint that guessed would ask
  this listener for routes it does not have. When `public` is true the client asks for no session and
  renders `TvMode` with the commentary, the scouting-report buttons and the exit link off
  (`web/src/table.tsx`).

## Abandoned update recovery

The release progress endpoint reconciles a saved running record with the updater owner's process
identity and the held release-operation lock. If neither is active after the startup grace, it
serves a failed record and retry guidance so the controls recover automatically. A manual release
remains active through its operation lock. Unknown ownership stays conservative; old ownerless
markers retain their age fallback. Reconciled elapsed time ends at the last recorded activity,
because an unreported exit time is unknown. This read does not rewrite source files or kill processes.

## Bot command delivery

The bot command endpoint acknowledges a split-fleet head's Start, Pause or Stop only after
persisting the desired mode for the worker. A failed write returns HTTP 503 and leaves the head's
displayed desired mode unchanged. A single-process fleet still applies the command locally when
persistence fails and logs the failure.

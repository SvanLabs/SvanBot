# Dashboard API contract

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

## Errors and stale data (0222)

- A request the server cannot answer returns a non-2xx status with `{"detail": "<reason>"}`: 404 for an
  unknown bot slot, hand or player, 500 when the store cannot be read or the handler failed, 401 without
  the operator session, 403 for a change sent to a non-loopback host when no operator token is set.
  An empty list with 200 always means "nothing recorded", never "could not read".
- The web client goes through `web/src/api.tsx`: `request` throws the server's `detail`; `usePoll`
  keeps the last good answer, reports the latest error and its age, and panels render `StaleNote`
  ("Update failed (...); showing data from HH:MM") while polls fail.
- Shared formatting lives in `web/src/format.ts` (`fmt`, `sgn`, `pct`, `SUITS`).
- Views (0237): a tab bar (Live, Opponents, Learning, Results, System, All; `widgets.tsx` `VIEWS`)
  shows a subset of the widget board in the user's own arrangement; the page opens on Live
  (2,507 px tall on a 1440-wide screen against 6,743 for All), the choice is remembered in the
  browser, arrow keys move between tabs, and "Arrange widgets" always shows the whole board. Views that
  leave a column empty get their own grid template; at phone width everything stacks.
- Web modules (0222): `main.tsx` holds the app shell and routing; `ui.tsx` the shared primitives
  (formatters, seat read, table themes, card, panel, empty state); `table.tsx` the live table, TV mode
  and decision strip; `training.tsx` the season, performance and learner panels; `panels.tsx` replay,
  the starting-hand guide and the opponent profile. Each opponent seat's `read` carries `size_tell`
  when a per-opponent river sizing tell is installed (0223), shown in the seat tooltip.

## Per-opponent reads (`GET /api/intel`, 0222)

`{fits: [{id: "fold"|"response"|"sizing", title, reads, stored, active, evidence: {gain_mnats,
half_width_mnats, n} | null, installed}], corrected_opponents, opponents: [{name, hands, fold_offset,
response_ratio, size_tell}]}`: the three per-opponent fits (0214, 0210, 0223) with the held-out evidence
of the stored fit and how many opponents each corrects in live play right now, and the 25 most-observed
opponents with any correction. A store read error answers 500. Panel: `web/src/intel.tsx`.

## Releases and one-click updates (0236)

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
  hand has no fetch); `percent` and `eta` weigh them by the last successful run's times
  (`artifacts/release-timings.json`), else defaults for the i7-4770K. The swap is confirmed by each
  process reporting its build: this process (`fleet`), the learner and analyst status, worker heartbeats.
- `GET /api/releases/log`: the 64 KB tail of `artifacts/release.log` (kept for tools).
- `GET /api/host` (0241): `{checks: [{key, label, value, status: ok|warn|info, advice}], checked_at}` —
  CPU and AVX2, microcode (judged against 0x28 on the i7-4770K only), frequency scaling, transparent
  huge pages, memory and swap, free space on the databases' disk and the archive disk (or a warning
  when the archive shares the SSD), `fstrim.timer`, kernel. Read fresh per request from `/proc`,
  `/sys` and systemd's timer links; unreadable facts are left out; `advice` is the operator's
  command. The System view's Host check panel polls it every 60 s. Nothing is ever changed.
- Panel: `web/src/updates.tsx` (`UpdatesPanel`, `UpdateProgress`); browser tests
  `web/tests/updates.spec.ts`.

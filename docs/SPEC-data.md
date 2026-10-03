# SPEC — data

> **Read this when** you touch the store, a schema, a compressed column or a backup.
> **Code:** `crates/libs/store`.
> **Related:** [`docs/OPERATIONS.md`](OPERATIONS.md) for backups, archives and restores in practice. · [All docs](README.md)

What SvanBot stores, in which format, and how it is protected. Source of truth is the code named in
each section; update this file with any schema or format change.

## Files

| Path | Owner | Contents | Protection |
|---|---|---|---|
| `artifacts/svanbot10.db` | `sv10-store::store::Store` | Live hands, decisions, replay records of big decisions, calibration samples, kv state, activity log | WAL, `synchronous=FULL`; startup `quick_check` with quarantine/restore; hourly sealed backups; per-hand digests |
| `artifacts/history.db` | `sv10-bot::history::HistoryDb` | Server hand-history exports and the multi-source corpus | WAL, `synchronous=NORMAL` (re-downloadable); startup check; weekly full and daily differential in the archive |
| `artifacts/backups/` | `tasks::backup_database` | `svanbot10-YYYYMMDDHH.db` (`SVANBOT_HOURLY_BACKUPS`: 3 kept, 2 when mirrored), `daily-svanbot10-YYYYMMDD.db` (`SVANBOT_DAILY_BACKUPS`: 1 kept), each with `.sha256` | Written by `VACUUM INTO` on a read-only connection of its own (the bots' writes never wait for it), checked, then sealed; a copy that fails its check is deleted |
| `SVANBOT_ARCHIVE_DIR/hourly/` (off by default; when `SVANBOT_MIRROR_HOURLY_BACKUPS` is set — with a second disk, or as the archive folder's own same-disk folder, whose state is `same_disk`, never `ok`, #725) | `tasks::mirror_backup` | Each hourly pair and its `.sha256` (`SVANBOT_MIRROR_HOURLY_BACKUPS` kept) | Moved straight to the name a restore reads, then removed from the SSD; a failed move discards the partial file and leaves the SSD pair; skipped when the disk lacks three copies plus 1 GB |
| `artifacts/data-format` | `sv10-store::packed::mark_data_format` | The highest data format written to the databases: `2` = cold JSON columns may be packed; absent = `1`, text only | `scripts/rollback.sh` refuses a build whose source reads a lower format; `archive unpack` converts back and writes `1` |
| `SVANBOT_ARCHIVE_DIR` (default `artifacts/archive`; here `/backup-disk/svanbot10` via `.env`) | `sv10-store::archive`, `archive` binary | `weekly/YYYY-Www/` (both databases full + `repo.bundle`), `daily/YYYY-MM-DD/` (live full + `history-delta.db` above the week's id watermarks), `monthly/YYYY-MM/` (weekly recompressed at zstd 19); each with a sealed `MANIFEST.json` | Stored and decompressed SHA-256 per file, row counts, watermarks, git commit; verified end to end before the staging dir is renamed in; restore checks hashes, `quick_check` and row counts; skipped when the disk lacks both databases' size + 1 GiB |
| `artifacts/quarantine/` | `sv10-store::integrity` | Damaged databases moved aside with a timestamp prefix | Never deleted automatically |
| `artifacts/tables/strengths-{flop,turn}.sv10tbl` | `sv10-equity::tables` | Exact per-combo board strengths for every suit-canonical flop/turn | Magic, shape and FNV-1a checksum verified on load; rebuilt by `tables build` (~31 s) |
| `crates/libs/equity/src/preflop_data.rs` | `sv10-equity::preflop` | Preflop class strengths and top-range equity ladder (exact f32 bits) | Compiled in; a test recomputes it |
| `artifacts/logs/` | `scripts/start.sh` | `svanbot10.log`, `learner.log` | Copy-truncate rotation at 50 MB, 3 generations |
| `artifacts/releases.log` | `scripts/release.sh` | One line per installed release | — |
| `.env` | operator | API keys, bot names, dashboard token, paths | Mode 600, gitignored, pre-commit secret scan |

External archives (the observation databases an older installation left behind, read by
`ingest archive <dir>`) are read-only sources: always opened `mode=ro&immutable=1` and
integrity-checked before reading.

## `svanbot10.db` schema

| Table | Columns | Notes |
|---|---|---|
| `hands` | `bot, hand_id` (primary key), `table_id, ended_at, hero_seat, hole, board, pot, net, winners, summary, showdown, digest, ev_net` | `hole`/`board` are concatenated card text; `winners` comma-separated names; `summary` is `HandSummary` JSON (each opponent action in its `history` carries `think_ms` — server-clock milliseconds since the previous event — and `street_open` when the previous event was new cards, from 2026-09-26; absent on older hands, our own actions and the first action after a resync); `digest` see below. Written before the hand updates the models. `ev_net` is the all-in luck-adjusted net (`sv10_model::allin`: the payout averaged over every runout when betting closed before the river with every live hand shown, minus what we put in; the net otherwise), filled by the fleet's background task within 30 s; it is outside the digest. |
| `decisions` | `id, bot, hand_id, ts, street, action, amount, equity, pot, to_call, latency_ms, detail, opponents, version, candidates` | `detail` is the `Decision` JSON (candidates, EVs, reason, champion version), **packed** (see Compressed columns); `opponents`, `version` and `candidates` (the candidate count) are copied out of it at insert and by the compaction, so SQL never reads inside the packed JSON |
| `replays` | `id, ts, bot, hand_id, net_digest, record` | Big spots only (pot ≥ 250 bb, call ≥ 100 bb or all-in): `record` (packed) is `sv10_bot::replay::ReplayRecord` JSON (seed, situation, params, players' and population stats, chosen action, candidate EVs); 14 days kept, pruned hourly |
| `replay_nets` | `digest, json` | Response networks referenced by replay records or queued audits, stored once per SHA-256 prefix; unreferenced ones pruned |
| `audit_queue` | `id, ts, bot, hand_id, net_digest, record` | Every live decision's `ReplayRecord` JSON (packed), waiting for the analyst; removed when audited, dropped after 1 day if the analyst is not running |
| `decision_audit` | `id, ts, bot, hand_id, street, live_action, deep_action, gap_bb, pot_bb, deep_ms, samples` | Analyst verdicts: the deep search's preferred action and the EV (bb, ≥ 0) the live choice gave up; 30 days kept |
| `scan_snapshots` | `id, ts, seen, digest, payload` | The findings scan's own inputs and outputs, one row per pass that changed anything: `payload` is that pass's JSON (see Scan snapshots), `seen` the last pass that read the same payload; `SCAN_SNAPSHOT_DAYS` (30) kept, pruned with the audits in the hourly backup |
| `calibration` | `id, bot, hand_id, ts, category, predicted, realized, scale` | Predicted vs realized chips per decision spot; `scale` = pot plus to-call at the decision (added 2026-09-15, backfilled) |
| `hand_provenance` | `bot, hand_id` (primary key), `target, arm, record, ts` | Experiment hands only: `arm` is `treatment` (a learner challenger) or `control` (the champion), `record` the provenance JSON (kind `parameter_challenger`/`champion_control`, target, assignment, season, champion, challenger, population watermark, net/range/calibration digests). Written in the hand's own transaction. Treatment hands are skipped by every production fit read (`hands_after`, `hands_page`, `recent_fit_hands`, the three decision-fit queries, the self-calibration insert and the live opponent-model update) through `sv10_store::store::ordinary_hand`; results, the race and the dashboard count them |
| `kv` | `key, value, updated` | Identical values are never rewritten |
| `events` | `id, ts, bot, level, message` | Activity log shown on the dashboard |
| `pack_dicts` | `id` (1–255), `family, created, bytes` | Preset dictionaries of the packed columns (see Compressed columns); never changed once written |

Columns `showdown`, `digest`, `scale`, `opponents`, `version` and `candidates` are added by in-place
migrations in `Store::open`.

### Scan snapshots

`scan_snapshots` holds what one pass of the findings scan read and what it emitted, so a past finding
is re-derived rather than quoted: the calibration summary as read, the installed
`calibration.v1` table **verbatim** — a single kv slot the learner overwrites in place every cycle,
the one input that is genuinely destroyed — the comparable classes with the window each was tested
on, every finding and measurement row with its value, and the constants that define the arithmetic
(`live_inputs_replay_version`, the `gap_*` values, `decision_loss_days`, `big_gap_bb`, `z95`, the
drift pair). `snapshot_version` names the record era, so a row written under an older shape is
identifiable.

The payload holds no wall clock, and the row is written only when its digest changes
(`Store::record_scan_snapshot`): an unchanged pass stamps `seen` on the newest row instead, so a year
of no movement stays one row and an empty table means the scan has not run — a different fact from a
scan that ran and saw nothing. `seen` minus a class's own `days` is the as-of cutoff behind its
counts. Retention is `SCAN_SNAPSHOT_DAYS` (30, pruned hourly beside the audits, on `seen` so the era
still being read is never dropped), coupled by compile-time assert to `AUDIT_RESULT_DAYS`, the
deepest window a payload summarizes — and 30 is the shortest that coupling allows, which is also
what the bytes argue for: one payload is ~30 KB and its inputs move nearly every pass (~1.5 MB a
day, ~44 MB resident, carried into every hourly copy; measured 2026-09-27), where an earlier estimate had guessed
a few hundred bytes a day and a year of it would be half a gigabyte. The payload is text, never a
packed frame: it is written once per change and read by a tool, never selected *into* by SQL — read
it with `Store::scan_snapshots`, never `json_extract`.

### `kv` keys

| Key | Writer | Value |
|---|---|---|
| `models.v1` | fleet, every 5 min and on shutdown | `ModelStore` JSON: per-player `PlayerStats`, population, `watermark` (last `hands` rowid folded in), `history_watermark`, `corpus_watermark`, `schema` (3) |
| `params.v1` | learner on promotion | Champion `Params` JSON with lineage version; the fleet hot-reloads it |
| `nn.response.v1` | learner | `StoredNet`: MLP weights, predictive `active` flag, chronology contract, paired-poker approval, validation and baseline log-loss, sample counts |
| `nn.response.candidate.v1` | learner | `StoredNet` awaiting the paired poker gate while an approved net is live; diagnostic only, never read by the fleet |
| `fold_calibration.v1` | learner (every cycle) | `FoldCalibration`: installed per-street fold logit shifts (flop, turn, river) and per-street evidence (n, predicted/actual fold rate, held-out gain and lower bound); read by the fleet into `Params::fold_logit_shift` |
| `river_jam_call.v1` | learner (every cycle; at start when missing) | `RiverJamFit`: equity shift for river calls against an all-in, with n, older-half over-estimate, held-out chips saved per call and its 95% lower bound, and whether it is active; read by the fleet into `Params::river_jam_call_shift` |
| `bot.names.<fingerprint>` | fleet (each start) | JSON list of every configured name an API key has played under; the fingerprint is the first 12 bytes of a domain-separated SHA-256 of the key (the key is never stored). The dashboard, fleet-check and season panels count all of a bot's names as one bot (identity: a key that plays on under a new name keeps its earlier names and its score, rather than starting a second bot) |
| `range_params.v1` | `calibrate` (daily via learner) | `StoredRangeParams`: fitted `RangeParams`, fit report, whether in use |
| `calibration.v1` | fleet | Aggregated per-spot corrections and residuals; `penalty_pot_cap` bounds a penalty per pot + call |
| `learner.status`, `learner.cycle`, `learner.experiments`, `learner.lineage`, `learner.clones.v2`, `learner.command` | learner / dashboard | Learner progress, last 40 experiments, promotion lineage, fitted clone pool, operator command (`learner.clones` is the pre-v2 key, no longer written) |
| `reputation.v1`, `season.lb.<season id>` | fleet, hourly | Reputation book from leaderboards; cached ended-season leaderboards |
| `history.status`, `integrity.status`, `hardware.profile` | fleet | Export download progress, last backup/digest check, detected hardware and tuning |
| `experiment.mode.v1` | fleet (head or only process), every 5 min | `experiment::ModeState`: champion/active, season, qualifying readings, protected trio, experiment pair, last validated reading, last error and switch reason. A restart overwrites it with champion mode; every process refuses a reading older than 8 min |
| `learner.experiment-targets.v1` | learner, every search cycle | `experiment::TargetQueue`: champion version, evidence watermark, the survivor in confirmation and every undecided ledger transition with its full challenger `Params` |
| `experiment.verdicts.v1` | fleet (head or only process) | Live verdicts by target id: `live-supported`, `live-harmful` or `inconclusive` with the estimate that reached it; target ids carry the champion and watermark, so a promotion or refit expires them |
| `learner.run.v1` | learner | `learner::run::Run`: the refresh or champion search in progress, stored after every step so a restart resumes it; empty when idle |
| `learner.rejection-ledger.v1` | learner | `search_ledger::Ledger` plus `confirm_rejected`: transitions a completed fresh-deal confirmation rejected under this scope, never offered as live targets |
| `season.current.v1` | fleet, every 2 min | `season::CurrentSeason` from `GET /season/current`: number, id and start time. The boundary the dashboard's season panels scope stored hands by; persisted so a restart scopes correctly before the first poll |

Champion `params.v1` and `learner.lineage` advance through one SQLite transaction after both JSON
payloads serialize successfully. A late write/commit failure leaves both prior values intact, so the
fleet cannot observe new parameters under an old lineage (or the inverse).

An active response model is exposed to decisions only when its stored layer shape exactly matches the
production feature layout, its training contract proves profiles were built strictly before each
sampled hand, and that exact artifact has passed fresh-deal paired poker evaluation. Legacy artifacts
missing either contract fail closed. Offline feature experiments use explicit layout names and never
write this key. `PriorStreetCalls38` counts a named seat's calls on completed flop/turn streets before
the sampled decision, capped at three and scaled by `1/3`; replay extraction and live inference share
the same history counter.

### Calibration units and frontier

`PendingCalibration` is captured only after the legalized action is successfully handed to the
connection writer. `predicted_incremental_chips` is the chosen candidate EV before the applied
calibration bias. Candidate call/raise EV already subtracts the action commitment, so
`hero_stack_before_action` is chips behind before that commitment and realization is exactly
`final_stack - hero_stack_before_action`. Multiple decisions in one hand retain independent
pre-action frontiers. `scale_chips` is pot plus the amount owed at the decision.

Samples remain in chips until hand settlement. Only the handler divides prediction, realization, and
scale by that hand's fixed positive big blind, exactly once, before inserting the row. A missing final
stack or non-positive big blind drops pending samples without inventing an outcome or unit; non-finite
values and non-positive scales are rejected. Applied category bias uses bb residuals; pot-relative
residuals remain diagnostic-only.

A category needs 60 samples before it is corrected at all. The applied bias is the part of the mean
residual that survives a 95% haircut (`|residual| − 1.96·se`), shrunk by `n / (n + 250)` and then
capped asymmetrically: **+3 bb** upward, because a residual is measured only where we already took
the action and raising its value extrapolates into unseen spots, and **−15 bb** downward, because
over-optimism is what spends chips and correcting it moves us toward checking, folding or a smaller
size. Every category is reported with its residual, standard error and applied bias whether
or not it is corrected.

### Crash recovery invariant

A hand row is committed before it is observed by the models. On startup every `hands` row above
`models.v1.watermark` is replayed into the models, so a crash between checkpoints loses nothing. A
checkpoint whose `schema` differs from `MODEL_SCHEMA` is discarded and the models are rebuilt from
all stored hands plus the history import.

## `history.db` schema

| Table | Columns | Notes |
|---|---|---|
| `raw` | `id, hand_id` (unique), `bot, table_id, hand_number, started_at, json, summary, profit` | One exported hand (`RawHand` JSON) and its replayed `HandSummary`, both **packed**; `profit` is the export's `$.profit`, copied out at insert and by the compaction |
| `corpus` | `id, hand_id` (unique), `source, bot, table_id, started_at, summary, digest` | Hands from other sources, `INSERT OR IGNORE` dedup; `summary` packed, `digest` over its text |
| `meta` | `key, value` | Download cursors and per-source import watermarks (`corpus:<source>:<path>…`) |
| `pack_dicts` | `id, family, created, bytes` | As in the main database, for this file's packed columns |

Corpus sources and their consumers (held-out gates decide):

| Source | Rows | Used by |
|---|---|---|
| `openpoker-archive-frames` | the archived live frames | Opponent statistics (full names and sizes the exports lack). Not the range fit or neural training (held-out worse). |
| `phh-*` (e.g. `phh-pluribus`) | 0 live (measured on a copy) | Nothing: names prefixed `<source>:`, `openpoker-` prefix refused; the Pluribus neural A/B was worse |

Only `openpoker-*` corpus rows ever reach opponent models (`corpus_only_after`, `recent_summaries`).

## Compressed columns

`decisions.detail`, `replays.record`, `audit_queue.record` (main) and `raw.json`, `raw.summary`,
`corpus.summary` (`history.db`) hold either text (rows from before) or an `sv10-pack` frame (a
BLOB): magic `SVZ`, one byte naming the preset dictionary (`pack_dicts.id`, 0 = none), the
uncompressed length (`u64` LE) and its CRC-32 (`u32` LE), then a raw DEFLATE stream (RFC 1951,
level 6). Any zlib reads it: `zlib.decompressobj(-15, zdict=<pack_dicts.bytes>)`. Readers go through
`sv10_store::packed::Codec::text`, which accepts both forms and fails on a length or CRC mismatch.

- **Writes**: new values are packed at insert with the column family's newest dictionary (none
  until the family has 200 rows).
- **Dictionaries**: the fleet trains one per family (`raw.json`, `summary`, `decisions.detail`,
  `record`) from whole rows spread over the newest 600, the newest last, 32 KB; ids are never
  reused and a dictionary never changes, so every copy of a database decodes on its own.
- **Compaction** (`sv10_bot::compaction`): in the fleet head, 500 rows per column per round in one
  transaction each, packing text rows and repacking dictionary-less frames once a dictionary exists;
  then `history.db` is VACUUMed when its free pages are over 128 MB and a fifth of the file. The main
  database reuses its free pages (`archive compact --vacuum-main` returns them with the fleet
  stopped, checking that no `hands` rowid moved). Progress: kv `compaction.status`, `review storage`.
- **Measured** on a copy of the live store: `history.db` 1,076 → 191 MB, `svanbot10.db` 763 → 497 MB,
  56 s for both. 15,000 sampled rows decoded by Python's zlib equal their originals, and the new
  columns match the JSON on every row. `review fold-cal` and `review raise-wars` print the same
  output, and read at the same speed (about 6.5 s).
- **Hand summaries stay text**: `hands.summary` (175 MB) is read on every model import and searched
  with `instr` for the opponent pages.
- **Opponent result blinds**: `hands_with_player_with_blinds` returns each matched hand with its
  own positive integer big blind from a valid summary. Missing, malformed, non-integer and
  non-positive blinds remain absent; chip results and the original opponent-hand query are unchanged.
- **Inspect** with `review decisions BOT HAND`, `review export HAND` and `review storage` (`sqlite3`
  shows packed values as blobs).

## Digests and checks

- **Hand digest** (`integrity::hand_digest`): SHA-256 over `bot, hand_id, ended_at, hole, board,
  summary` (always the text, never a packed frame), each prefixed by its little-endian `u64` length, truncated to 16 bytes, hex. Corpus rows
  use the same function with `(bot, hand_id, started_at, source, "", summary)`. Hourly verification
  logs mismatches to the activity log (`integrity.status`); `ingest` runs `verify_corpus` after imports.
- **Startup** (`integrity::ensure_healthy`): `PRAGMA quick_check(5)` on each database. A damaged file
  is moved to `quarantine/` and the newest backup whose sidecar SHA-256 and `quick_check` both pass is
  restored; with none, the database starts empty (history re-downloads).
- **Live damage**: the hourly backup checks the live database first; on failure the bot exits 70 and
  the supervisor restart runs the startup restore, so good backups are never rotated out behind a
  damaged copy.

## `SV10TBL1` board-table format (`sv10-equity::tables`)

Little-endian throughout:

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | Magic `SV10TBL1` |
| 8 | 4 | Board length (3 flop, 4 turn) |
| 12 | 4 | Board count `n` |
| 16 | 4 | Combos per row (1326) |
| 20 | 8·n | Sorted canonical board masks (`u64`) |
| 20+8n | 4·n·1326 | Strengths (`f32`), row per board, combo-index order in the canonical suit labelling |
| end−8 | 8 | FNV-1a 64 over every preceding byte |

Lookup canonicalizes the board (smallest mask over the 24 suit permutations), binary-searches the key
and maps combos back through the permutation. Files are written to `.tmp` and renamed.

## Hand formats

- `HandSummary` (`sv10-model::model`): `players [(seat, name)]`, `button`, `bb`, `history
  [ActionRecord]` (blinds excluded; `to` is the street total after the action), `board`, `shown
  [(seat, [card; 2])]` (showdown reveals only), `stacks [(seat, chips before blinds)]`.
- Anonymous seats (server exports name only winners) use the `ANON_PREFIX` name prefix and update
  population statistics only.
- Imports: server exports (`sv10-model::history`, street label is the post-action street), archived
  live frames (`sv10-venue::tracker::replay`), PHH files (`sv10-model::phh`, kept only when folders'
  chip flows reproduce the finishing stacks).

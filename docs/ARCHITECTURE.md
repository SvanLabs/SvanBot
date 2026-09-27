# SvanBot architecture

Version 10.0.0 (workspace `Cargo.toml` is the single version source). CPU-only, Rust 2024,
SQLite storage, React dashboard. User-facing guide: `docs/GUIDE.md` (served on the
dashboard). Planning and decisions: the issue tracker, <https://github.com/SvanLabs/SvanBot/issues>.
Specs: `SPEC-protocol.md`, `SPEC-data.md`, `SPEC-learner.md`; runbook `OPERATIONS.md`.

## Processes

```
                        dashboard (web/, React)
                                 ▲ state (SSE + API)
                                 │
 openpoker.ai ◄─ WebSocket+REST ─► sv10-bot ◄──── params.v1 ──── learner
 (tables, leader-                 5 bot tasks: seat, track,       fits, clone population,
  board, histories)               decide, act; hourly backup      challenger search
        │ exports                 │ ▲                               │ reads hands
        ▼                         ▼ │                               ▼
   history.db ◄──────────────  svanbot10.db  ◄──── audit queue ───► analyst
   (server exports,           (hands, decisions, kv,                deep re-solve of big
    training corpus)           calibration, audits)                 decisions; wiring table
                                  │
                                  ▼
                           archive (nightly) ──► /backup-disk (HDD): daily / weekly / monthly
```

The same three pictures (processes, decision path, learner loop) are drawn as SVG on the dashboard's
docs page from `diagram:` fences in `docs/GUIDE.md` (0300); keep the two in step.

| Process | Binary | Role |
|---|---|---|
| Fleet | `sv10-bot` | One tokio task per bot: connect, seat, track the table, decide, act; stores hands before models; serves the dashboard API; hourly checked backups; replay records of big decisions; a hand in progress when the process exits (hot swap or stop) is saved for the next one, which settles it from the resync replay's `hand_result` — stored once, with its real net (0315). Split mode (`SVANBOT_FLEET=split`, 0128) runs one head process (dashboard, background tasks, canonical models via the hand tailer) plus one worker per bot (`SVANBOT_WORKER=1` + `SVANBOT_ONLY=name`): workers write hand rows, heartbeat `bot.live.<name>` every 5 s and obey `bot.want.<name>`; default is the single all-in-one process |
| Learner | `learner` | Clone-fitted opponent population, neural response model training, successive-halving champion/challenger search, daily range refit via `calibrate`; paced by new hands and an operator-set cooldown; every job a stored run taken in steps of at most ~2 min (`sv10_bot::learner`, 0334) |
| Analyst | `analyst` | Measures the wiring table daily when idle (`wiring.v1`, 0316). Re-solves every live decision from the `audit_queue` with a deep search (10x the live budget, 16M samples on every core on the i7-4770K; 0161) and records whether the live choice matched and the EV it gave up under the record's own parameters, self-calibration included, which is post-pricing tactics and never a pricing error (`decision_audit`, 0346); when the queue is empty it re-runs the newest big-spot replays under a changed champion — the champion's knobs at the **recorded decision's own prices and live-fitted set**, adopted through the one `Params` method the promotion contract shares (`adopt_recorded_local`), so the only thing that varies against the recorded action is the knob set (0358, 0366; before 0366 the re-solve ran on `params.v1` alone, which by that same contract carries no live fits, so the flip rate over-attributed disagreement to the champion; 0357) — over a sample pinned to the current replay version (`REPLAY_VERSION`: a pre-v3 record carries no per-opponent corrections), in slices of at most 100 s between audit batches (0334); it stores the drift summary in `analyst.drift` once complete (0128), naming the basis tag, the budget (the analyst's 10x, the one deliberate difference from the recorded action), the population (replay ids, window, version mix, rows the filter dropped) and where the prices came from, and re-measures rather than accepting a row whose tag is missing or older — `review drift` prints that row and says whether it is current or due (0366; 0120 reads the number against a standing threshold); never changes play |
| Ingest | `ingest` | Imports other data sources into the corpus (archive frames, PHH files; read-only sources, resumable batches) and A/B-measures a source on the neural model |
| Archive | `archive` (nightly timer) | Weekly full / daily differential / monthly archives with sealed manifests on a second disk; verify and restore |
| Monitor | `scripts/monitor.py` | Read-only results alerts (big losses, nemesis opponents, stalls, fleet errors) |
| Tools | `review`, `calibrate`, `probe`, `sim`, `bench`, `tables` | Live review, leak finder and decision replay, range fit, benchmarks and canonical spots, samples/sec suites, simulations |

Supervision: `scripts/start.sh` runs the fleet under a crash-loop backoff supervisor (5 s doubling to
5 min), the learner at `nice 15` with low I/O priority and the OOM killer's first choice, the results
monitor, and log rotation; `scripts/svanbot10.service` starts it at boot.

## Crates

`crates/` has three folders (0225): `deps/` holds our own foundation crates that replace third-party
ones (`sv10-rng`, `sv10-digest`, `sv10-rt`, `sv10-mmap`, `sv10-pack`; the only third-party code they
use is `libc` for system calls), `libs/` the poker and data libraries (`sv10-cards`, `-equity`, `-engine`, `-nn`, `-model`,
`-policy`, `-venue`, `-store`, `-stats`), and `apps/` the programs (`sv10-bot`, `sv10-core`). The root
`Cargo.toml` names every internal path and every third-party version once, in
`[workspace.dependencies]`; crates depend with `x.workspace = true`. Documents written before
2026-09-26 name the old flat paths (`crates/<name>`); the crate names did not change:

<!-- docs-check: off (old paths on purpose) -->
| Before 2026-09-26 | Now | Crate |
|---|---|---|
| `crates/rng`, `crates/digest`, `crates/rt` | `crates/deps/rng`, `crates/deps/digest`, `crates/deps/rt` | `sv10-rng`, `sv10-digest`, `sv10-rt` |
| — (new) | `crates/deps/mmap`, `crates/deps/pack` | `sv10-mmap` (0228), `sv10-pack` (0229) |
| `crates/cards`, `crates/equity`, `crates/engine`, `crates/nn` | `crates/libs/cards`, `crates/libs/equity`, `crates/libs/engine`, `crates/libs/nn` | `sv10-cards`, `sv10-equity`, `sv10-engine`, `sv10-nn` |
| `crates/model`, `crates/policy`, `crates/venue`, `crates/store` | `crates/libs/model`, `crates/libs/policy`, `crates/libs/venue`, `crates/libs/store` | `sv10-model`, `sv10-policy`, `sv10-venue`, `sv10-store` |
| — (new) | `crates/libs/stats` | `sv10-stats` (0231) |
| `crates/bot`, `crates/core` | `crates/apps/bot`, `crates/apps/core` | `sv10-bot`, `sv10-core` |
<!-- docs-check: on -->

Layered, each with its own tests; `sv10-core` re-exports every module as `sv10_core::<module>` and
holds the tool binaries (`sim`, `probe`, `bench`, `tables`) and integration tests.

```
sv10-rng (no dependencies) ──── used by every logic crate and sv10-rt
sv10-digest (no dependencies) ─ used by sv10-venue, sv10-store, sv10-rt and sv10-bot
sv10-stats (no poker types) ──── used by sv10-model and sv10-bot
sv10-cards ─┬─ sv10-equity ─┐
            ├─ sv10-engine ─┼─ sv10-model ─┬─ sv10-policy ── sv10-core (facade) ─┐
 sv10-nn ───┴───────────────┘              ├─ sv10-venue ────────────────────────┼─ sv10-bot (I/O, drivers)
                                           └─ sv10-store ────────────────────────┤
                                              sv10-rt ───────────────────────────┘
```

| Crate | Modules |
|---|---|
| `sv10-rng`, `sv10-cards`, `sv10-nn`, `sv10-equity`, `sv10-engine`, `sv10-model`, `sv10-policy` (poker logic, no network or database; the only file access is `sv10-equity::tables` mapping the strength tables and `sv10-policy::hardware` reading `/proc`) | `sv10_rng` (xoshiro256++ with rand 0.10's exact samplers, no dependencies; reference vectors in `crates/deps/rng/tests`), `cards`, `eval` (7-card evaluator), `range`, `preflop` (class tables compiled in from `preflop_data.rs`, regenerated by `tables preflop-data`, checked bit-exact by a test), `equity` (Monte Carlo, shared deals, exact river/turn/flop strengths), `tables` (precomputed suit-canonical strength tables), `engine` (multiway NLHE rules, side pots, luck-free `expected_net`), `situation`, `model` (per-opponent stats), `oprange` (range reconstruction, board cache), `policy` (EV search), `features` + `nn` (neural response model), `agents` + `sim` (archetypes, clone fitting, paired evaluation), `calibrate`, `history`, `phh`, `allin` (all-in luck), `flow` (chips moved between two players in a stored hand, reconciled against the pot; head-to-head, 0221), `hardware` |
| `sv10-mmap` (0228; `libc` only) | Read-only `MAP_SHARED` mapping of a whole file with an audited `unsafe` core (bounds, alignment and endianness checked before viewing floats): the strength tables are one page-cache copy for every process instead of a 92 MB heap copy each (PSS 31 MB per process with three running) |
| `sv10-pack` (no dependencies; 0229) | Raw DEFLATE (RFC 1951: stored, fixed and dynamic Huffman blocks, lazy matching, length-limited codes; ratio equal to zlib at level 6), CRC-32 (slicing by 8), preset dictionaries prepared once (`Dictionary`), and the checked `SVZ` frame the stores' compressed columns use. The decoder is pinned by zlib-made fixtures, the encoder by `scripts/tests/test_pack.py` (zlib decodes what it writes) |
| `sv10-digest` (no dependencies; 0222) | SHA-256 (FIPS 180-4), HMAC-SHA-256 (RFC 2104), hex and constant-time compare, verified against the NIST and RFC 4231 vectors; replaced the `sha2` and `hex` crates for the state hash, store integrity, archives, replay digests and the dashboard session |
| `sv10-stats` (pure statistics, no poker types; 0231) | `moments` (running and sample means with interval half-widths), `normal` (inverse normal CDF, Bonferroni family-wise z), `logistic` (sigmoid, clamped logit and log-loss), `optimize` (golden-section search), `grading` (decision grades and accuracy, 0220), `margins` (calibration residuals by predicted-EV bin, 0222), `exp_memo` (an exact memo for `f32::exp`, bit-identical by construction; 0353). One tested home for primitives `sv10-bot` had copied into `analysis`, `headtohead`, `nnresidual`, `margins`, `foldcal` and `playerfold`, kept operation for operation (every `review` study prints the same numbers) |
| `sv10-venue` (protocol logic, no network) | `tracker` (openpoker frames → situations/hands, `replay`), `statehash` (server `state_hash` verification) |
| `sv10-store` (persistence) | `store` (SQLite: hands, decisions, replay records, calibration, kv, events; digests), `packed` (compressed cold JSON columns: codec, dictionaries, compaction batches, data format; 0229), `integrity` (checks, sealed backups, quarantine/restore), `archive` (daily/weekly/monthly archives, manifests, verified restore) |
| `sv10-rt` (runtime helpers, no third-party runtime) | `.env` loader, v4 ids, `statvfs` free space |
| `sv10-bot` (I/O, drivers; uses `sv10-rt`, `sv10-store`, `sv10-venue` at their own paths, no re-export fan) | `client` (openpoker connection; `client::seat` owns the seat lifecycle and the between-hands table moves, 0206), `setup` (dashboard bot setup → `.env`), `replay` (big-decision records and bit-exact re-runs), `history` (server exports, corpus, model import), `compaction` (packs stored cold JSON in the background, VACUUMs `history.db`; 0229), `neural`, `livefits` (the one module every live fit goes through: refit, load, install, report; 0205), `foldcal` + `raisewar` (the fits), `pacing` (learner cycle gate), `installs` (everything live play installs from the store — promoted params, neural and range models, compute profile, live fits, per-opponent corrections — with one rule: a failed read keeps what is installed; 0222), `jobs` (background job runner that logs a panicking job by name, and any job over the two-minute budget; 0222, 0334), `learner` (the learner's refresh and search as stored steps; 0334), `watchdog` (autonomy watchdog: learner, analyst, fold calibration, backup and the experiment poller's heartbeats against their limits, logged on each change; 0222, 0327), `margins` (calibration at the decision margin; 0222), `playersize` (per-opponent river sizing tells, store side; 0223); the statistics they share are in `sv10-stats`; `tasks` (periodic jobs), `api/*` (dashboard), `analysis`, `headtohead`, `reputation`, `guide`, `config`, `live` |

Clean `cargo build --release --workspace --bins` (no LTO, 256 codegen units, incremental; 0302): about
3 min on this box at 8 idle-priority threads; a one-file change rebuilds in ~16 s.
Toolchain pinned in `rust-toolchain.toml` (1.98.1, `rust-version` 1.98). Workspace `[workspace.lints]`:
`unsafe_code` denied everywhere except `sv10-rt` (env setup, `statvfs`) and `sv10-mmap` (mmap, munmap, madvise), `unused_qualifications`,
clippy `all`. CI-equivalent gate: `cargo clippy --workspace --all-targets` with zero warnings, `cargo fmt --check`. Structure decisions: ticket 0068 (crate architecture and drivers).

## Decision path (live)

```
 your_turn ──► situation ──► opponent ranges ──────────► shared deals
 hand_id,      seats, stacks, their stats, our image,     Monte Carlo on every core,
 turn_token    pot, board     size and timing tells       or exact runouts
                                                                │
      ┌─────────────────────────────────────────────────────────┘
      ▼
 price candidates ──► self-calibration ──► choose, legalize ──► send + record
 fold, check/call,    measured bias per    best EV, mixing      action with hand_id and
 raise sizes: stats   category, bounded    only near-equal      turn_token; decision,
 + network, fold      by margin evidence   options; only valid  replay and audit queue
 offsets, live fits                        actions
```

Live latency: 95 ms at the median, 325 ms at the 95th percentile, with the CPU shared by
other programs. Self-calibration decides the majority of preflop choices (0332; `review wiring`
counts the share over the fleet's own decisions).

1. `client` receives frames; `tracker` maintains seats, bets (from `table_state`), history and pot.
   At `hand_start` (or the first turn of a hand joined by resync) `experiment::latch` fixes the bot's
   policy for the whole hand: the champion, or — only for the experiment pair while the fleet holds
   #1–#4 and the last leaderboard reading is under 8 minutes old — one arm of the running target
   (treatment = the live champion with the learner challenger's knobs, local fits kept; control =
   the champion). The two pair bots swap arms every 120 hands (0291).
2. On `your_turn`, `tracker.situation` builds a `Situation`; `policy::decide_with` runs on a blocking
   thread with an 8 s timeout (fallback: legal check/fold).
3. `decide_with` first returns uncallable chips to their owner (`Situation::without_uncallable`),
   reconstructs each opponent's range (`oprange`), samples shared deals (live: `tuning.live_samples`,
   640x the learner's budget on a reference-speed machine, scaled down on slower ones (up to 1.6M samples, about 190 ms p50; 0161, was 160x in 0148), dealt in one seeded chunk per logical core by `SharedDeals::new_parallel` (heads-up with a flop or later, when every opponent combo × board completion fits the budget, the deals are the exact enumeration instead, each weighted by its combo's range weight: the river always, the turn and flop live; zero sampling noise and faster, 0165) and
   `equity_vs_ranges_parallel`; about 6 ms on the i7-4770K), and scores fold/call/check
   and several raise sizes by EV with opponent response models (fold estimates and all-in call
   equities corrected by the installed live fits, `livefits::LiveFits` — against an overbet all-in the
   haircut grows with the shove's size, `slope × (1 + ln(r / 1.5))` capped at 0.40, 0242; the response network's
   prediction for each opponent scaled by their own residual ratios, `Profile::response_ratio`, 0210;
   heads-up postflop fold estimates shifted by the opponent's own fold offset, `Profile::fold_logit_offset`, 0214;
   both installed through `playerfits::PlayerFits`, 0218; per-opponent river sizing tells tilt a bettor's river range, 0223, installed the same way; an
   aggressive action's think time against the actor's typical time — the server stamps every frame —
   tilts their range by the fitted `RangeParams::think_exp`, 0 until `calibrate` finds a held-out gain, 0234),
   then applies self-calibration
   (a penalty is capped at its category's supported per-pot residual, `policy::applied_bias`, 0207; an
   upward correction is bounded by the residual supported at the decision margin, predicted −1..+1 bb,
   `margins::MARGIN`, 0222, because the category mean is set by big-EV spots whose action it cannot change).
   `ResponsePricing` is the single in-process pricing module for one proposed raise-to: it derives
   each responder's fold/continue probabilities, continuing range, and raise-given-continue share
   from the same state and amount. Every responder is priced against the range they read *us* for
   (`ModelStore::hero_seen_view`): the fleet's own observed play, kept in the models as the image
   opponents form of us — per bot and in aggregate, decayed like any player's tallies — and weighted
   by `Params::hero_image`, 0 until the paired sims and the gate say the read is worth more than the
   population view it replaces (0321). The stat fold estimate is blended 30% with 70% neural fold
   output; postflop raise share blends 40% stats with 60% neural conditional raise output. A neural
   artifact is admitted only after layout, chronology, predictive, and fresh-deal paired-poker gates
   pass.
4. The chosen action is legalized against `valid_actions` and sent with `hand_id` + `turn_token`.
5. The decision's full inputs (`ReplayRecord`) go to `audit_queue` for the analyst; big spots also to `replays`.
6. `hand_result` → hand row stored (with digest, and for an experiment hand its `hand_provenance`
   row in the same transaction) → opponent models updated, except after a treatment hand, which also
   writes no self-calibration samples.

**Opponent tallies are recency-weighted** (0168): each live observation first decays that player's
decision tallies by `0.5^(1/1000)` (`ModelStore::half_life_hands`, `OPPONENT_HALF_LIFE_HANDS`); hand
counts and the population prior are not decayed. `review opponent-adapt` scores the newer half of the
stored hands under a range of half-lives and prints each one's gain over all-time tallies. The image
of our own play decays on the same cadence, so it describes how we have been playing lately (0321).

## Concurrency

- **Processes**: fleet (nice 0), analyst (nice 10, every logical core), learner (nice 15, every logical
  core), `calibrate` (spawned daily by the learner), results monitor, archive timer. The scheduler keeps
  live play first, deep re-solves second and the parameter search last.
- **Fleet threads**: tokio's multi-threaded runtime carries the bot connections; decisions run on the
  blocking pool and deal their Monte Carlo samples across the rayon pool. Dashboard handlers that read
  the store or build snapshots (`state`, SSE snapshots, hands, replay, fleet race, highlights, opponent
  detail) run through `api::off_runtime` on the blocking pool, never on a runtime worker. A handler
  that panics or cannot read the store answers 500 with a `detail` (`api::ApiError`, 0222); a
  snapshot figure whose read fails is left empty and logged at most once a minute
  (`api::snapshot_read`). Without an operator token, every POST must be addressed to a loopback host
  (DNS-rebinding guard), and every response carries `nosniff`, `X-Frame-Options: DENY` and a
  same-origin referrer policy.
- **Realtime dashboard** (`/api/events`, SSE, 0211): table events (`action`, `board`, `result`,
  `decision`, `hand`) are forwarded as they happen; every bot update sends that bot's whole live
  table (`api::state::table_json`, a few KB, each opponent seat with its model `read`, 0212) as a
  `table` event at most every 150 ms per bot, which the page merges in place; the full `state`
  snapshot (~100 KB with metrics, training and logs) comes every 5 s. The hand list refreshes on
  `hand` events and polls only every 30 s as a fallback. Split-fleet worker tables reach the head
  through heartbeats, so they follow the 5 s snapshot.
- **Store**: one writer connection behind a mutex plus four read-only connections (`Store::read`).
  WAL lets readers run beside the writer, so dashboard and learner queries never delay a bot's write.
- **Learner**: each successive-halving round plays the champion once per table and schedules every
  (candidate, table) run on one rayon pool (`sim::paired_eval_many`, bit-identical to separate
  `paired_eval` calls). Neural response features are extracted sequentially in chronological order:
  each hand sees only profiles observed before it, then advances those profiles.

## Season scope in the dashboard

A season is a separate contest with its own leaderboard, so what a panel *claims to be* decides
what it reads. Everything presented as this season's performance or standing scopes stored hands to
`season::CurrentSeason::started_at` (`/api/fleet`, `/api/highlights`, the per-bot `metrics` in
`/api/state`, and the headline result of `/api/analysis`), and every such payload carries a `season`
block (`scoped`, `number`, `id`, `started_at`) so the page labels what it actually read; with no
known boundary `scoped` is false and nothing is dropped. Figures worth keeping across the boundary
stay, labelled as lifetime (`all_time`), and are never merged into the season's.

Accumulated learning is the other side of the line and is never scoped: opponent models,
self-calibration, the fitted range model, the learner's champion and training state, the leak
finder's own lines, positions, outcomes, trend and advice, and the achievement badges, which stay
earned across a rollover.



## Invariants

- Never send an action absent from `valid_actions`; always echo `hand_id` and `turn_token`.
- Answer each `turn_token` at most once (reset by `action_rejected`); a dead WebSocket writer ends the session.
- Blinds exist only as `table_state` bets; `player_action.street` is the post-action street.
- A hand is persisted before it updates any model; checkpoints carry a watermark and startup replays.
- The learner promotes only on a positive 95% lower bound from sequential fresh-deal confirmation (`sv10_bot::promotion`).
- Experiment mode (0291) never changes a policy mid-hand, never touches the protected trio, fails
  closed on a failed, stale or unstored reading, and keeps treatment hands out of every production
  fit; its live evidence can retire a target or prioritize its confirmation, never promote it.
- Fitted models (neural response, per-opponent response correction, range fit) and any extra data source are used only while they beat
  their baselines on held-out live data.
- External archives are opened read-only and immutable; every database is integrity-checked before use.
- Speed-only changes must reproduce `sim paired` results exactly on a fixed seed.

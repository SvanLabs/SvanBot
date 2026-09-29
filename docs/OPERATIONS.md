# SvanBot operations runbook

> **Read this when** you are running a fleet: the everyday commands, updates and rollbacks,
> backups, benchmarks and fault drills.
> **Before this:** [Quick start](../README.md#-quick-start) in the README.
> **Related:** [`docs/GUIDE.md`](GUIDE.md) section 10 for the dashboard. · [All docs](README.md)

**On this page:** [Everyday](#everyday) · [Build and test](#build-and-test) · [Benchmarks](#benchmarks) ·
[Data](#data) · [Cleanliness](#cleanliness) · [Linux settings](#linux-settings) ·
[Fault drills](#fault-drills) · [Hot-swap releases](#hot-swap-releases) · [Split fleet](#split-fleet-off-by-default) ·
[Public TV](#public-tv-off-by-default)

All commands run from the repository root. Binaries live in `target/release/`.

## Everyday

| Task | Command |
|---|---|
| Start fleet + learner + dashboard | `scripts/start.sh` (builds if binaries are missing; `REBUILD=1` forces) |
| Stop everything | `scripts/stop.sh [--hold 30m\|8h\|forever]`: the keepalive leaves the fleet down for the hold (default 30 min); `systemctl --user stop svanbot10` holds forever; `scripts/start.sh` clears it |
| Keepalive | `scripts/keepalive.sh` restarts `svanbot10.service` when no supervisor runs, no hold is in force and no release holds the lock; decisions in `artifacts/logs/keepalive.log`, `KEEPALIVE_DRY=1` to check. Install once: `scripts/units.sh && systemctl --user enable --now svanbot10-keepalive.timer` |
| Rename a bot | Change its name in `.env` (`SVANBOT_MAIN_NAME` / `BOT_n_NAME`) and restart. Its season stays one record because each start links the names under the API key. For names used before that record existed, set `SVANBOT_ALIASES=New:Old[,New2:Old2]` once. The server export labels older hands with the new name, so fleet-check matches export rows by hand id |
| Update from GitHub (one click) | Dashboard → Releases & updates → **Update**: fetches the update branch (`SVANBOT_UPDATE_BRANCH`, default `main`; `SVANBOT_UPDATE_REMOTE`, default `origin`), fast-forwards this checkout, runs `scripts/release.sh`, and shows a progress bar (stages, time left, bots still playing, the hot swap). By hand: `scripts/update.sh`; `scripts/update.sh --check` only fetches and prints `<behind> <commit>`. It refuses uncommitted build inputs and local commits the branch lacks; a checkout that is *ahead* of the branch with every one of its commits on a remote branch has nothing to install, and the run says which branch line it is on and exits 0 without building anything (#394); a checkout whose history is unrelated to the branch is a different repository and moves onto it only with `SVANBOT_ADOPT_UPSTREAM=1` (see *Moving a checkout onto this repository*); a failed release restores the checkout, and play never stops. **First Update on an older install**: the installed build predates the fetch, so run `git pull` once (then click Update or run `scripts/release.sh`) |
| Ship a new build without stopping play | `scripts/release.sh` (committed tree; lint, the test build and the release build side by side at idle CPU priority, then the tests; ~40 s for a one-file change, ~4 min cold; installs atomically; the fleet hot-swaps when no bot is mid-turn, the learner between steps (at most ~2 min); every stage prints its time and one over 120 s is a warning, and a stage over budget keeps its log as `target/stage/<stage>.over-budget.log`; the build stage names another cargo holding its build lock before waiting on it (`scripts/build-lock.sh`), because that wait is charged to the build and reads afterwards as a slow compile; history in `artifacts/releases.log`, progress in `artifacts/release-progress.json`) |
| Restart only the fleet | `scripts/restart-bot.sh` |
| Season boundary check | `scripts/season-check.sh before LABEL` shortly before a season ends, `scripts/season-check.sh after LABEL` about an hour into the next: carry-over (opponent models, champion lineage, response/range models, stored hands) and live state (every bot on the new season, playing, season hands reset); PASS/FAIL lines, exit 1 on failure, files in `artifacts/season-checks/` |
| Add, remove or switch off bots; change keys, buy-in, seeking | Dashboard → Settings → Open bot setup (`#setup`): validated, "Check key" asks openpoker.ai for the registered name, saves `.env` atomically (previous copy `artifacts/.env.previous`, mode 600) and restarts the fleet at the next moment no bot is mid-turn. Without `SVANBOT_WEB__OPERATOR_TOKEN` it only works on a loopback-bound dashboard opened as 127.0.0.1 |
| Work on the dashboard | `cd web && npm run dev` serves `web/src` with hot reload and proxies `/api` to the running dashboard (`SVANBOT_WEB_PORT`, default 5000; `SVANBOT_API` overrides the target). Log in with the operator token as usual |
| Status, version, per-bot results | `scripts/status.sh` |
| Install or refresh the systemd units | `scripts/units.sh`: renders `scripts/svanbot10*.service` and `scripts/svanbot10*.timer` for **this** checkout into `~/.config/systemd/user/` and reloads systemd. The units in the tree are templates naming the checkout as `@SVANBOT_ROOT@`, so an install anywhere under any `$HOME` gets units that point at itself; `cp`ing them by hand installs a unit that points at nothing. `scripts/units.sh --check` names any installed unit that is missing or was rendered for another directory (`scripts/status.sh` reports it), and `scripts/units.sh --print svanbot10.service` writes one rendered unit to stdout. `scripts/units.sh --setup` additionally enables the service and the timers for boot where a user systemd instance answers, and says how to enable by hand where none does; `scripts/setup.sh` runs it after a successful release, so enabling by hand is only for relocations and installs setup.sh never saw |
| Move an install to another directory | `scripts/stop.sh --hold forever`, move (or copy) the whole checkout, `scripts/units.sh` in the new location, `systemctl --user enable --now svanbot10.service`, `scripts/start.sh`. Runtime data is `artifacts/` inside the checkout, so it travels with it; an `SVANBOT_ARCHIVE_DIR` outside the checkout is unaffected |
| Autostart at boot | `systemctl --user enable svanbot10.service` (disable to keep it off); fresh installs get this from `scripts/setup.sh`, which enables the service and the timers through `scripts/units.sh --setup` |
| Review after decision changes | `./target/release/review all 5` |
| Leak finder as JSON | `./target/release/review leaks` |
| Fold-prediction calibration by street | `./target/release/review fold-cal` (the learner's fit, printed, not stored) |
| Equity estimate vs exact showdown equity | `./target/release/review raise-wars` (heads-up postflop decisions that committed the pot, estimate vs exact equity against the shown hand, by raises before, street, estimate and villain bet size; the river, deep-pot, overbet and size-scaled overbet call fits with their held-out gates) |
| Opponent recency study | `./target/release/review opponent-adapt` (replays every stored hand and scores later opponent decisions from all-time vs recency-weighted tallies at several half-lives) |
| Replay big decisions | `./target/release/review replay 20` (recorded inputs; exit 1 unless every one is bit-identical), `review replay id=123`, `review replay 50 --current` (what today's promoted parameters would do in the same spots: the champion's knobs on the record's own budget and prices — its self-calibration, range and live fits — so the only thing that varies is the knob set; the footer names the basis) |
| Calibration at the decision margin | `./target/release/review margins [DAYS] [CATEGORY...]` (residual by predicted-EV bin before and after the live self-calibration correction; `*` = off at 95%) |
| Opponent timing tells | `./target/release/review timing-tells` (timed opponent actions by context, and the shown strength behind fast, typical and slow postflop aggression against each actor's typical time; the live `think_exp`) |
| Opponent river sizing | `./target/release/review sizing-tells` (heterogeneity study: bet size vs shown strength, pool and per opponent), `review sizing-fit` (the learner's per-opponent tell fit at several priors, not stored) |
| Stored-hand digests | `./target/release/review verify-digests [FILE]` (recompute every hand's content digest and optionally a file's SHA-256; exit 1 on any mismatch) |
| State-hash mismatches | `./target/release/review state-hash [N]` (diagnosed mismatches, newest first: STALE replay vs DIVERGED with the field census; the full snapshot is kept in the incident table) |
| Autonomy watchdog | `./target/release/review autonomy` (learner, analyst, fold calibration, backups and the experiment poller: `ok` or `STALE` with age and limit; exit 1 if any is stale). The fleet head runs the same check every 10 minutes and logs `autonomy: …` once when a loop stops and once when it recovers |
| Every review command | `./target/release/review help` |
| Logs | `artifacts/logs/svanbot10.log`, `artifacts/logs/learner.log`, `artifacts/logs/monitor.log` |
| Results monitor | started and stopped with the fleet by `scripts/start.sh`/`stop.sh` (by hand: `python3 scripts/monitor.py --big-loss-bb 250`). Read-only: 30-minute SUMMARY (per bot, the opponents at our worst and best tables, toughest all-time by the same chip flow) and OPPONENTS (new names, most-played opponents with style, VPIP/PFR and our net with them at the table — a table result shared by everyone dealt in, not a head-to-head attribution), BIGWIN/BIGLOSS (≥ 250 bb), NEMESIS (an opponent beating us in chips moved between us and them in champion hands — the experiment arms' treatment hands are out, as in the bot's own ledger — at 95% family-wise across every opponent tested), STALL, ERROR. `--once` prints one pass |
| Query the live database by hand | Always read-only: `sqlite3 "file:artifacts/svanbot10.db?mode=ro"` (or `-readonly`). A read-write session that holds a transaction blocks the fleet's writes past its 10 s busy timeout, which loses hands and stalls turns. Failed hand inserts are now retried every 30 s, but a stalled write still delays the frame loop |
| Who held the write lock | Every take of the write connection is timed: a hold or a wait of ≥ 1 s logs a WARN naming the site that took it — `write connection held 12.3 s at crates/apps/bot/src/…:123 (artifacts/svanbot10.db); pid 4567`, with `— long enough to fail every other writer` once the hold passes the 10 s `busy_timeout`, and `waited 12.3 s for the write connection at …` when the hold was elsewhere. So the first `database is locked` after a stall names a file:line instead of leaving it to guesswork; thresholds and wording are in `crates/libs/store/src/store/slow.rs` |
| Figures the store could not answer | A bot's snapshot figures always carry `stale` and `error` beside them: a failed results read serves the last good reading (never zeros) and names the failure, and with no previous reading serves the panel's shape zeroed and flagged. `monitor.replays.recorded` is `null` with a reason rather than `0` when `replay_stats` fails. The store warning behind both is logged at most once a minute (`snapshot_warn`), since every panel asks on every poll |
| A funding decision never invents a balance | `/season/me` is read for a join or a top-up through `balance_from` (`crates/apps/bot/src/client/rest.rs`): this read when it carries a balance, else the last payload on record, else nothing — in which case the join is deferred and the top-up is skipped, each with a log line, and the 600 s top-up cooldown is stamped only once a decision was actually made |
| A/B against the live pool | `sqlite3 -readonly artifacts/svanbot10.db "select value from kv where key='models.v1'" > /tmp/m.json`, then `SIM_MODELS=/tmp/m.json SIM_A='<params>' SIM_B='<params>' ./target/release/sim paired 32 1500` (~30 s with the fleet playing; `SIM_STACK_BB`, `SIM_SEED`). The current champion is `params.v1` in the same table |
| Where a setting's value came from | The startup log names every numeric setting and the value in force after clamping (`settings: SVANBOT_BUY_IN 5000, ...`), so an operator never has to read `.env` and the dashboard side by side |
| Are we losing *right now* | `./target/release/review recent [BOT] [N]` — the last N hands: net and all-in EV per hand with 95% intervals, the luck in chips, and the current streak. The season aggregate cannot answer it (a wide interval around a big positive number hides a bad afternoon in both directions), and the session net in `status` resets at every hot swap, so a dashboard figure right after a swap is a handful of hands, not a trend |
| The fleet's own findings | The Autonomy panel's **WHAT THE FLEET FOUND** block, and `findings.v1` in the store. Every 30 minutes the head re-runs the instruments: the deep re-solve's per-decision loss by street and action (P0, over 0.02 bb/decision across 500+; the classes are a table — class, n, bb per decision, its 95% interval, the decisions in the window, and what the floor made of them (`P0 · filed`, `measured`, `not yet decidable`, `never queued`) — so a class too rare to read is not mistaken for a clean one, and the analyst's filter, rendered from `worth_auditing`'s own constants, is stated once above the table rather than in every row), the nemesis test (P1, over champion hands: the experiment arms' treatment hands are out of the ledger), **style drift** (P1: the share of hands whose first preflop decision was each action, the last day against the six before, flagged at 5 points and 5 standard errors, reported as one finding per decision point so a shift and its counter-shift are one change; a several-fold drop in first-in raising after a calibration-rule change can go unnoticed for a day), and categories whose price is off (P2, a *measurement* — the rule is that a residual is not a loss: the number is realized minus the **uncorrected** predicted EV, over every settled decision, which is the population self-calibration is fitted from and not the audit's big spots). A new P0 finding is filed as a ticket by the fleet itself, labelled `found-by-fleet`, and **that ticket closes itself** when the class stops reproducing; at most three per pass. Unless there is nothing to say, the block leads with its coverage — how little of the window carries the live inputs the deep re-solve grades, and any class whose population could not be read — because that is the reason a scan with thirty rows can still decide nothing, and a question the scan could not measure is named under the list, so an empty scan is not mistaken for a clean bill |
| Where we actually lose chips | `./target/release/review audit-by [DAYS] [REPLAY_VERSION]` — the analyst's deep re-solve by street, by the action we took, and by the category the bot priced (the last by joining the audit to the decision record). This is the *decision* cost *given the prices live play used*: the deep re-solve takes the record's own parameters with the self-calibration corrections in them, so a pricing error the correction has absorbed cannot appear in the gap, and the rows are the analyst's big spots (pot >= 50 bb, a call of at least a quarter of the pot and 12.5 bb, or any all-in), never every decision. `review margins` is the *pricing* residual on every settled decision, and the two disagree: a mispriced action we were right to take costs nothing. A row's best-candidate column is a modal candidate, not a preference: most costed disagreements are another *size* of the action we took rather than a different action, so the table prints that split beside it. A gap is only comparable across records that carry the same inputs, so every verdict stores the replay version it graded: the header always states the mix (`version not recorded` rows are pre-column ones and are excluded by any filter), and `<days> 3` reads the records that carry the per-opponent corrections live play used |
| Did a promotion start behaving differently? | `./target/release/review drift` — the analyst's post-promotion row (`analyst.drift`): today's champion's knobs re-solved at the newest big-spot replays' **own recorded prices** (the record's self-calibration, range and live fits) give the share of big-spot actions that flip and the mean/max deep gap in bb. It is the same price basis and budget as the `audit-by` gaps above, differing only in the policy graded — the champion's knobs here, the record's there (before that the re-solve ran on `params.v1` alone, which carries no live fits, so the flip rate over-attributed disagreement to the champion). The row names the basis tag, the budget, the population (replay ids, window, version mix, rows the version filter dropped) and where the prices came from, and the check re-measures rather than keeping a row whose tag is missing or older, so the "grows well past 4%" reading can never be taken from a number measured on another basis — a row written by an older build carries no tag at all and prints as due for a re-check |
| Where the chips go against one opponent | `./target/release/review rival NAME... [since=DATE]` — the chips that moved between us and them (flow) and our whole net at their tables, each raw and with all-in luck removed, with 95% intervals and the hands a win would need to be proven; the flow split by how it ended, street, pot type, position and who raised last preflop; the biggest confrontations; and how we answer their flop c-bet against every other raiser's, by what we held. Use a `since=` window: all five of the biggest pots against the heaviest rival were played before banking capped stacks |
| Is each component worth anything? | `./target/release/review wiring [N]` re-runs the newest N recorded big decisions with each live component switched off in turn and reports, per component, the share of decisions that move and the EV they give up per decision under the full model — plus how many records replay exactly as recorded. It stores the report the dashboard's **Wiring table** panel reads (`wiring.v1`); the analyst refreshes it daily, and only while the audit queue is empty, so live audits always win. Below it, **self-calibration's reach over every decision of the last 24 h**: per street, the share whose best action changes when each candidate's recorded bias is removed. The big decisions above barely see it, yet it decides the majority of preflop choices, mostly fold to call |
| Six-max benchmark | `./target/release/bench6 4000` plays the champion against itself or `SIM_B='{...}' bench6 4000` against a challenger over **identical deals at a six-max table**, against a frozen archetype pool (station, maniac, nit, tag, lag) — no store, no live population, so the same command measures the same thing on any machine in any season. Reports bb/100 per arm, the paired difference with its 95% interval, and the breakdown **by position** and by how each hand ended, which is the only measurement that can see a seat and a hands-up number cannot. Runs in seconds. The level printed is that pool with default parameters, not the live fleet: read the difference, not the level |
| Learner pacing | Two independent jobs. Evidence refreshes wait only for a new hand (#314), so the fits live play reads are never much more than a refresh old; they run at idle CPU priority and `learner_threads` (compute profile) is their share of the box. Champion search keeps its schedule: after each cooldown during season days 1–3, then at the dashboard Autonomy panel's new-hands limit. A due search yields to a refresh only when the fits are 500 hands (one evidence epoch) behind, which is also the scope the rejection ledger and the experiment queue are measured under. Missing season state uses the late-season rule. The panel shows the next job, hands left, season day and reason, and names a running refresh. Settings live in `learner.settings` and override `.env`; defaults are `LEARNER_MIN_NEW_HANDS=500` (champion search only), 60-minute cooldown, no backoff and 6 h maximum search idle. `LEARNER_THREADS` is overridden by the dashboard compute profile. Start champion search now with "Start next search early" (`POST /api/training/command {"command":"start"}`, picked up within 10 s). The +1 bb/100 fresh-deal promotion gate is unchanged |
| Experiment mode | Dashboard → Learning → Experiment mode; `./target/release/review experiment` (stored mode, targets with live hands and their verdicts) and `review experiment TARGET` (estimate, gate, every hand with arm, decision ids and replay ids) |
| Learner rejection ledger | `./target/release/review ledger` (which search transitions are barred as decided-dead and which are still accumulating evidence, for the live champion and evidence watermark) |

## Build and test

```
scripts/check.sh full                                  # anti-regression gate: rustfmt, clippy -D warnings, cargo-deny, workspace tests, tsc (~20 s after a one-file edit; each step timed, over 120 s warned)
scripts/check.sh deep                                  # the same with property/fuzz tests at 100k cases
scripts/release.sh                                     # what the fleet runs (never plain cargo build into target/release: it would swap in untested)
python3 scripts/test.py                                # every workspace test, built with the gate profile and run in parallel
python3 scripts/test.py -p sv10-policy -- decide       # one crate, tests whose name contains "decide"
CARGO_TARGET_DIR=target/dev cargo test --profile gate -p sv10-core --test characterization   # the golden snapshot alone
CARGO_TARGET_DIR=target/dev cargo clippy --release --workspace --all-targets   # zero warnings (workspace lints)
cargo fmt --all --check                                # rustfmt.toml: max_width 140
cd web && npm run build                                # tsc 7 (strict, noUnused*) + Vite 8
cd web && npx playwright test                          # browser tests against scripts/web-test-server.sh
```

The release profile has no LTO (256 codegen units, incremental): thin LTO re-optimized the whole
program once per binary, 110 of the 135 s of a one-file release, and the reference `sim paired 48 1200`
runs 32 s either way with bit-identical results. Tests build with the `gate` profile: the release
settings with `sv10-bot` at opt-level 1 (its one-file test rebuild 27 s → 6 s, the suite no slower;
every other crate, including the golden snapshot's poker math, stays at opt-level 3). Compilers use
all 8 threads at idle CPU priority (`nice 19` + `SCHED_IDLE`); live decision p95 during a release stayed
at 301 ms (381 ms the hour before). Measured 2026-09-27 on the i7-4770K: one-file release 8 m 36 s →
39 s, cold release ~9 min → 3 m 44 s, one-file `check.sh full` ~2 min → 20 s. `scripts/test.py` runs only test executables (never `sim`,
`probe`, `tables`), each in its package directory, as many at once as cores and memory allow; it
records each binary's peak RSS in `$CARGO_TARGET_DIR/test-rss.json` and prints the slowest.
`scripts/release.sh` builds the tests in `target/dev` (the cache the pre-commit hook keeps warm; only the
shipped binaries carry the commit id and build in `target/stage`) and runs them before installing.

Toolchain: Rust 1.98.1 pinned in `rust-toolchain.toml`, Node 26, React 19.3, Vite 8, TypeScript 7.
`scripts/web-test-server.sh` runs the dev `sv10-bot` in a throwaway root on port 5099. It uses a fake
key, keeps the bots stopped and points every endpoint at a closed local port, so the tests never touch
openpoker.ai or the live databases. 72 browser tests across 12 spec files, all passing.

Database resource measurements from `scripts/resource-report.py` use a read-only, WAL-aware
transaction, so committed table pages are included while the fleet is running.

Portable and offline builds:

```
scripts/portable.sh        # x86-64-v2 + v3 binaries, scripts, dashboard, docs -> target/dist/*.tar.gz (32 MB, needs glibc >= MANIFEST)
scripts/install.sh         # in the unpacked bundle: picks v3/v2 from /proc/cpuinfo flags, installs into target/release, creates .env
scripts/vendor.sh          # vendor/ (349 MB, gitignored) + .cargo/vendor.toml; then cargo build --release --offline --config .cargo/vendor.toml
```

Cross targets: the pure-logic crates (`sv10-core` and below) `cargo check` for
`aarch64-unknown-linux-gnu` and build fully static for `x86_64-unknown-linux-musl`
(`cargo build --release --target x86_64-unknown-linux-musl -p sv10-core --bins`). The fleet binary
additionally compiles C (bundled SQLite, aws-lc for TLS), so those targets need a C cross toolchain:
`gcc-aarch64-linux-gnu` or `musl-tools` (not installed on this box; building musl C against glibc
headers fails on the removed `*64` symbols and must not be forced).

Before committing: the pre-commit hook (`scripts/pre-commit.sh`, install with
`ln -sf ../../scripts/pre-commit.sh .git/hooks/pre-commit`) runs `scripts/check.sh commit`: the secret
scan (refuses `.env` files and staged lines containing any `.env` value), and when `crates/` is staged
rustfmt, clippy `-D warnings`, the golden snapshot and the property tests; tsc when `web/src` is staged.
`release.sh` runs `check.sh lint` (rustfmt, clippy, cargo-deny) alongside its builds.

Property and fuzz tests print a seed on failure; `SV10_PROP_CASES` scales them:
`crates/apps/core/tests/properties.rs` (engine chip conservation and zero-sum settlement; policy decisions
legal and finite from random reachable states), `sv10-venue` `corrupted_frame_streams_never_panic`
(random corruptions of a captured frame stream through the live dispatch and turn situations),
`sv10-bot` `legalize_only_returns_offered_actions`. Dependency policy: `deny.toml` (permissive
licenses, crates.io only, no wildcards).

## Benchmarks

Pause the learner first, so it does not compete for the CPU: `pkill -STOP -f target/release/learner`,
and resume it with `-CONT` afterwards.

| Benchmark | Command | 2026-09-15 reference (i7-4770K, fleet stopped) |
|---|---|---|
| Micro | `./target/release/probe --bench` (the native build) | eval 24 ns; decide 3-way flop 0.98 ms; cold flop strengths 2.40 ms |
| Instruction levels | `probe --bench` from `target/dist-v3/release` and `target/dist-v2/release` | best of 3 with the fleet running: native eval 24.0 ns / decide 0.93 ms / cold flop 11.50 ms; v3 23.9 / 0.94 / 11.64; v2 24.7 / 0.96 / 12.04; paired sim identical on all three |
| Sampler | `sim paired 24 1000`, learner paused, 5 interleaved runs | median 14.46 → 13.68 s after the guided branch-free combo sampler; −2.41 bb/100 unchanged; MLP predict 2.1 µs in `probe --bench` |
| Continuation | same as above | median 13.39 → 11.79 s after per-rung preflop combo scores and presorted order; result unchanged |
| Tables path (2026-09-16) | `sim paired 8 400` with the live pool, run from outside the repo | 30 s → 2 s: `tables_dir` now falls back to the executable's ancestors, where it used to silently recompute board strengths |
| Paired throughput | `SIM_B='{"call_margin":0.005}' ./target/release/sim paired 12 600` | 4 s with tables (−1.13 bb/100, 95% −9.40..+7.15 since exact heads-up river deals, 2026-09-23; −0.20, −7.83..+7.43 after the chance correction; before it +1.02, −7.78..+9.81); result must be identical across speed-only changes |
| Cold start | `./target/release/probe --bench` (first lines) | preflop class table and top-range ladder 0 ms (compiled in; were 278 + 265 ms of 8 threads per process), hardware detect 2 ms; `probe` run 0.53 → 0.03 s |
| Board tables | `./target/release/tables build` / `tables check` | 31 s build; flop 9.3 MB, turn 87.3 MB in `artifacts/tables` |

### Samples per second (`bench`)

`bench [learner|live|micro|all] [--repeat N] [--profile] [--allocs] [--threads N]` measures the work
the fleet does in one process, each suite printing one JSON line per repeat with a checksum, so a
speed-only change is shown to compute the same numbers:

- `learner` — the paired champion-against-challenger evaluation the learner runs, in table runs per
  wall second and per CPU second.
- `live` — one decision at a time at the live sample budget over fixed spots; latency p50…p999.
- `micro` — the 7-card evaluator, the combo sampler, shared deals, heads-up equity and the RNG.

The workload is frozen in `artifacts/bench-fixture.json`: `learner bench-fixture` writes it from the
live store (the champion as searches play it, the population models, the response net and this
machine's sample budgets); with no fixture the archetype pool and default parameters stand in.

Two builds are compared with `scripts/bench-ab.py BASE NEW --suite learner --repeat 10`, which
alternates them back to back on the shared machine and reports the paired ratio with a 95%
t-interval — a gain counts only when the interval excludes 1, and the checksums must match. `perf` is
blocked on this host (`perf_event_paranoid` 3, no root); `--profile` samples process CPU time with
`SIGPROF` and symbolizes through `addr2line`, so build with `--profile profiling` for source lines.
`--allocs` counts allocations and the heap peak, off by default (the counters cost about 4%).

#### Phase 1 baseline (2026-09-27)

Conditions: the fleet playing, the learner **running** for `learner`/`micro` and **paused** for `live`
(the latency suite is the runbook convention), ten paired repeats, medians. The reference column is
the build before the deals/sampler pass; `artifacts/bench-fixture.json` is the frozen workload.

| Suite | Metric | Reference | After | Paired ratio (95%) |
|---|---|---|---|---|
| learner | table runs / CPU-s | 0.850 | 0.924 | **1.089** (1.084–1.094) |
| learner | table runs / s | 4.51 | 4.94 | **1.079** (1.012–1.145) |
| live | decision mean | 95.7 ms | 81.2 ms | **0.850** (0.842–0.858) |
| live | decision p95 | 228.8 ms | 190.0 ms | **0.831** (0.820–0.843) |
| live | decision p99 | 247.9 ms | 211.0 ms | **0.857** (0.830–0.883) |
| live | decision p50 | 76.7 ms | 67.6 ms | **0.876** (0.816–0.936) |
| live | CPU s per repeat | 66.7 | 60.2 | **0.903** (0.900–0.907) |
| live | decision p999 | 259.4 ms | 248.7 ms | 0.950 (0.814–1.086) |
| micro | combo sample | 20.6 ns | 14.7 ns | **0.763** (0.656–0.870) |
| micro | deal, 2 opponents | 150 ns | 119 ns | **0.841** (0.797–0.886) |
| micro | reweight, 2 opponents | 16.0 ns | 13.4 ns | **0.883** (0.810–0.955) |
| micro | reweight, 1 opponent | 15.2 ns | 12.0 ns | **0.836** (0.758–0.914) |
| micro | deal, 1 opponent | 91 ns | 70 ns | 0.885 (0.710–1.059) |
| micro | evaluator | 17.4 ns | 17.7 ns | 1.052 (0.920–1.185) |
| micro | heads-up equity | 11.4 M/s | 12.0 M/s | 0.998 (0.908–1.087) |
| micro | RNG | 1.70 ns | 1.62 ns | 0.984 (0.940–1.027) |

**Only the ratios compare builds, never the absolute columns**: the evaluator reads 10.9 ns on a
fully idle box and 17.4 ns here, where the fleet and the two alternating bench processes share the
four cores — a load factor of about 1.6 that moves with the hour. The two cells with an interval
reaching 1 (deal 1-opponent, evaluator) are the ones where the 2-opponent cell of the same code path
is significant, so the effect is real and the interval is contention noise. The machine has throttled
7.1 h since boot (2.5% of uptime), spread over the day, which is inside the same noise. Raw outputs:
`/tmp/ab-{learner,micro,live}.{json,log}`; re-measure on a quiet box before quoting a cell.

## Data

| Task | Command / action |
|---|---|
| Manual backup | automatic hourly into `artifacts/backups/` (sealed with `.sha256`; `VACUUM INTO` on its own read-only connection, so bots never wait on it); the newest `SVANBOT_HOURLY_BACKUPS` (default 3, or 2 when mirrored; ~580 MB each) and `SVANBOT_DAILY_BACKUPS` (default 1) dailies are kept on the SSD, and anything older is in the nightly archive on the second disk; an hourly move of the sealed pair to that disk (`<archive>/hourly/`, the SSD copy is removed) is **off by default**: set `SVANBOT_MIRROR_HOURLY_BACKUPS=24` to turn it on, which also drops the SSD to 2 hourlies; older points live in the nightly archive (`archive list`) |
| Verify a backup | `sha256sum -c <(echo "$(cat F.sha256)  F")` and `sqlite3 F 'pragma quick_check'` |
| Second-disk move fails | The pair goes straight to the names a restore reads — no temporary name, no read-back — so a failure removes the partial file (a half-written file under the real name would read as this hour's backup) and leaves the SSD pair untouched; the failure is loud and non-fatal: `error!` in `artifacts/logs/svanbot10.log`, a `fleet` error (newest one printed by `scripts/status.sh`), and `training.storage.mirror` = `{state: failed, dir, error}` on the dashboard. The message names the step and the file; `Structure needs cleaning (os error 117)` is the kernel's EUCLEAN — the second disk needs `fsck` with it unmounted, which is an operator action. The operator's call (2026-09-28, #374) is to stop guarding the hour with this disk: it rejects the dance, not the data, so the next hour tries again |
| Restore | automatic on startup when a database fails its check; manual: stop, copy a sealed backup over `artifacts/svanbot10.db`, start |
| Compressed columns | automatic: new decision details, replay/audit records and history exports are stored packed, and the fleet packs older rows in the background (then VACUUMs `history.db`). `./target/release/review storage` shows rows still text, free pages and the last pass; `review decisions BOT HAND` and `review export HAND` print the JSON (`sqlite3` shows blobs). `./target/release/archive compact [--dir DIR] [--vacuum-main]` packs everything now (`--vacuum-main` only with the fleet stopped; it checks no hand rowid moved); `archive unpack` (fleet stopped) converts back to text before installing a build that predates the packed columns |
| Archive (second disk) | `svanbot10-archive.timer` runs `./target/release/archive run` at 04:30 into `SVANBOT_ARCHIVE_DIR` (`.env`; here `/backup-disk/svanbot10`, default `artifacts/archive`): weekly full, else a daily differential; the month's archive; keeps 14 daily / 8 weekly / 12 monthly. Install with `scripts/archive-timer.sh` |
| Inspect archives | `./target/release/archive list`; `archive verify --deep` (all) or `archive verify weekly/2026-W38` |
| Restore from the archive | `./target/release/archive restore daily/YYYY-MM-DD --to /tmp/restore` (never into `artifacts/`); then `scripts/stop.sh`, copy `svanbot10.db` and `history.db` over `artifacts/` (remove their `-wal`/`-shm`), `scripts/start.sh`. The code: `git clone /tmp/restore/repo.bundle` from a weekly or monthly |
| Data snapshot | Runtime data is not part of this repository: `svanbot10.db`/`history.db` via sqlite `.backup` + zstd, plus `backups`, `release-snapshots`, `misc` (tables, logs, season checks) and screenshots tarballs with `SHA256SUMS`, published as `data-YYYYMMDD` releases on a repository named in `SVANBOT_DATA_REPO`; `.env` never uploaded. Restore (fleet stopped): `scripts/fetch-data.sh` (`--all` for backups and snapshots; `FORCE=1` to overwrite). The public derived set for a release (aggregates + schema, never raw opponent hands) comes from `archive export-derived --to DIR`, scrubbed and failing closed; fetch it with `scripts/fetch-data.sh --derived` (fleet may run). Cloud sessions: `scripts/cloud-setup.sh` |
| Quarantined files | `artifacts/quarantine/` (damaged databases moved aside, never deleted automatically) |
| Import a PHH tree / measure a source | `./target/release/ingest phh <dir> <source> --dry-run`; `SVANBOT10_ROOT=<copy of artifacts parent> ./target/release/ingest neural-ab <source> 3` |
| Import archived frames | `./target/release/ingest archive <dir> --dry-run`, then without `--dry-run` (idempotent, resumable) |
| Refit range model | `./target/release/calibrate 20000` (`CALIBRATE_CORPUS=1` to measure the corpus; `CALIBRATE_START=live CALIBRATE_FREEZE=a,b` starts from the live fitted set with fields held, a dry run for shape-term A/B tests; `CALIBRATE_LINES=1` reports range calibration per postflop line type; every fit logs the held-out likelihood split by the shown player's largest bet — under 1.5x, 1.5–4x, 4x+ pot — so a size term shows where it helps) |

## Cleanliness

| Task | Command |
|---|---|
| Report | `scripts/clean.sh`: unknown entries in `artifacts/` (a copied `.env` is named, never removed: it holds the keys), non-rotation files in `artifacts/backups/`, untracked files, oldest rotated logs, superseded build artifacts, leaked test and release scratch dirs, session screenshots, disk |
| Apply safe actions | `scripts/clean.sh --apply` (also every 6 hours by `svanbot10-clean.timer`, independent of the archive, so a failing archive no longer stops cleanup; install with `scripts/archive-timer.sh`): remove a whole build profile no project command uses (`target/dev/debug`, made only by a bare `cargo build`/`cargo test`; 5.7 GB on 2026-09-27) once a day passes without a write to it, and build variants nothing has read for two days (`scripts/prune-builds.py`; cargo never deletes a replaced `<crate>-<hash>`, and one profile held 18 copies of the bot crate — the first run freed 10.6 GB of 21), `target/tmp` and `target/udeps`, `sv10-*` scratch dirs in `$TMPDIR` and `target/.web-stage-*` older than an hour (`/tmp` is RAM: 11,033 leaked test dirs held 2.4 GB), screenshots in `artifacts/` older than a day; prune worktrees, zstd the oldest rotated log, drop `target/dev` under 10 GB free. Every build-artifact removal waits for the guard that skips it while cargo or rustc runs, and a run that leaves `target/dev` alone because of it says so in the log. Never touches databases, backups, tables, `.env` files or `target/release`. `scripts/test.py` gives each run its own `TMPDIR` and removes it, so test runs no longer leak |
| Unused dependencies | `RUSTFLAGS="-C target-cpu=native -W unused-crate-dependencies" CARGO_TARGET_DIR=target/udeps cargo check --release --workspace --lib` (bins and tests may still need a flagged crate: verify with `--all-targets`) |
| One-off archives | `/backup-disk/svanbot10/one-off/<date>-<what>.tar.zst` with `.MANIFEST.txt` and `.sha256` for anything that does not belong in the nightly archive |

## Linux settings

| Setting | Where | Why |
|---|---|---|
| Learner `nice 15`, `ionice -c2 -n7`, `oom_score_adj +500` | `scripts/start.sh` learner supervisor (inherited by each run) | live play keeps the CPU, disk and memory under contention |
| Archive `Nice=15`, idle I/O, `OOMScoreAdjust=500` | `scripts/svanbot10-archive.service` | same |
| Pressure (PSI) | `scripts/status.sh` last line | re-tune only if cpu some avg60 > ~20% or io full > ~10% sustained |
| THP `always`, swappiness 5, schedutil, mq-deadline | system defaults, left as is | measured sufficient; sched_ext is not in the Debian kernel |

**Host checklist (verified; the dashboard's System view shows each item as the Host check
panel):**

1. Microcode: `sudo apt install intel-microcode` (enable `non-free-firmware` in the APT sources), then
   reboot; `grep -m1 microcode /proc/cpuinfo` should show `0x28` on the i7-4770K. No BIOS flash is
   needed.
2. SSD TRIM: `systemctl is-enabled fstrim.timer`; if not, `sudo systemctl enable --now fstrim.timer`
   (the Kingston A400 is DRAM-less).
3. Archive disk: `SVANBOT_ARCHIVE_DIR=/backup-disk/svanbot10` in `.env`, so the nightly archives go to the HDD
   (the hourly backup mirror is off by default).
4. Leave alone: THP `always`, intel_pstate, SQLite `synchronous=FULL` on the live database, and swap
   as the host already has it configured.
5. Optional: the Radeon HD 5870 has no compute use (no Vulkan or ROCm for TeraScale 2). Removing it
   and using the i7's HD 4600 outputs saves roughly 20–30 W at idle.

## Fault drills

| Drill | Expected outcome | Last run |
|---|---|---|
| Corrupt `svanbot10.db` header while stopped, start | error logged, file in `quarantine/`, newest sealed backup restored | Every build: `sv10-store` test `damaged_database_is_restored_from_newest_verified_backup` |
| Corrupt the newest backup too | that backup skipped (hash mismatch), an older one restored | Same test (a rotted newer backup is skipped) |
| No backup at all | file quarantined, database starts empty | Every build: `damaged_database_without_backup_is_quarantined_and_starts_empty` |
| Kill `ingest` mid-run, re-run | resumes from the batch watermark, no duplicates (`verify_corpus` 0 mismatches) | 2026-09-15 on a scratch root: killed after batch 1 (1,000 rows, watermark 1000); re-run added 9,000, 10,000 distinct rows, 0 mismatches |
| Hot-swap release while seated | fleet exits 75 when no bot is mid-turn, supervisor restarts at once, seats resync (a hand in progress at that moment is saved and settled by the new process from its resync replay); learner swaps between steps | Run live: every bot connected again within the server's 120 s grace window |
| Restore the monthly archive into a scratch dir | the databases rebuilt, hashes and row counts verified, `repo.bundle` clones | Run by hand into a scratch dir: `archive restore monthly/YYYY-MM` completed with every hash and row count verified and the bundle cloned; daily differentials: every build, `sv10-store` test `weekly_daily_monthly_restore_and_prune` |
| Revoke a bot key | that bot stops with `auth_failed` (mode `error`), no reconnect loop; other bots continue | Not run (needs a real key revoked); code path `SessionEnd::Fatal` |
| Seated, traffic but no hands for 10 min | watchdog logs a warning, leaves, re-queues | Not run deliberately (needs a stuck live table); code in `client::mod` |
| Live DB damaged while running | hourly check fails → bot exits 70 → supervisor restart restores | Restore half covered by the tests above; exit path in `tasks::backup` not run live |
| Second disk refuses a move | no half-copied file is left under an hour's name, the SSD pair stays where it is, play and the SSD backups carry on, and the reason is in the log and the storage row | Seen live on a failing second disk (EUCLEAN `Structure needs cleaning`, 22 times between 2026-09-26 11:32 and 2026-09-28 05:58, the check-by-check copy rejected every time it was tried); the kernel's filesystem error counter for that disk is printed by the Host panel's `Filesystem errors` row; every build: `sv10-bot` tests `hourly_backups_are_moved_to_the_second_disk_and_keep_the_newest`, `a_failed_move_leaves_the_ssd_pair_and_names_the_step` |
| Server drops every connection | all bots reconnect with backoff and resync | Seen live (a broken pipe on every bot at once): all reconnected within seconds |

## Hot-swap releases

`scripts/release.sh` first validates every managed path and holds
`artifacts/release-operation.lock` for the complete snapshot/build/install operation. It builds Rust
in `target/stage` and the dashboard in a separate target staging directory, runs the workspace
tests, checks the required binaries, then installs exact executable and web directory sets through
`scripts/rollback.sh --install`. Target and web must share a filesystem so the directory renames are
atomic; a later swap failure performs compensating renames back to the complete prior sets. The
commit is baked into `--version` and `/api/health`, and the run output is tee'd to
`artifacts/release.log` (install records stay in `artifacts/releases.log`).

Before lint or build, the release script resolves the latest installed identity and creates
`artifacts/release-snapshots/<commit>/`. The snapshot contains every regular executable in
`target/release`, the complete `web/dist` tree, the installed-commit marker, and `SHA256SUMS`; it
becomes visible only after every copy and hash check succeeds. Only the newest `SV10_KEEP_SNAPSHOTS`
(default 5, ~1.3 GB) are kept (older ones archived on 2026-09-24 are in `/backup-disk/svanbot10/release-snapshots`): each new snapshot prunes the oldest, never itself (26 had piled up
when the disk filled on 2026-09-24). An existing snapshot is verified and
kept, never overwritten. On the one-time transition from legacy two-field `--version` output, the
marker is adopted only when the latest valid release record, the inode of a running installed bot,
and the local `/api/health` commit all agree. Missing or conflicting evidence blocks release.
Every 15 s the fleet compares its executable's inode, size and mtime with the one
it started from; a replacement untouched for 20 s that answers `--version` triggers the swap: wait
(at most 90 s) until no bot has an unanswered turn, save models, exit 75. The supervisor restarts
at once on 75 (5 s under supervisors started before this change); bots reconnect, get
`already_seated` and resync their tables inside the server's 120 s grace window. Hands are stored
before models update, so a swap loses nothing that startup replay and history import cannot
restore. Drill: `grep -n "hot swap\|swapping" artifacts/logs/svanbot10.log` after a release.

### Dashboard updates

The Releases & updates widget replaces SSH for routine updates: the installed commit (from
`releases.log`), the live build (`/api/health`), what the update branch on GitHub holds (checked every
30 minutes and on **Check now**), and the grouped changelog of what an update would install. **Update**
opens a confirm dialog, then `POST /api/releases/update` starts `scripts/update.sh` detached in its own
process group (fetch, fast-forward, `release.sh`, install; refused with 409 on uncommitted build inputs
or a concurrent run; lock at `artifacts/release.lock`, removed however the run ends, aged out after
2 h). The progress bar (`GET /api/releases/progress`) weighs the stages by the last run's times, shows
the time left and the bots still playing, then the hot swap per process; a reload mid-update picks the
run up again. On failure it names the stage; the fleet keeps playing the installed build and the
checkout returns to it.

**Roll back to a saved build**: every update snapshots the
build it replaces; the widget lists them, and a confirmed **Roll back** runs `scripts/update.sh
--rollback <commit>` → `scripts/rollback.sh <commit>` — the same hash-verified, atomic restore as the
terminal, with the same lock and progress bar, and play continues. Rolling back never moves the
checkout, so the next Update offers the newer commits again. A build that predates the packed columns cannot read
them: the widget marks it "reads only uncompressed data" and `rollback.sh` refuses it
(it compares the build's `DATA_FORMAT` with `artifacts/data-format`); to go back that far, stop the
fleet, run `./target/release/archive unpack`, then roll back.

### Verified code rollback

List available snapshots, select the exact prior installed commit, and verify it before restoration:

```bash
find artifacts/release-snapshots -mindepth 1 -maxdepth 1 -type d -printf '%f\n'
scripts/rollback.sh --verify <commit>
scripts/rollback.sh <commit>
```

**An installed build with no commit identity** (a build compiled straight into `target/release`
instead of through `release.sh`) cannot be snapshotted by commit, so `release.sh` refuses it. With operator approval, `RELEASE_ADOPT_UNIDENTIFIED=1 scripts/release.sh` copies it to
`artifacts/unidentified-builds/<UTC stamp>/` (binaries, dashboard, `VERSIONS`, `SHA256SUMS`) and then
releases. It proceeds only while a verified snapshot exists, and that snapshot stays the rollback target.
Restoring the preserved copy is manual: check it with `sha256sum --check SHA256SUMS` in that directory.

Rollback refuses unresolved commits, incomplete manifests, changed hashes, unsafe or symlinked
managed paths, cross-filesystem swaps, and missing, non-executable, or wrongly identified required
binaries before touching installed files. The global operation lock excludes releases, snapshots,
and other rollbacks. It stages and rechecks exact binary/web directory sets, swaps them by atomic
same-filesystem renames with compensating restoration on a later failure, and appends
`rollback from <previous-commit>` to `artifacts/releases.log`. Watch for the normal hot swap:

```bash
grep -n "hot swap\|swapping" artifacts/logs/svanbot10.log | tail -20
scripts/status.sh
```

Confirm that the fleet returns without action rejections and that `/api/health` reports the restored
commit. This restores code and the dashboard only. If SQLite integrity failed, stop services and use
the separately verified `archive restore` procedure; never combine database restoration with a live
code rollback.

### Moving a checkout onto this repository

**The one-time case this covers**: a fleet cloned from a private tree, moved onto the repository that
tree publishes to. The two histories share no commit — the published repository is produced from the
private one by a filtered export, which rewrites every hash — so `git merge-base` between the
checkout's head and the update branch's tip is empty. That is not a diverged checkout with commits worth
keeping; it is a different repository, and the only way onto the branch is to put the checkout on it.

`update.sh` cannot do it by fast-forward and must not do it by merge. A merge commit joining two
unrelated histories has no ancestor relation to the update branch, so `--ff-only` would fail after it
forever and the Update button would be dead for good.

Git will still make the move through its ordinary fast-forward if it is told, for the length of one
command, that the histories are related:
`git replace --graft <the branch's root commit> <this checkout's head>` gives the branch's oldest
commit a parent in this checkout, which makes that head an ancestor of the branch. `git merge --ff-only`
then moves the tree exactly as it does on every other update, and the replace ref is deleted as soon as
it has. Doing it that way rather than with a `reset --hard` buys two things: git's own guards apply to
the move, so it **refuses** when uncommitted edits to a tracked file would be overwritten rather than
discarding them, and what is left behind is an ordinary clone — nothing in `refs/replace`, no standing
claim that the two histories are one.

Adoption is destructive all the same — the working tree becomes the branch's tree — so it happens only
when an operator said so in advance:

```bash
git remote set-url origin https://github.com/SvanLabs/SvanBot.git
SVANBOT_ADOPT_UPSTREAM=1 scripts/update.sh    # or set it in .env, then press Update
```

`SVANBOT_ADOPT_UPSTREAM=1` is the whole permission, and it is read from the environment precisely
because the dashboard starts the script — a button press cannot supply it, so the button alone can
never adopt. Without it the run stops with `unrelated history: set SVANBOT_ADOPT_UPSTREAM=1 to move
this checkout onto <branch>`, which is a different refusal from the `local commits not on <branch>`
a merely diverged checkout gets; that one never rewrites anything, and it still never does. Neither
is the third case — a checkout ahead of `<branch>` whose commits are all on some remote branch
(#394). That one installs nothing and rewrites nothing either, but it is not a fault: the checkout
is simply further along a line than the branch this install follows, so the run reports it, exits 0,
and says which line (`SVANBOT_UPDATE_BRANCH=<that branch>` follows it here instead).

What the adoption keeps, in the order it matters:

- **Everything git does not track.** `artifacts/` — the store, `release-snapshots/`, the hourly
  backups, `bulk/`, the logs, `.env` — is untracked or ignored, and the fast-forward rewrites tracked
  files only. This is why the checkout is adopted in place instead of cloned: the running fleet keeps
  reading the store from the path it already has, with no copy of a live database to get wrong.
- **Every commit the checkout had.** Before anything moves, its head is written to
  `refs/adopt/before-upstream-<UTC stamp>`. `rollback.sh` resolves commits in this repository, so this
  ref is what keeps the installed private build rollback-able once the tree it was built from is gone.
- **A record of what was dropped.** Files tracked here and absent from the branch are counted and the
  first 20 written to `artifacts/release.log`, all of them recoverable from that ref — for example
  `git checkout refs/adopt/before-upstream-<stamp> -- .claude/` to bring back local skills the
  published repository does not carry.

It is refused while `scripts/rollback.sh --validate-source-clean` finds uncommitted build inputs, and
again by git itself if a tracked file has uncommitted edits the move would overwrite — in both cases
the checkout is left exactly as it was. A `release.sh` failure afterwards resets the checkout to the
commit it started on — the same recovery every other update has, so the source on disk still matches
the installed build. After one adoption, every later Update is the ordinary fast-forward on this
branch.

**The scripted bootstrap.** A checkout whose own `update.sh` predates the adoption path cannot adopt
itself, and one carrying uncommitted edits is refused. `scripts/adopt-upstream.sh`, run from a
checkout of this repository, sets the stage for it:

```bash
scripts/adopt-upstream.sh --dir /path/to/fleet --dry-run   # what it would revert; changes nothing
scripts/adopt-upstream.sh --dir /path/to/fleet [--backup DIR]
```

It refuses while `artifacts/release.lock` exists, and exits without changes if the checkout is already
on the branch. Otherwise it writes a mode-700 backup (default `adopt-backup-<UTC stamp>` beside the
checkout): `repo.bundle` (every ref), `worktree.tar.gz` (tracked and untracked source, ignored paths
left out), `uncommitted.patch`, `env`, and inventories of `artifacts/` before and after. Only then does
it revert the uncommitted tracked edits, rename `origin` to `private` and point `origin` here, and run
this repository's `update.sh` from an untracked copy inside the checkout with
`SVANBOT_ADOPT_UPSTREAM=1` — the same adoption described above, not a second copy of it. It does not
copy `artifacts/`; nothing in the move touches it. The by-hand recipe above remains for a checkout
that is already clean and already has a current `update.sh`. Verify it took:

```bash
git remote -v                                      # origin is this repository
git log --oneline -1                               # a commit that exists on the update branch
git for-each-ref --format='%(refname)' refs/adopt  # what this checkout used to be
git for-each-ref --format='%(refname)' refs/replace # empty: no graft was left behind
git status --porcelain                             # empty: the tree is the branch's tree
scripts/update.sh --check                          # a normal count, not the whole branch's history
```

## Split fleet (off by default)

`SVANBOT_FLEET=split` (in `.env`, the environment wins; it has been run live and rolled back to `all`, because the dashboard sees workers only through 5 s heartbeats and gets none of their realtime events) makes `scripts/start.sh` run a head process plus one worker per bot from `.env`
instead of the single all-in-one fleet. The head serves the dashboard (worker bots appear with
`remote: true`), runs every background task and owns the canonical models; workers play, write
hand rows, heartbeat every 5 s and obey operator commands relayed through `bot.want.<name>`. Workers also play with everything live: the params watcher (promotions, fold and river-jam shifts, response and range models), their own season clock, self-calibration applied locally (only the head writes the table), and the head's opponent-model checkpoint reloaded within 30 s of each save. `scripts/restart-bot.sh` restarts the head and every worker.
Switching modes, like any fleet restart, is an operator `scripts/stop.sh` + `scripts/start.sh`
(the permission classifier blocks agent restarts of the live fleet). Rollback is the same two
commands without the env var. Hot-swap releases work per process: each watches the binary inode
and exits 75 at its own no-mid-turn moment (workers never save models, so a worker swap cannot
drop observations). Heartbeats older than 30 s show the slot as local-offline, never a dead
worker's last state.

## Public TV (off by default)

`SVANBOT_TV_PORT` (default `0`) starts a second listener that serves the table view to an audience
with no operator token; `SVANBOT_TV_HOST` (default `127.0.0.1`) is where it binds. Setting the port is
what turns it on:

```sh
SVANBOT_TV_PORT=8788 scripts/start.sh
```

**What it carries:** the built page, `/api/health`, `/api/tv` (every bot's table, projected for a
spectator) and `/api/tv/events` (the same tables pushed as they change). **What it answers instead:**
404 to every other `/api/` path. No dashboard route is mounted on this listener at all, so
`/api/state`, `/api/events`, every `POST` and every operator read are *absent* rather than
unauthorized. `crates/apps/bot/src/api/tv/tests.rs` opens a socket and asserts exactly that, and the
payload is an allow-list (`crates/apps/bot/src/api/tv.rs`): hole cards, the policy's decision and its
version, the think clock, the per-seat opponent read and the operator's own figures are dropped, so a
field added to the dashboard's table payload is absent here until somebody adds it on purpose.

**The listener is the whole security boundary.** There is no token to lose, so binding it is
publishing it: `0.0.0.0` puts a table view on the internet with nothing in front of it. For a public
stream, leave the bind on loopback and put a TLS reverse proxy in front — which is also where a
hostname, a certificate and a rate limit live:

```
# Caddyfile
tv.example.com {
    reverse_proxy 127.0.0.1:8788
}
```

The dashboard's `SVANBOT_WEB__OPERATOR_TOKEN` does not apply to this listener in any way.

**Check which listener you reached.** `/api/health` answers on both, and each names itself:

```sh
curl -s localhost:8788/api/health     # {"ok":true,"public":true,…}  ← the TV
curl -s localhost:5000/api/health     # {"ok":true,"public":false,…} ← the dashboard
```

`public` is the only reliable discriminator. An unknown path outside `/api/` answers 200 with the
built page on both listeners, so a status code from a browser says nothing about which one you hit;
the web client probes `/api/health` first and renders the table view without a login prompt when
`public` is true.

**Verify one before pointing an audience at it:**

```sh
curl -s localhost:8788/api/health | grep -o '"public":true'
curl -s localhost:8788/api/tv | head -c 300                          # tables; no "hole", no "decision"
curl -s -o /dev/null -w '%{http_code}\n' localhost:8788/api/state    # 404, never 401
```

A **401** from that last line means you pointed at the dashboard's port. A **200** means you published
the dashboard itself: `/api/state` on the TV is not merely denied, it is not there.

Three things the TV does not have, on purpose: no commentary (it is built from decision events
carrying equity, and the public stream does not carry them), no scouting reports (a seat plate on the
TV is a label, not a button — the read behind it is the model's opinion of a named person), and no
exit link (there is nowhere to exit to). Setting the port back to `0` or unsetting it leaves the
dashboard untouched.

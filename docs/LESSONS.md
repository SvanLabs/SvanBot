# Lessons

> **Read this before** changing decisions, the learner, the client, data or operations. Each entry
> is a mistake this project already paid for, what it cost, and the *Rule* it left behind.
> **Tip:** read the section for the area you are about to touch; most refused changes repeat one of
> these. · [All docs](README.md)

**On this page:** [Strength and measurement](#strength-and-measurement) ·
[Rewrites and releases](#rewrites-and-releases) · [Protocol and live play](#protocol-and-live-play) ·
[Data and operations](#data-and-operations)

## Strength and measurement

1. **Intuition is not evidence.** Fixes that looked obviously right measured negative: pricing
   villain 3-/4-bets preflop cost −34 bb/100, the c-bet response knob −4.3, the
   check lookahead was neutral, 16× decision samples gained nothing.
   *Rule*: every behaviour change passes a paired sim on identical cards before it ships, and
   ships as a learner knob (default off) when it does not clearly win.
2. **Unpaired sims cannot see real edges.** Unpaired tuning runs were noise, and the
   first champion/challenger loop could not detect +1..+5 bb/100.
   *Rule*: paired deals with luck removal, successive halving, and the sequential fresh-deal
   promotion gate. Never promote on the search survivor's own sample.
3. **Research estimates are hypotheses.** PGO, mimalloc, AVX2 MLP kernels, SIMD evaluation,
   thread affinity and alignment were all recommended and all measured as no gain.
   *Rule*: benchmark before adopting; speed-only changes must reproduce the guard
   `SIM_B='{"call_margin":0.005}' sim paired 12 600` exactly.
4. **Research agents report false positives.** The gaps audit listed "critical" crashes that were
   test-only code or already logged (gaps audit, verification addendum).
   *Rule*: the main session verifies every finding against the code before acting on it.
5. **More data is not better data.** 10,000 Pluribus hands made the neural model worse;
   legacy SvanBot hands helped only a cold start; reputation priors gave no held-out gain
  . *Rule*: a data source earns a consumer only by beating its baseline on held-out data.
6. **Optimise the thing that binds.** Decisions take 1 ms p50 against a 41 s hand pace; the learner was the CPU user and ~80% of it was recomputing board strengths.
   *Rule*: profile first; decision-path speed has no chip value, learner throughput does.

33. **Measure the loop you actually run, with the variant that could win.** One experiment tested thin LTO with
    16 codegen units on touch-rebuilds, found it slower, and adopted nothing; the gate kept relinking 28
    fat-LTO test executables (130–170 s per edit). No LTO with incremental builds and 256 codegen units
    rebuilt the same edit in 5 s with bit-identical results. *Rule*: before declaring "no gain",
    measure the real edit-test loop and the variants at the far end of the knob, not just the middle.
34. **A benchmark that drops its results cannot see what they hold.** The DEFLATE decoder ran at
    120 MB/s in its benchmark, but the store read decision details 3–10× slower: each value was
    decoded behind its 32 KB dictionary, which was then drained off the front, so every returned string
    kept 33 KB of capacity. The benchmark dropped each value at once and reused the memory; the store
    kept 68 MB of details alive in 2.3 GB of allocations. *Rule*: benchmark through the real caller,
    holding the results as it does, and assert `capacity == len` for values returned to be stored.

## Rewrites and releases

7. **Rewrites reset verified behaviour.** The clean-room rewrite that produced this tree accepted a strength
   regression and needed days of live fixes. *Rule*: modify in place behind
   characterization tests; no new-folder rewrites.
8. **Test before touching.** The decision core was untested when it was first split.
   *Rule*: a chip-moving module gets a characterization test before it changes.
9. **Never swap untested binaries in.** Building straight into `target/release` replaces the
   running fleet's executable. *Rule*: `scripts/release.sh` (stage build, tests, install, hot
   swap between turns).
10. **Long builds die in the background.** Background `release.sh` runs were killed by the
    session memory guard (live checkpoint 2026-09-15 18:26). *Rule*: build the stage, then
    release in the foreground.
11. **Browser tests rot silently.** The Playwright suite carried over from before the rewrite no longer matched the new dashboard.
    *Rule*: the dashboard suite runs against a sandboxed bot on every web change.

## Protocol and live play

12. **The spec moves.** A client written against one revision of the protocol drifted from the live one. *Rule*: re-fetch
    `https://docs.openpoker.ai/llms-full.txt` before protocol work.
13. **Resync is a first-class path, not an edge case.** Resynced hands lost their results
    and their pot geometry. *Rule*: every tracker feature is tested on a
    snapshot-only resync as well as a full hand.
14. **Frames repeat.** A duplicated turn frame made a bot act twice; a dead writer marked
    unsent actions as answered. *Rule*: each `turn_token` is answered at most once, and
    only after it was actually sent.
15. **All-in pots break naive EV.** Folds into pots with all-in players were valued as winning
    the whole pot; overshoves were called too wide. *Rule*: every EV change has an
    all-in and side-pot test.
16. **REST races the socket.** Rejoins raced auto-rebuy into HTTP 429. *Rule*: one
    scheduler owns buy-in timing.
17. **A panic must not end a bot.** A panic ended a bot's task for good; poisoned locks froze
    every decision (bug-hunt notes 1–2). *Rule*: per-bot supervisors, `parking_lot` locks,
    decisions on a guarded thread with a legal fallback.
18. **Server limits are documented.** Endless deep history retries hit the free export cap
   . *Rule*: read the limits in the spec before designing a retry loop.

## Data and operations

19. **Never advance a frontier on failure.** Backfill skipped pages that failed to store.
    *Rule*: `Result` all the way; progress markers move only after a successful write.
20. **Silent drops hide real faults.** Swallowed store errors and uncounted unparseable
    import rows. *Rule*: log every dropped error; count and alarm on unreadable input.
21. **Hard-coded constants go stale.** A hard-coded big blind and bot names.
    *Rule*: derive from table state or config.
22. **Backups can fill the disk.** 48 hourly + 30 daily copies would have filled the SSD within a
    season (bug-hunt notes 4). *Rule*: every writer of large files has a free-space guard and a
    retention limit; archives are verified before older ones are pruned. On 2026-09-24 the root disk still
    reached 100%: 24 hourly copies (15 GB) plus 26 never-pruned release snapshots (6.4 GB) left no room
    for a dev build. Size retention by the file size today, not at design time.
23. **Lazy statics plus rayon can deadlock.** Cold preflop tables hung the pool. *Rule*:
    no rayon inside lazy initialisers; use scoped threads.
24. **Dashboards must not lie.** Panels showed dead or placeholder data. *Rule*: a panel
    that has no live source is removed, not faked.
25. **External archives are read-only.** Legacy stores are irreplaceable and one is
    already corrupt; a read-write open can add WAL files or checkpoint into them. *Rule*: `mode=ro&immutable=1`.
26. **A stat fix is a behaviour change.** The cold 4-bet stat fix changed fold-to-3-bet
    and 4-bet features, and so decision EVs, but was committed without the full suite; the golden
    snapshot failed at the next release (2026-09-16, found by `release.sh`). *Rule*: run the
    anti-regression gate (`scripts/check.sh`) before every commit that touches
    `crates/`; regenerate the golden only after confirming the change is intended.
27. **JSON floats do not round-trip by default.** serde_json's fast float parser can land one ulp
    off, so a decision rebuilt from stored JSON differed from the live one (found building replay). *Rule*: `serde_json` runs with `float_roundtrip` (enabled in `sv10-bot`, unified
    across the workspace build); anything meant to reproduce exactly is tested through JSON.
28. **A gate with no approval path is an off switch.** `paired_poker_approved` was added as an
    exposure requirement for the neural response model, but nothing in production ever set it, so
    the fleet silently fell back to the stat model for a day while the dashboard said "stat
    fallback" and the learner logged "awaiting paired-poker approval" every cycle. *Rule*:
    a new gate ships together with the code that can pass it and a test that exercises the pass;
    a condition only ever satisfied in unit tests is a disabled feature.
29. **A cap that binds everywhere is not a safety margin, it is the model.** Self-calibration's
    flat ±3 bb cap was hit by 15 of 27 categories, hiding a −24 bb river-call error and applying
    corrections that the evidence did not support. *Rule*: limit a correction by the
    evidence behind it (a 95% haircut, sample shrink), and keep a flat cap only where extrapolating
    is unsafe — making an action *more* attractive than the spots it was measured in.
30. **A retry loop needs an end.** Four past seasons were re-requested hourly forever at a server
    timeout ceiling that never moved, storing nothing. *Rule*: every retry loop counts
    consecutive failures that made no progress, backs off, and gives up with one honest log line.
31. **A build outside the release path strands the next release.** A development build outside the
    release path was compiled straight into `target/release` (no commit identity). The fleet ran it
    for two days, and `release.sh` then correctly refused to overwrite what it could
    not identify, delaying the next release. *Rule*: only `release.sh` writes
    `target/release`; dev builds use `CARGO_TARGET_DIR=target/dev`. When it happens anyway, preserve
    the build with `RELEASE_ADOPT_UNIDENTIFIED=1`; never force past the check.
32. **A rule fixed in one place is still broken next door.** One fix made a failed store read keep the
    installed live fits, but the same watcher still turned a failed read of the neural or range key
    into "uninstall" for four more artifacts, and the dashboard rendered failed reads as empty panels
    (found by a whole-codebase review). *Rule*: when a robustness rule is fixed, move every
    site of that shape behind one module (`installs`, `jobs`, `api::store_read`) and test the rule
    once at that seam.
35. **Every process migrates at once.** A hot swap restarts the bot, learner and analyst together; each
    checked for a new column, two saw it missing, and the slower `ALTER TABLE` killed the learner
    (2026-09-26 06:55). *Rule*: schema changes go through `sv10_store::packed::ensure_column`,
    which treats a column a peer just added as success, and the busy timeout is set before the schema.

36. **A refit on a hand watermark loses its idle-time floor.** Splitting the learner's refits onto
    their own hand watermark silently deleted the hourly `refit_stale` that the wait loop
    ran, so on a quiet table the fold calibration and deep-pot call fit would have aged without
    bound — the watermark only advances when hands arrive. *Rule*: when work moves from "every loop
    iteration" onto a watermark, keep the cheap time-based version of it as a floor; the expensive
    part belongs to the watermark, the cheap part to the clock.
37. **A cache key you did not know about rebuilds everything.** Releases took 8–9 minutes after two changes
    had tuned the profile flags on cold builds. `--timings` on the real edit showed the rest: thin LTO
    re-optimized the whole program once per binary (eight of them), the release re-linted from a cold
    cache of its own, and the exported commit id (a tracked env var) recompiled the bot crate in lint and
    the tests too. Fixing those three took a one-file release to 39 s. *Rule*: time the pipeline
    you run with `cargo --timings` per stage before touching flags, and keep env vars that feed
    `env!`/`option_env!` out of every build that does not ship.
38. **A slow hold names the waiter, not the holder.** One investigation chased "some writer holds the database lock
    over ten seconds" through 309 warnings, and the worst of them was 14.2 s on `kv.rs:10` — a
    single-row UPSERT. A single-row write is not work; it is waiting for another process, and the site
    the alarm reports is the victim's own trivial call. The culprit was the hourly backup's ~2.8 GB of
    I/O on a shared, DRAM-less SSD, which appears nowhere in the stack. *Rule*: measure the wait apart
    from the work (a `busy_handler` that records whether the lock was ever found taken) before
    concluding anything about the site a duration alarm points at.
39. **A number measured on one population cannot be quoted for another.** One study's "turn raise costs
    0.458 bb/100" fell to 0.033 against a champion-matched control, and river raise's 0.216 to 0.042:
    the originals were graded on replay records that never carried the per-opponent corrections, so
    they were inflated 3–5x. The same trap sits in the audit's own populations — residuals cover every
    decision while the deep re-solve only queues pot ≥ 50 bb. *Rule*: an instrument names
    its population and its basis wherever it prints, and a number from a superseded input era is
    re-derived, never quoted.
40. **Your own instrument cannot grade the parameters it is fed.** `replay::audit` re-runs the deep
    search with the record's own parameters, `ev_bias` included, so it plays with the same mispriced
    EVs the live bot played with and structurally cannot see a pricing error — only post-pricing
    tactical disagreement. The rows still read as "priced off". *Rule*: before trusting a
    measurement of a parameter, check whether the instrument is given that parameter or re-derives it;
    a re-solve on recorded inputs grades the choice, not the price.
41. **A floor is only meaningful against the window that can supply it.** `GAP_MIN_DECISIONS = 500`
    inside an 8-day window did not filter the rare classes, it deleted them: the all-in family accrues
    ~44 verdicts a day pooled, so 500 was unreachable, and the biggest pots in the game lived in the
    one family the instrument could never speak about. *Rule*: when a threshold gates on a
    sample size, check the slowest class's accrual against the retention window, and measure on the
    longest window the store keeps rather than the shortest that is convenient.
42. **A limit is crossed by the batch, not by the change.** `crates/apps/bot/src/findings.rs` sat at 453
    lines, comfortably under the 500-line rule. Four tickets in one batch each added a few
    dozen lines to it; no single change came near the limit, and together they landed it at 687, which
    `scripts/check-file-size.sh` refuses outright — it is not grandfathered in
    `scripts/file-size-baseline.txt`, and `--baseline` is an operator decision precisely so that a file
    born oversized cannot be blessed into it. The same batch was the first to move the dependency set
    since the notices were generated, so `docs/THIRD-PARTY-NOTICES.md` went stale under it, and a rustfmt
    drift sat in a file (`learner/search/tests.rs`) that no one had reason to look at again.
    *Rule*: before the full gate, run its cheap checks over the whole batch —
    `scripts/check-file-size.sh`, `cargo fmt --all --check`, `scripts/notices.py --check`,
    `cargo deny check` — each is seconds against the gate's minutes, and each fails on the sum of the
    batch rather than on any one change, so no individual ticket's author ever sees it coming.
43. **A version stated in one place is stated in half of them.** The 10.0.1 release bumped sixteen
    workspace members in `Cargo.toml` and left `Cargo.lock` at 10.0.0, and nothing said so: the gate
    ran `cargo` steps that quietly rewrote the lock in the working tree and never compared it to
    anything. Every build after that dirtied the tree, so every agent had to decide whether the
    `Cargo.lock` diff was its own change or the repository's, and a `git checkout -- Cargo.lock`
    before each commit became a habit nothing enforced. The lock is a second copy of a fact the
    manifest already states, and a second copy drifts where no check reads it.
    *Rule*: a fact stated in two files is checked in the gate — `cargo metadata --locked` compares
    the lock to the manifests in under a second, and fails on a version bump or a dependency alike.

<!-- docs-check: off -->
The entry below describes `scripts/promote.sh` and its tests, deleted on 2026-09-28 when the
repository moved to trunk-based work on `main`. The lesson is about the mistake, not the script.

44. **A fixture that models the wrong shape passes on the bug it exists to catch.** `scripts/promote.sh`
    refused a promotion when `main` was not an ancestor of `dev`. A promotion *is* a merge commit of
    `dev` into `main`, and a merge commit lives on `main` and never on `dev` — so `main` stops being an
    ancestor of `dev` the moment it has been promoted once, and the guard refused every promotion after
    the first. The first real run refused `main` at the merge commit of the promotion that had just
    succeeded. Seven test cases were green on it: every one left `main` fast-forwardable to `dev`, so
    none of them ever built the shape a promotion leaves, and the suite graded the bug as the
    specification. *Rule*: a case that exists to prove a state is accepted asserts the fixture reached
    that state before it asserts what the code does with it — here, that `main` is *not* an ancestor of
    `dev`, which is what makes it the state under test rather than another caught-up one. The same
    file failed the same way from the other side: its stub `gh` answered `pr create` with a pull
    request number for *any* request, where GitHub refuses one whose head has no commit the base
    lacks. A double more capable than the thing it stands in for cannot fail the code under test, so
    the suite could not tell a script that opens a valid promotion pull request from one that asks
    for a pull request GitHub will reject — and it caught #381 only because an assertion happened to
    look at the call instead of at the answer.
<!-- docs-check: on -->

45. **An identity comparison cannot answer a question about content.** The same script decided whether
    `main` was current with `[ "$main" = "$dev" ]`, the two tips' commit ids. A promotion is a merge
    commit of `dev` into `main`, so from the first promotion onward `main`'s tip is a commit `dev`'s
    tip never equals, however identical the two trees are — and the early return was dead from the day
    it could matter. Every run that found `main` already holding what `dev` held went on to open a
    promotion pull request listing nothing: `printf '%s\n' ""` is one empty line, so `wc -l` read the
    empty list as `1 commit(s)`. A `check` cycle, a release run and a review cycle went on a diff that
    changed no file, after every promotion that had already merged. *Rule*: name the state you are
    testing by the thing you actually mean — "these two hold the same thing" is a comparison of trees
    (`git diff --quiet`), not of ids — and count a list by what is in it (`grep -c .`), not by the
    lines a `printf` chose to write. The same script wears the mistake a second way: its guard against
    a `main` carrying work of its own asks whether `main` has a *non-merge* commit `dev` lacks, which
    is commit shape standing in for content. A conflict resolved on `main` is carried by a merge
    commit, so it passes that guard while holding a file `dev` never had — and the script went on to
    ask GitHub for a pull request GitHub refuses. Ask the question you mean, of the thing that holds
    the answer: containment of *content* is `git rev-list --count dev --not main` being zero.

46. **A wait charged to the thing that waited reads as slowness in it.** The 2026-09-28 05:02 release
    reported a 498 s build stage, four times the budget, and the explanation written down was a cold
    cache. It was not one: every crate in `target/stage` had compiled at 03:10, and cargo invoked *no
    crate at all* across those eight minutes — a build that compiles nothing is not a slow build, it
    is not a build. It was cargo's build lock. Anything else building in the same target directory
    holds `release/.cargo-lock`, and the second cargo waits with one line, `Blocking waiting for file
    lock on build directory`, for as long as the holder runs; a `--profile release` build elsewhere in
    the tree was finishing as the release started. The line that named it was in
    `target/stage/release-build.log`, which the next release truncates — so the number outlived its
    reason, and the release after it, warm and unblocked, built in 20.6 s. *Rule*: the seconds spent
    in a stage are charged to that stage, so a stage has to be able to name a wait — probe the build
    lock non-blockingly and say it is held before waiting on it (`scripts/build-lock.sh`) — and keep
    the log of a stage that went over budget, because the next run truncates the only record of why
    it did.

47. **A file that has to be hand-edited after every install is wrong on every machine but one.** The
    systemd units shipped in `scripts/` named `/srv/svanbot10` — the reference machine's checkout —
    in `WorkingDirectory`, `ExecStart`, `ExecStop` and `EnvironmentFile`. Nobody's install is there,
    so the recipe in the runbook was `cp` followed by an edit, and the installed copies in
    `~/.config/systemd/user/` were a fork of the tree with `%h/svanbot10` in them. A fork is what it
    behaved like: a `git pull` that changed a unit changed nothing the machine would run, because the
    file systemd reads had been edited away from the file git tracks, and neither side could tell.
    Relocating a running deployment was blocked on it for the same reason — the units named the old
    directory and nothing said so. *Rule*: ship the template, not one machine's copy. A path that
    depends on where the software was installed is filled in at install time, from the directory the
    installer is run from (`scripts/units.sh`), and the render is re-runnable and comparable, so
    "are the installed units the ones this checkout would write?" is a question with an answer
    (`--check`, reported by `scripts/status.sh`) rather than a diff nobody takes. The rendering
    itself is where the second trap is: a path is arbitrary bytes, and every substitution operator
    within reach reads some of them as syntax — `&` and `\1` in `sed` and `awk`, `&` in bash's own
    `${v//pat/rep}` since 5.2 — so `/home/a & b/SvanBot` substituted with any of them puts the
    placeholder back into the unit. Substitute literally, and escape into the target language's
    rules on the way out (`%` is a systemd specifier and has to be doubled; a path with a space needs
    the `Exec` line quoted). `scripts/tests/units.sh` renders a checkout under `od d %25 r & p` and
    asks real systemd what it resolves to, because every one of those characters is legal in a home
    directory.

# Research #787: Why target/release lacks monitor while stage has it

Facts only, no fix. Part of #786.

## Answer in one paragraph

`crates/apps/bot/src/bin/monitor.rs` was introduced in `7963b44`
(2026-10-03 20:28 UTC, "scripts: one implementation for the results
monitor and the fleet check", Closes #717), about 8h AFTER the
installed commit `0af80f0` (merged 2026-10-03 12:38 UTC, PR #697).
`target/release/.sv10-installed-commit` still reads `0af80f0`, so the
installed bundle predates the monitor binary and cannot contain it.
The `0af80f0` release snapshot likewise has no monitor (13 binaries,
no `target/release/monitor`, no SHA256SUMS entry). A current build did
compile monitor into `target/stage/release/monitor` (2026-10-04 01:17,
for commit `f4375fe`), but the release gate failed in lint before
reaching the install stage, so the stage tree was never installed.

## Evidence

- Intro commit: `git log --diff-filter=A --format="%H %ci %s" --
  crates/apps/bot/src/bin/monitor.rs` gives only `7963b44 ... 2026-10-03
  20:28:11 +0000 scripts: one implementation for the results monitor and
  the fleet check`. Stat (`git show --stat 7963b44`) shows new
  `crates/apps/bot/src/bin/monitor.rs` (27 lines) + `crates/apps/bot/src/monitor.rs`
  (431 lines) + `[[bin]] name = "monitor"` in `crates/apps/bot/Cargo.toml:31-34`.
  `git merge-base --is-ancestor 0af80f0 HEAD` is true; `0af80f0`
  (2026-10-03 12:38:11) predates `7963b44` (2026-10-03 20:28:11) by ~8h.
- Installed commit: `target/release/.sv10-installed-commit` contains
  `0af80f0`. `scripts/rollback.sh --installed-commit` resolves it.
- Snapshot `artifacts/release-snapshots/0af80f0/` (created 2026-10-04
  01:16): `find ... -name "*monitor*"` returns nothing;
  `target/release/` holds analyst, archive, bench, bench6, calibrate,
  ingest, learner, neural_ab, probe, review, sim, sv10-bot, tables
  (binaries dated 2026-10-03 12:22/12:50, i.e. the `0af80f0` era) plus
  `.sv10-installed-commit`; `SHA256SUMS` lists those 13 binaries + web
  assets and has no monitor entry.
- Stage vs installed: `target/stage/release/monitor` exists
  (4877080 bytes, 2026-10-04 01:17); `target/release/monitor` does not
  exist. `artifacts/release.log` head reads "== checking and building
  f4375fe (lint, tests and release in parallel)" and
  `target/stage/release/` holds freshly built (01:17) analyst, archive,
  calibrate, ingest, learner, monitor, neural_ab, sv10-bot, etc.
- Why never installed: `scripts/release.sh:127-137` starts lint,
  test-build, release-build in parallel; `scripts/release.sh:138-156`
  waits on all three and `exit 1` on any failure. The run failed at
  lint: `artifacts/release.log` tail shows "== lint failed (log:
  target/stage/lint.log)" whose body (`target/stage/lint.log`) ends in
  the keepalive failure "keepalive test: a running supervisor was
  treated as down / check.sh: keepalive tests". The install stage at
  `scripts/release.sh:171-173` (`scripts/rollback.sh --install
  "$STAGE/release" "$WEB_STAGE" "$commit"`) was therefore never
  reached; the `--version` pre-install check at
  `scripts/release.sh:163-165` was likewise never reached.
- Verifier/copy split (why the gate, not the installer, is the cause):
  `scripts/rollback.sh:369-373` `require_binaries()` verifies only
  `sv10-bot learner analyst` (with `--version` + commit identity;
  `strict` at the `install_release` call site). `create_snapshot()` at
  `scripts/rollback.sh:450-455,471` and `install_release()` at
  `scripts/rollback.sh:574-596` both copy with `find <src> -maxdepth 1
  -type f -perm /111 ! -name '.*'` — i.e. ALL executables, so a
  completed install WOULD carry monitor. `swap_install()` at
  `scripts/rollback.sh:511` only renames directory sets. So the
  installer is monitor-agnostic; the installed tree lacks monitor only
  because no install has completed since `7963b44`.
- Who expects monitor now: `scripts/start.sh:34-36` requires
  `sv10-bot tables monitor` (+ learner/analyst at `:37-38`) and diverts
  to `scripts/release.sh` at `:40-43` on any miss; `scripts/install.sh:59`
  and `scripts/portable.sh:26` already list monitor in their copy loops;
  `scripts/supervisors.sh:75` launches `./target/release/monitor`.
  Detail of the gate failure itself belongs to #788.

Generated-by: opencode/muse-spark-1.3-contributor-free

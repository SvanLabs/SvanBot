# #788 research: why the release gate fails (lint + keepalive)

Question: why does the release gate fail, blocking `scripts/release.sh` and therefore `scripts/start.sh`?
Answer: the only failing step is the keepalive tool test inside the `check.sh lint` job,
and it fails only when `SVANBOT_FLEET=split` is exported in the invoking environment.
On a clean tree with a default environment the test passes. No code fix was made here.

## Exact failing check

`scripts/check.sh:163` (inside the `full|deep|lint` tool-test sequence):

```sh
run_suite "keepalive tests (run bash scripts/tests/keepalive.sh)" bash scripts/tests/keepalive.sh
```

`scripts/release.sh:127` runs that gate as the parallel `lint` job
(`start_job lint "$DEV" no-commit scripts/check.sh lint`, with `CARGO_TARGET_DIR=target/dev`),
so the failure surfaces as `== lint failed (log: target/stage/lint.log)` and the release
exits at `scripts/release.sh:156` before the `== testing` stage. Later lint suites
(partial-fleet, update, adopt-upstream, file-size, build-lock, patch-inbox, clean, units)
never ran. Nothing in `cargo-deny`, `rustfmt`, `clippy`, `docs-check`, or the ticket/codec
tool tests failed.

## Full failure text (nothing beyond it exists)

`target/stage/lint.log` in full (17 lines, 2026-10-04 01:19 run):

```text
== placeholder markers
== ai provenance
provenance: 37 commit(s) in HEAD name their system
provenance: AI-PROVENANCE.md lists the 8 system(s) in HEAD
== rustfmt
== Cargo.lock matches the manifests
== file sizes (docs/CONTRIBUTING.md: 500-line limit, baseline may only shrink)
== clippy -D warnings
== cargo-deny (licenses, bans, sources)
== cargo-deny advisories
advisories ok
== third-party notices current
== tickets lint + tool tests (tickets, codec vs zlib, test runner, release/rollback, keepalive, update, adopt-upstream, file-size, build lock, systemd units)
   (tickets lint + tool tests (tickets, codec vs zlib, test runner, release/rollback, keepalive, update, adopt-upstream, file-size, build lock, systemd units): 7 s)
== docs name only paths and commands that exist
keepalive test: a running supervisor was treated as down
check.sh: keepalive tests (run bash scripts/tests/keepalive.sh)
```

`target/dev/check-suite.log` (the `run_suite` capture, `CARGO_TARGET_DIR=target/dev`):

```text
keepalive test: a running supervisor was treated as down
```

`artifacts/release.log` in full (26 lines; the `tail -30` at `scripts/release.sh:153`
is why the dashboard shows only this):

```text
== free space
463629 MB free on /home/administrator/SvanBot (>= 4096 MB)
== snapshotting installed release 0af80f0
Verified release snapshot 0af80f0
Release snapshot 0af80f0 already exists; keeping it unchanged
== checking and building f4375fe (lint, tests and release in parallel)
== lint failed (log: target/stage/lint.log)
== placeholder markers
== ai provenance
provenance: 37 commit(s) in HEAD name their system
provenance: AI-PROVENANCE.md lists the 8 system(s) in HEAD
== rustfmt
== Cargo.lock matches the manifests
== file sizes (docs/CONTRIBUTING.md: 500-line limit, baseline may only shrink)
== clippy -D warnings
== cargo-deny (licenses, bans, sources)
== cargo-deny advisories
advisories ok
== third-party notices current
== tickets lint + tool tests (tickets, codec vs zlib, test runner, release/rollback, keepalive, update, adopt-upstream, file-size, build lock, systemd units)
   (tickets lint + tool tests (tickets, codec vs zlib, test runner, release/rollback, keepalive, update, adopt-upstream, file-size, build lock, systemd units): 7 s)
== docs name only paths and commands that exist
keepalive test: a running supervisor was treated as down
check.sh: keepalive tests (run bash scripts/tests/keepalive.sh)
   test-build ok (1 s)
   release-build ok (1 s)
```

## Narrowed case

The failing assertion is the sleeper-pid fleet-up case, `scripts/tests/keepalive.sh:18-22`:
it backgrounds `sleep 60`, writes that pid to the five all-layout pid files
(`supervisor.pid`, `learner-supervisor.pid`, `analyst-supervisor.pid`,
`monitor-supervisor.pid`, `logrotate.pid`) in a throwaway root, and expects
`KEEPALIVE_DRY=1 scripts/keepalive.sh` to print `fleet up`.

`pid_alive` (`scripts/supervisors.sh:3-10`, file-exists plus numeric plus `kill -0`
plus non-zombie `ps -o stat=` check) is not broken: the sleeper is alive and passes it.
The mismatch is the expected set. `scripts/keepalive.sh:15-19` preserves an inherited
`SVANBOT_FLEET` over `.env` (and the throwaway root has no `.env` at all), then
`scripts/keepalive.sh:51-53` builds the expected list via `expected_supervisors()`
(`scripts/supervisors.sh:23-35`), which branches on `${SVANBOT_FLEET:-all}`.
With `SVANBOT_FLEET=split` it expects `head-supervisor.pid` plus per-worker pid files
instead of `supervisor.pid`, so none of the files the test wrote match; the all-layout
`supervisor.pid` is still alive so `fleet_present=1` (`scripts/keepalive.sh:55-57`),
and the decision is `fleet partial, repairing` (`scripts/keepalive.sh:64-67`),
not `fleet up`, which trips the `fail` at test line 22.

## Reproduction matrix (investigated tree, HEAD f4375fe, `git status` clean)

- `bash scripts/tests/keepalive.sh` with a default environment: passes
  (`keepalive: ok`, `start supervisor detection: ok`, exit 0).
- `SVANBOT_FLEET=split bash scripts/tests/keepalive.sh`: fails byte-identical to the gate
  (`keepalive test: a running supervisor was treated as down`, exit 1).
- `LEARNER=0 bash scripts/tests/keepalive.sh`: passes (dropping learner/analyst files
  from the expected set cannot break this case; only the fleet-layout switch does).

So the failure reproduces from clean caches only when the split layout leaks in through
the environment; it is environment-dependent, not cache-dependent. The `target/stage`
vs `target/dev` split is irrelevant here (the lint job runs with
`CARGO_TARGET_DIR=target/dev`; `target/stage/lint.log` is just release.sh's capture).

## Pointers on how the environment leaked in

- Neither `scripts/release.sh`, `scripts/check.sh`, nor `scripts/resources.sh` sources `.env`
  or exports `SVANBOT_FLEET`; `run_suite` children cannot export back into the `check.sh`
  shell. The value must have been exported in the environment that invoked the 01:19
  release (operator shell with `set -a; source .env`, an inline `SVANBOT_FLEET=split`
  prefix, or a dashboard-triggered release inheriting fleet variables).
- Before releasing, `export -p | grep -E 'SVANBOT_FLEET|LEARNER|ANALYST|OPENPOKER'` shows
  what the keepalive tool test will inherit.
- Corroborating context (not proof): the checkout's gitignored `.env` carries
  `SVANBOT_FLEET=split` plus four extra bot keys/names (`OPENPOKER_API_KEY_2..5`,
  `BOT_2_NAME..BOT_5_NAME`) and was modified 01:18, one minute before the failing
  01:19 release. Extra bot keys alone do not break this case in `all` mode
  (`fleet_names()` in `scripts/supervisors.sh:14-19` is only consulted for split);
  the exported `SVANBOT_FLEET=split` is the load-bearing variable.
- Related second half of the same gate output: `start.sh`'s supervisor detection
  (`scripts/tests/keepalive.sh:78-94`) already covers both layouts, so the start path
  the map ticket (#786) cares about is unaffected by this finding.

Generated-by: opencode/muse-spark-1.3-contributor-free

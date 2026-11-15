# 06 — Build, CI and tooling (SvanBot v11.0.0 research)

Read-only survey, 2026-11-15. No builds were run. The timings come from `gh run view` and the job-log API: check run 43720582988 (push to main, 5m44s), release run 43712748389 (PR image-only, 8m01s) and the last tag release 36593874507 (bundle 7m39s, image 9m36s).

## 1. What we have

- **Toolchain:** pinned `1.98.1`, minimal profile plus clippy and rustfmt. `.cargo/config.toml` sets `target-cpu=native`. CI sets `RUSTFLAGS=""`, and the portable build and the Dockerfile override it too.
- **Profiles:**
  - `release`: opt 3, no LTO, `codegen-units=256`, `incremental=true`, `debug=0`.
  - `profiling`: line-tables-only.
  - `dev`: opt 1, with dependencies at opt 3.
  - `test`: opt 3.
  - `gate`: inherits `release`, with `sv10-bot` at opt 1.

  So every build that tests use is a fully optimised build.
- **Gate (`scripts/check.sh`, 217 lines):** modes `commit`, `full`, `deep` and `lint`. In `full`, a background `cargo test --no-run --profile gate --workspace` starts at the top. The checks then run in series:
  - rustfmt, the `--locked` metadata check, file sizes, clippy `--release` and cargo-deny (licenses, bans, sources, then advisories)
  - notices, the tickets lint, 18 Python unittest files and docs-check
  - 14 bash suites, then `scripts/test.py`, a custom parallel test runner
  - tsc

  Every step is timed against a 120 s budget.
- **CI (`check.yml`):** three jobs: `web` (13 s), `gate`, and an aggregate `check`. `gate` uses Swatinem/rust-cache v2.9.3 over `target/dev` and `artifacts/tables`.
- **Release (`release.yml`):** on a `v*` tag, the `bundle` job runs `portable.sh`, which does three sequential cold `cargo build --release` runs (v1, v2, v3) with no cache. The `image` job builds the Dockerfile with a fourth cold build, using a GHA layer cache with `mode=max`.
- **cargo-deny:** permissive licenses, crates.io only, wildcards denied, duplicate versions only warn.
- **Not installed locally:** nextest, sccache, mold, wild, clang, lld, cargo-sweep and hakari. The machine has 6 cores. `target/dev` is **14 GB** on a disk that is 97% full (3.4 GB free).

## 2. Step-level time breakdown

### `check` workflow, `gate` job: 5m37s of the 5m44s wall time

| Step | Time | Note |
|---|---|---|
| Runner set up, checkout (full history), setup-node (npm cache hit) | 6 s | |
| `rustup show` (toolchain) | 8 s | |
| rust-cache restore | 5 s | 289 MB, key `v0-rust-gate-…` full match. The post-job step reported "Cache up-to-date", so the cache is never refreshed. |
| `cloud-setup.sh` | 3 s | Tables came from the cache; cargo-deny was already present. |
| placeholder, provenance, rustfmt, lock, file-size | 3 s | |
| clippy `-D warnings` | 20 s | It shares a cargo lock with the background test build. |
| cargo-deny (two runs) | 3 s | |
| tickets lint and the Python unittests | 9 s | |
| **"docs name only paths…" (263 s)** | **263 s** | **Mislabelled.** `docs-check.py` itself is fast. This step label also covers the 14 bash suites. The first of them, `release-rollback.sh`, sources `lib-release-bin.sh`, which runs `cargo build --profile gate -p sv10-release`. That build **blocks on the target-dir lock until the background workspace test build finishes**. These 263 s are really the cold build of the workspace crates at opt-level 3. |
| workspace tests | 14 s | 939 tests in 35 binaries; build 0.2 s, run 12.3 s. |
| tsc | 1 s | |
| Post steps (cache already up to date) | 3 s | |

**Root cause:** rust-cache caches only *dependencies* by default, and it sets `CARGO_INCREMENTAL=0` ([rust-cache README](https://github.com/Swatinem/rust-cache)). Every run therefore rebuilds all 18 workspace crates from scratch at opt 3 on a 4-vCPU runner, which takes about 4.5 minutes. The test run itself takes 14 s. About **85% of CI time is compiling our own crates.**

### `release` workflow, PR image-only run: 8m01s

| Step | Time |
|---|---|
| Checkout, buildx boot, base image pulls, apt layers | 40 s |
| `cargo build --release --workspace --bins` (cold; `COPY . .` invalidates every layer) | 175 s |
| **Export of the GHA cache, `mode=max`** | **235 s**: 66 s preparing, then multi-GB layers written, including the build stage's whole `target/` |
| Cleanup | 15 s |

On a pull request the image is never pushed, so the 235 s export buys nothing. The next run misses the cache again at `COPY . .` anyway ([docker GHA cache docs](https://docs.docker.com/build/cache/backends/gha/)).

### Tag release (last one: 2026-09-29)

- **bundle:** npm install 4 s, then v2 at 217 s and v3 at 212 s, run in series. v1 has since been added, so it is now about 3 × 215 s ≈ **11 min**.
- **image:** 9.6 min, in parallel with the bundle.
- Neither job reuses the other's binaries or CI's.

## 3. 2026 practice, applied to this repo

- **Linker.** rust-lld has been the default on x86_64-unknown-linux-gnu since 1.90 ([Rust blog](https://blog.rust-lang.org/2025/09/01/rust-lld-on-1.90.0-stable)), so we already link fast. [mold](https://github.com/rui314/mold) or [wild](https://github.com/davidlattimore/wild) would save seconds per binary on 11 binaries. That is low value; measure it before adopting.
- **Cranelift and the parallel frontend.** Both are still nightly-only, and both are 2026 project goals rather than stable features ([cg_clif goal](https://rust-lang.github.io/rust-project-goals/2026/improve-cg_clif-performance.html), [parallel front end goal](https://rust-lang.github.io/rust-project-goals/2026/parallel-front-end.html)). **Reject both for v11:** the toolchain is pinned to stable. Cranelift would also break the rule that test builds are the optimised builds the golden snapshot proves.
- **Debuginfo.** Release already uses `debug=0` and profiling uses line-tables-only, which is the [recommended split](https://nnethercote.github.io/perf-book/build-configuration.html). `split-debuginfo` only matters if debuginfo is turned on. Nothing to gain here.
- **Profile tuning.** This is the biggest lever we control. `test` and `gate` compile *every* workspace crate at opt 3. Hot crates (`cards`, `equity`, `engine`, `nn`) would stay at opt 3; glue crates (`store`, `venue`, `rt`, `bot`, `release`) would go to opt 1, the same treatment `sv10-bot` already gets. The golden snapshot proves the results are bit-identical, so this is safe to try under `/measure`.
- **Caching.** Three options:
  - Set `cache-workspace-crates: true` and `save-if: github.ref == 'refs/heads/main'`, and add the commit or a date to the key so main refreshes the cache. As it stands, the key changes only when `Cargo.lock` changes, and post-job reports "Cache up-to-date".
  - [sccache](https://github.com/mozilla/sccache) with the GHA backend caches per crate and does not need incremental. It is the alternative when a PR touches a crate near the root.
  - Fallback: build the gate's test binaries on push to main and restore them on PRs.
- **Incremental.** `incremental=true` in the profile is right at the desk and wrong in CI; rust-cache already overrides it, which is correct.
- **Test runner.** [cargo-nextest](https://nexte.st/) gives process-per-test isolation, retries, JUnit output and `--partition`. The run takes 12 s, so the speed gain is small. Its value is replacing `test.py`, which is 7.5 KB of Python, and its custom RSS scheduler. nextest has `test-threads` and `threads-required` for that.
- **workspace-hack / [cargo-hakari](https://docs.rs/cargo-hakari).** Hakari pays off when `-p` builds unify features differently from `--workspace` builds. Here the `-p sv10-release` build (the wrapper tests) and the `-p sv10-core` build (the commit tier) can rebuild dependencies with different feature sets. The gain is medium-low. Measure it first, using `cargo build -p X` after a `--workspace` build and counting rebuilt dependencies.
- **Keeping `target/` small.** Rust 1.91 stabilised `build.build-dir` ([releases.rs 1.91](https://releases.rs/docs/1.91.0/)), so intermediates can live on another disk or tmpfs while final binaries stay in `target/`. [cargo-sweep](https://github.com/holmgr/cargo-sweep) can run `--toolchains`/`--time` from the 6-hourly clean timer, or `prune-builds.py` can be extended. 14 GB of `target/dev` on a 97%-full disk is a live risk.
- **Gate design.** The tiers are already right: commit is path-gated, full matches CI, deep adds 100k property cases. Two improvements:
  - (a) Split the CI `gate` job into `lint` (fmt, clippy, deny, notices, the Python and bash suites, docs) and `test` (the build and the tests). They run in parallel and stop fighting over one cargo lock.
  - (b) Commit-tier change-based selection: map the staged paths to crates, use `cargo metadata` to find the reverse dependencies, and run `-p` over that set. The full tier stays exhaustive.
- **xtask.** Use the [cargo-xtask](https://github.com/matklad/cargo-xtask) pattern: a `crates/xtask` binary with a `.cargo/config.toml` alias `xtask = "run -p xtask --profile dev --"`. It replaces the shell and Python dev tooling with tested Rust. **Ops scripts that run on toolchain-less installs must not become xtask.** Those belong in the existing `sv10-release` binary, or stay as shell.
- **Release pipeline.**
  - Build the v1, v2 and v3 bundles as a **matrix** of parallel jobs, then assemble them in a fan-in job.
  - Build the image `FROM debian` by copying in the bundle's v1 binaries (`needs: bundle`) instead of compiling a fourth time.
  - Drop `cache-to` on PRs, or use `mode=min`.
  - [cargo-dist](https://opensource.axo.dev/cargo-dist/) generates the matrix, checksums and installers. It does not model three ISA levels in one tarball, our attestation, or our `install.sh` CPU detection, so adopt it only if the bundle format changes. Otherwise hand-roll the matrix.

## 4. Scripts: keep / merge / delete for the xtask migration

`scripts/` has 57 entries: 54 files and 3 directories.

| Entry | Purpose | v11 action |
|---|---|---|
| check.sh | the gate (commit, full, deep, lint) | **xtask `gate`** (keep a 3-line shim) |
| pre-commit.sh | hook that calls check.sh commit | keep as a shim that runs `cargo xtask gate commit` |
| pre-commit-secret-scan.sh | scans staged changes for `.env` secrets | merge → xtask gate |
| check-file-size.sh | 500-line limit | merge → xtask gate |
| file-size-baseline.txt | data for the file-size check | keep (data) |
| docs-check.py | docs drift | merge → xtask gate |
| docs-check.live | data: the documents docs-check covers | keep (data) |
| provenance.py | `Generated-by` check and roster | merge → xtask `provenance` |
| notices.py | third-party notices and SBOM | merge → xtask `notices` |
| test.py | parallel test runner | **delete** → nextest |
| build-lock.sh | is cargo holding the target lock? | merge → xtask (library function) |
| build-ab.py | profile-knob A/B timing study | merge → xtask `measure` or move to `studies/` |
| clean.sh | cleanliness report and apply | merge → xtask `clean` (with cargo-sweep) |
| prune-builds.py | prunes superseded artifacts | merge → xtask `clean` |
| reap-worktrees.sh | reports dead worktrees | merge → xtask `clean` |
| vendor.sh | offline vendoring | merge → xtask `vendor` |
| portable.sh | 3-ISA bundle | **xtask `dist`** (CI matrix calls it per level) |
| cloud-setup.sh | CI and agent bootstrap | keep as shell (it must work before cargo can) and trim it |
| setup.sh | first-run setup on a new machine | keep as shell |
| install.sh | installs a bundle on a target with no Rust | keep as shell (it runs without a toolchain) |
| release.sh / update.sh / rollback.sh | wrappers around sv10-release | keep |
| sv10-release-bin.sh | builds or locates sv10-release | keep and fold into the three wrappers |
| adopt-upstream.sh | moves a fleet checkout onto this repo | move into `sv10-release adopt` |
| fetch-data.sh | restores runtime data from a release | move into `sv10-release fetch-data` |
| start.sh / stop.sh / restart-bot.sh / status.sh / supervisors.sh / keepalive.sh | fleet lifecycle | keep for v11.0; candidates for `sv10-release fleet …` later |
| season-check.sh | season-boundary check | keep (ops) |
| container-start.sh | container entrypoint | keep |
| resources.sh | sourced job-limit environment | keep (sourced by every build) |
| host_resources.py | CPU, cgroup, RAM and disk budgets | merge → `sv10-rt` or xtask (8 references; port with care) |
| resource-report.py | disk and memory report | merge → xtask `report` |
| progress.py | dashboard release progress | keep, or port into sv10-bot (9 references) |
| monitor.py | one-release shim (#717) | **delete** |
| tickets.py | local-markdown tracker | **delete**; the tracker is GitHub issues (3 references to update) |
| claims.py | one-claim rule for the tracker | merge → xtask `claim` or delete (1 reference) |
| close-linked-issues.py | run by the workflow | keep (CI-only), or replace with a GitHub-native close |
| local-review.py | local agent review | keep (agent workflow) |
| patch-inbox.sh | ship-patch inbox | keep (fleet protocol) |
| units.sh | renders and installs systemd units | keep; absorb archive-timer.sh |
| archive-timer.sh | installs the archive and clean timers | merge → units.sh |
| 7 `svanbot10*.service` / `.timer` files | unit templates | keep (data) |
| web-test-server.sh | Playwright sandbox server | keep |
| tests/ (14 bash and 18 Python suites) | tool tests | port to Rust tests next to each migrated tool; delete each one as its tool moves |
| studies/ | evidence analysis (poker.py) | keep |
| \_\_pycache\_\_/ | bytecode (gitignored) | delete locally |

**Net result:** 16 entries fold into xtask; test.py, monitor.py and tickets.py are deleted; about 5 move into sv10-release; the rest stay as shell, systemd templates or data.

## 5. Ranked speedups

| # | Change | Estimated saving | Effort |
|---|---|---|---|
| 1 | **Cache the workspace crates in CI:** `cache-workspace-crates: true`, save on main only, include the commit in the key (or use sccache with GHA) | CI gate 5.6 → **~2–2.5 min** (most PRs rebuild a few leaf crates) | S |
| 2 | **Image job:** copy the bundle's binaries instead of recompiling; no `cache-to` on PRs | image 8 → **~1.5 min**; PR check 8 → ~1 min | S |
| 3 | **Matrix the bundle** over v1, v2 and v3 | tag bundle ~11 → **~4 min** | S |
| 4 | **Opt 1 for glue crates** in `gate` and `test` (golden-proven, `/measure`) | 60–120 s off every cold test build, locally and in CI | S–M |
| 5 | **Split CI into lint and test jobs**, and give the wrapper tests a prebuilt `SV10_RELEASE_BIN` so they stop waiting on the cargo lock | 20–40 s of wall time; red lint results arrive in about 1 min | S |
| 6 | **Disk:** `build-dir` off the root disk, cargo-sweep on the clean timer | frees GBs of the 14 GB; protects a disk at 97% | S |
| 7 | **Commit-tier reverse-dependency test selection** | commit hook 30–90 s → seconds when a leaf crate changes | M |
| 8 | **xtask migration**, nextest replacing test.py | maintenance, not speed (about 5 s); deletes about 3 kLOC of shell and Python | L |
| 9 | Hakari workspace-hack | 10–30 s on `-p` builds; measure first | M |
| 10 | mold or wild instead of rust-lld | under 10 s; skip unless measured | S |
| — | Cranelift, parallel frontend | not on stable; rejected | — |

Items 1–3 together bring CI from about 5.6 to about 2.5 minutes, and a tag release from about 11 to about 4.5 minutes, without touching the gate's semantics.

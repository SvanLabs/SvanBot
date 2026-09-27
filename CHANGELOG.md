# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Nothing yet. Merged changes land here and move under a version when a release is tagged
(`docs/RELEASE.md`).

## [10.0.0] - 2026-09-27

The first public release; `10.0.0` is the version in the workspace `Cargo.toml`. There are no
`Changed`, `Fixed` or `Removed` sections, because there is no earlier public version to change,
fix or remove.

### Added

- **The fleet.** Up to five portfolio bots on [openpoker.ai](https://openpoker.ai) from one
  machine — 6-max no-limit hold'em, virtual chips, 14-day seasons. One process runs a task per bot:
  connect, seat, track the table, decide, act. A hand in progress when the process exits is saved
  and settled by the next one from the resync replay, with its real net. A split layout (head
  process plus one worker per bot) is available and off by default.
- **An exploitative decision path.** Every legal action — fold, check/call, several raise sizes —
  is priced by expected chips against the range each opponent has actually shown, using a 7-card
  evaluator verified over the whole space, shared board-strength tables, and exact heads-up
  enumeration where it fits (the river always, the turn and flop live) instead of sampling. Monte
  Carlo deals are seeded and split across cores. A legal fallback is prepared before every search,
  and the search is capped at 8 s against the turn clock.
- **Opponent modeling that has to beat its baseline.** Recency-weighted per-player statistics; a
  range model fitted to showdowns and refitted on a schedule; a small neural response model
  (38 → 48 → 24 → 3) used only while it beats the hand-built statistical model on the most recent
  hands; and per-opponent fold, response and river-sizing fits installed only while they predict
  that opponent's newer hands better than the shared model.
- **Self-calibration.** The bot measures its own bias per spot category and corrects it, bounded by
  the residual supported at the decision margin rather than by a flat cap.
- **The autonomous learner.** A separate low-priority process that refits the models, retrains the
  network, and searches small one-knob challengers against the champion on paired deals where luck
  cancels — including all-in hands, scored by their average over the runouts instead of the one
  that happened. Successive halving narrows the field; the survivor is promoted only on a 95% lower
  bound above +1 bb/100, confirmed again on fresh deals. Promoted parameters reach live play within
  turns. Every evaluation, promoted or rejected, is listed with its interval.
- **Experiment mode.** While the fleet holds the top four places, the fourth and fifth bots play an
  unresolved challenger live, swapping arms every 120 hands. Treatment hands are kept out of every
  production fit, and live evidence can retire or prioritize a target but never promote it.
- **The analyst.** Deep re-solves of stored live decisions at ten times the live budget, recording
  whether the live choice matched and what it gave up, plus a drift summary over a pinned sample of
  the newest big-spot replays. It never changes play.
- **The control room.** A React 19 + Vite dashboard served by the fleet binary: live tables with
  table themes, range explorer heatmaps, the decision strip, "why this move" EV bars, the live
  action stream over server-sent events, opponent intelligence, the leak finder, self-calibration,
  fleet race, season race, champion profile and promotion lineage, autonomy and experiments panels,
  highlights, hand replay, host check and the documentation. Widgets arrange per browser, views are
  tabs, and the last tab is remembered.
- **One-click update and rollback.** The dashboard fetches, gates, builds, installs and hot-swaps
  between turns behind a stage-by-stage progress bar. A failed stage names itself and changes
  nothing; a saved build is one click away as a rollback.
- **Storage.** SQLite with WAL, a single writer connection and read-only readers; cold JSON columns
  packed with the project's own DEFLATE codec; integrity checks, sealed backups, quarantine and
  restore; daily, weekly and monthly archives with sealed manifests on a second disk, verified
  before older ones are pruned; server hand exports imported into a training corpus.
- **Our own foundations.** `crates/deps/` replaces the third-party crates this would otherwise
  depend on: a seedable RNG with reference vectors, SHA-256 and HMAC, runtime helpers, shared
  read-only file mappings for the strength tables, and a DEFLATE codec with a zlib-equal ratio,
  pinned by fixtures. Their only third-party dependency is `libc`. Sixteen crates in three layers:
  foundations, poker and data libraries that touch no network or database, and the programs.
- **A gate instead of trust.** `scripts/check.sh` (modes `commit`, `full`, `deep`) runs the
  placeholder-marker scan, the secret scan, rustfmt, clippy with `-D warnings`, `cargo-deny`, the
  500-line file limit, the docs-drift check, the golden determinism snapshot, the property tests,
  the full workspace test suite and the dashboard's type check. `scripts/release.sh` stages a
  build, tests it, installs it and hot-swaps it, so an untested binary never lands in the running
  fleet's directory.
- **Operator scripts and a service unit.** Setup, start/stop/status under a crash-loop supervisor,
  backups, monitoring, archive timers, update and rollback, a portable bundle, an offline vendored
  build, and a systemd user unit that starts the fleet at boot.
- **Hardware measurement instead of assumptions.** CPU only by construction, with no GPU path and
  no fallback that expects one. The compute profile is measured on the machine that runs it, and
  the host check reports microcode, huge pages, memory, free space and SSD TRIM, with the command
  to fix anything that is off.
- **Tools.** `sim`, `probe`, `bench` and `tables` from `sv10-core`; `review`, `calibrate`, `ingest`,
  `replay` and `archive` from `sv10-bot`.
- **Provenance, enforced.** Every artifact in this repository names the AI system that produced it:
  `Generated-by: <tool>/<model>` in commit footers and in issue and pull request bodies, checked by
  `scripts/provenance.py` as part of the gate.
- **Licensing.** Dual-licensed MIT OR Apache-2.0, with `THIRD-PARTY-NOTICES.md` generated from the
  dependency set and a CycloneDX SBOM target alongside it.

### Security

- The Open Poker API key lives only in `.env`, which is gitignored and created mode 600. The
  pre-commit hook refuses a staged change containing the value of any key, token, secret or
  password from it, and the dashboard's setup API returns a `…last4` hint rather than a key.
- With no operator token configured, the dashboard API refuses changes that are not addressed to a
  loopback host, and every response carries `nosniff`, `X-Frame-Options: DENY` and a same-origin
  referrer policy.
- Foreign archive databases are opened read-only and immutable; every database is integrity-checked
  before use, and a failed read keeps what is already installed rather than falling back silently.
- `cargo-deny` checks licences, bans, sources and advisories in the gate.

[Unreleased]: https://github.com/SvanLabs/SvanBot/compare/v10.0.0...HEAD
[10.0.0]: https://github.com/SvanLabs/SvanBot/releases/tag/v10.0.0

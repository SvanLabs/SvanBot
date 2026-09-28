# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Banking follows the bot's chip stack** (#359). Below 500k chips, counting the off-table balance plus the table stack,
  a bot banks at 1,000 bb on the table (was 2,000 bb). From 500k it stops banking and keeps a deep
  stack for the big hands. The settings are `SVANBOT_BANK_STACK_BB` and `SVANBOT_BANK_UNTIL_CHIPS`.
- **`scripts/adopt-upstream.sh`** moves a fleet cloned from another repository onto this one in
  place, with a verified backup first (#352, #353, #354).
- **A `dev` branch** (#362). Work lands on `dev`; `main` is the released line — what everyone who
  installs SvanBot runs and updates from — and moves only by promotion (`docs/RELEASE.md`).
- **Readiness labels and a board** (#358). Every open issue is `agent-friendly`,
  `blocked-on-decision` or `needs-triage`, and https://github.com/orgs/SvanLabs/projects/1 shows them.
  Dependabot opens grouped monthly updates.
- **Cleanup every 6 hours** (`svanbot10-clean.timer`), no longer chained to the nightly archive:
  when the archive failed, cleanup silently stopped too.
- **`scripts/units.sh`** renders the systemd user units for the checkout it is run from and installs
  them (#15). The units in `scripts/` are templates that name no fixed directory, so an install under
  any home directory gets units pointing at itself and moving one is a re-render instead of an edit.
  `scripts/units.sh --check` names any installed unit that is missing or was rendered elsewhere, and
  `scripts/status.sh` reports it.

### Changed

- **The hourly backup moves to the second disk instead of being copied and checked there** (#374).
  The copy was written under a temporary name, its bytes read back against the seal, the page cache
  dropped and only then renamed — and the second disk answers that dance with `EUCLEAN` ("structure
  needs cleaning"), 22 times between 2026-09-26 11:32 and 2026-09-28 05:58, so the hour's copy never
  landed. The sealed pair now goes straight to the name a restore reads, in one step, and the SSD
  copy is removed after it: a failed move discards the partial file and leaves the SSD pair exactly
  where it was, so the worst case is one hour without a copy on a disk that is already failing, never
  a lost backup. This is the operator's call (2026-09-28): guard the hour on the SSD, not on the disk
  the kernel is complaining about.

- **The learner refreshes its evidence on the hands as they arrive** (#314). Every gate was
  denominated in hands, so the play rate decided how fast the models improved: on the live fleet the
  learner waited about two and a half hours between a 57 s refresh and a 468 s search, and the
  Autonomy panel reported it as "872 of 1000 new hands". A refresh now waits for nothing but a new
  hand — the fits live play reads are never more than about a minute old — while champion search
  keeps its own gate (the dashboard's new-hands limit, the early-season cooldown, the backoff), since
  a champion is only replaced on fresh-deal evidence. A due search yields to a refresh only when the
  fits are a whole evidence epoch (500 hands) behind, and the rejection ledger and the experiment
  target queue are scoped to that epoch rather than to the refresh watermark, so continuous
  refreshes cannot retire the search's memory of what it has already measured (0285). The panel names
  a running refresh instead of reporting it as challenger validation.

- **The findings panel states one explanation per family and tables the deep re-solve** (#321). The
  panel printed the same paragraph thirty times — the filter, the floor and the interval in every row —
  with the line saying none of it was measurable in the footnote. It now leads with the coverage (how
  much of the window carries the live inputs the deep re-solve grades), states each family's filter and
  threshold once above the rows it covers, and renders the classes as a table: class, n, bb per
  decision, its 95% interval, the decisions in the window, and what the floor made of them — `P0 ·
  filed`, `measured`, `not yet decidable`, `never queued`. A style shift and its counter-shift are one
  finding per decision point (a call up and a raise down are one change in the mix, not two findings).
  `findings.v1` carries `coverage`, `classes` and `legends`, and a ticket the loop files carries its
  family's explanation, so it reads on its own.
- **The updates panel keeps one run to a statement** (#320). Four things on the Releases card
  contradicted each other. The first was fixed in #351 (a finished run's `elapsed` is its duration,
  not the age of its record); the card now says when as well — `finished 4 min ago` — so the last run
  cannot be read as this one. A failed check rendered the incoming-commit list it could not have
  refreshed with nothing saying so: the check carries `fetched_at`, the time of the last fetch that
  worked, and the list names it. `Update complete` and `Checkout: Uncommitted changes` sat in one
  panel as though they were about the same run: the row is `Next update`, and its tooltip says that
  an update builds from this checkout, will not start while build inputs are uncommitted, and that
  the fleet keeps playing the installed build — advice an operator of a playing machine can act on,
  not a demand to commit. And the check's failure line is in operator terms ("GitHub could not be
  reached from this host — no network, or it is offline"), with the tool's own wording in the tooltip.
- **The self-calibration bias column is a measurement on every row** (#318). One cell held three kinds
  of thing — a number when a correction was applied, `learning` while a spot was under `MIN_SAMPLES`,
  and `calibrated` when it was not — so a spot that had settled to no correction read as a badge, and
  the column could not be scanned or compared. Every row now reads its correction in bb at two
  decimals, with `0.00 bb` for none (`bias_bb` itself, not a stand-in that could drift from it), and
  the state moved into the tooltip, worded from the backend's own `bound_by`: too few decisions to
  measure yet, a gap inside its 95% band, or a raise the margin would not support. The old wording
  was also the dashboard's second copy of `MIN_SAMPLES`
  (`crates/apps/bot/src/tasks/calibration.rs`); that copy is gone.

- **The front-page highlight cards point at their evidence** (#331). The five cards under Highlights
  read like a pitch: a latency range with no instrument, a think-time capability with no "when", and
  "zero-downtime" doing work that "the bots keep playing" does better. Each card is now one claim
  with the file, test or command that holds it up named inline — the `live` suite of `bench` and the
  published table for the latency, `calibrate` and the held-out install rule for the models,
  `crates/apps/bot/src/promotion.rs` for the gate, the invariant tests in
  `crates/apps/bot/src/client/decide.rs` and `crates/libs/store/src/integrity.rs` for safety, and
  `scripts/update.sh` for updates.

- **A draw that cannot fill its budget refuses to answer** (#424). The equity loop returned the mean of
  whatever it scored, so a request for 2,500 deals was answered from 17 of them, and a draw that scored
  nothing answered `0.0` — the same number as "hero never wins". #383 raised the rejection-sampling
  budget to 200 attempts per sample, which fixed the reachable cases; the cliff stayed, because the
  budget is finite and the caller's ask is not: on the issue's sparse-range harness eleven opponents
  return 1,700 deals of the 2,500 asked for, thirteen return 63, and nothing said so. A draw that
  scores fewer deals than it was asked for now reports no measurement, and a decision that cannot
  measure its equity refuses the spot — the safe action, check where the rules allow it and fold
  otherwise, with `equity: None` — instead of pricing its candidates on a zero. The refusal is
  recorded as a NULL equity, which every reader already treats as unavailable (`--` in `review all`,
  a dash in the dashboard), and the offline generators, whose draws cannot be short, say so with
  `expect` instead. Six-max play never reaches the budget and is unchanged.

- **A full-ring deal set arrives complete** (#383). The Monte Carlo loops behind every equity — the
  shared deals a decision prices its candidates on, and the per-range fallback — gave up after 20
  attempts per sample and returned what they had without saying so. A deal is rejected when two
  opponents' drawn combos share a card, so sparse ranges are the hard case: nine opponents returned
  2053 of the 2500 deals asked for, ten returned 299. The budget is 200 attempts per sample now, paid
  only where a request cannot be filled — milliseconds at a full ring, and nothing at all at six-max,
  where the loop never reaches 20, so the golden snapshot and the six-max fill test are unchanged.

- **The maintainer job builds from a cache** (#363). `claude-maintainer.yml` — the twice-daily run
  that fixes the oldest open `agent-friendly` issue — compiled the workspace from nothing every time,
  because unlike the `gate` job it restored no cache: a cold `npm ci` and a cold compile before the
  agent did anything, `full took 227 s` where the same job warm takes `121 s`. It restores the gate's
  two caches now, and with them the gate's `RUSTFLAGS` baseline (#330): a cache carries `target/`
  between runners and `.cargo/config.toml` compiles for the host doing the compiling, so caching this
  job without that line would have brought back the SIGILL it was fixed for.

- **The Experiments panel says why candidates die** (#317). Every entry the learner recorded was a
  rejection, and the reason was a sentence inside `rationale`, so the panel could only show a wall of
  `rejected`: whether the search is starved of candidates that move a decision, re-proposing what the
  ledger already barred, or losing every survivor on fresh deals was not readable at all. Each
  experiment now carries a `stage` and a `reason`, the learner counts outcomes by the hour in
  `learner.search-funnel.v1` — including the halved-out stage, which recorded nothing before — and
  the panel opens with the last 24 hours as counts by stage and by knob. `promotion::Verdict`'s
  rejection carries a `Reason` enum rather than a string, so a branch that stops reporting why it
  rejected no longer compiles.
- **`main` is the repository's default branch** (#402). It was `dev`, so the front page and a fresh
  `git clone` showed the branch work lands on. `main` — the released line, promoted from a green
  `dev` — is what a visitor and a new install should land on. Work still lands on `dev` and every
  pull request still goes into it; `README.md`, `CONTRIBUTING.md`, `AGENTS.md` and `llms.txt` now
  name the base to use rather than the one GitHub offers. Three things followed the default branch
  without saying so and are now explicit, because each of them crossing to `main` would put content
  on the released line without passing through `dev`: Dependabot opens against `dev`
  (`target-branch`), and `claude-maintainer.yml` and `promote.yml` check `dev` out by name — the
  latter because GitHub runs a `workflow_run` workflow from the default branch, so `promote.sh` would
  otherwise have to be promoted before it could promote.
- **`main` keeps itself current** (#370). `main` is the released line — what everyone who installs
  SvanBot runs and updates from — and it moved only when someone remembered to open the promotion
  pull request, so it drifted: it was promoted once and was behind again within the hour.
  `scripts/promote.sh` now keeps a promotion pull request open with auto-merge armed, and
  `.github/workflows/promote.yml` runs it whenever a `check` on `dev` finishes (#404), so `main`
  follows `dev` by itself and never carries a build that failed.
- The Claude workflows act as the SvanLabs GitHub App (`svanlabs[bot]`) (#355).

### Fixed

- **A `main` carrying its own content promoted once `dev` moved on** (#387). The refusal to promote a
  diverged `main` asked two questions — does `main` hold a non-merge commit `dev` lacks, and has
  `dev` no commit `main` lacks — and a conflict resolved on `main`'s side inside a *merge* commit was
  invisible to both as soon as `dev` moved on: the first counts non-merge commits only, and the second
  is then the ordinary, healthy shape. The promotion went through as a real merge that kept `main`'s
  resolution on `main`'s side alone, where `dev` can never reach it, and left the two trees different
  however green `dev` was. Both guards are one now: `main`'s tree against the tree of the merge base of
  the two lines — `dev`'s content at the point they last agreed. A promotion merge holds exactly that
  tree, which is what makes the healthy shapes pass, and anything `main` carries of its own makes them
  differ whichever commit holds it. The refusal still names which shape it found. Found by building
  the state on purpose, not by an incident.
- **A cancelled `check` on `dev` parked `main`, and nothing retried the promotion** (#404).
  `promote.yml`'s job was gated on the finished run's conclusion being `success`, which read as "a red
  `dev` promotes nothing" and was never the thing doing that work: the guard is the `check` status the
  ruleset on `main` requires of the promotion pull request, so a `dev` that would fail the gate cannot
  merge whatever starts the job. What the filter did instead was make a promotion depend on the run
  that *reported* — a `check` that ends `cancelled` (a push superseded by the next one, or a
  preempted runner) is not `success`, so the job was skipped, and when no later push came, `main`
  rested behind a `dev` that had been green the whole time, silently. A `check` on `dev` is now
  considered whatever it concluded; `scripts/promote.sh` is idempotent, refuses a `main` carrying
  content of its own, and returns without a word when the two trees are equal, so looking again costs
  a few `gh` calls.
- **A pull request into `dev` named an issue and closed nothing** (#410). GitHub interprets a closing
  keyword in a description only when the pull request targets the repository's *default* branch — "The
  pull request must be on the default branch" — and with `main` the default (#402), `Closes #<issue>`
  made no link at all: PR #409 merged into `dev` with `Closes #408` in its body and #408 stayed open.
  Every document here asks for that line, and the maintainer's rule is the oldest open
  `agent-friendly` issue **that no pull request closes yet**, tested with `gh pr list`, which lists
  open pull requests only — so an issue whose fix had landed stayed open, was reported as closed by no
  open pull request, and could be picked and done a second time. `.github/workflows/close-linked-issues.yml`
  now closes what a merged description names, using GitHub's own keywords
  (`scripts/close-linked-issues.py`, covered by the gate). On `main` the platform has already closed
  them and the request is a no-op.
- **A red gate named the suite it died in, not the test** (#408). `scripts/check.sh` ran the Python
  tool tests and the eight shell suites with their output discarded — `unittest` writes its failures to
  stderr and the suites write theirs to stdout — so the whole of the evidence was the suite's name.
  On 2026-09-28 a push to `main` failed inside the tool tests (run 36387980518): the log holds the
  `== tickets lint + tool tests` header and `Process completed with exit code 1`, the same tree passed
  on `dev` two minutes earlier and passes locally on both trees, and which test failed is still
  unknown. Each suite now runs with its output kept and the last 30 lines printed before the gate
  reports, which is what the two steps above it already did for tickets lint and docs drift.
- **Every Dependabot pull request carried a failed review check** (#375). GitHub withholds Actions
  secrets from a workflow a bot triggered, so `CLAUDE_CODE_OAUTH_TOKEN` reached the review empty and
  the run died on a credential it was never given. The check was red on every dependency bump and
  said nothing about the bump. The review job is now skipped for a Dependabot-opened pull request;
  the gate still checks the bump.
- **A fleet set to `dev` could not install a release** (#365). The gate's update test builds a fixture
  whose origin has only `main`, and its list of ambient variables to clear missed the three settings
  `scripts/update.sh` reads for where it fetches from. With `SVANBOT_UPDATE_BRANCH=dev` exported — what
  an operator sets to put a fleet on `dev` — the fixture looked for a branch its origin does not have,
  so `scripts/check.sh full` failed and every release on that fleet stopped before it could build. CI
  could not catch it: no such variable is set there.
- **A pull request opened by a bot failed the provenance gate** (#367). The check exempts a commit
  whose author is `<name>[bot]`, because the author field already names the system, but the
  pull-request half never received the author — so every Dependabot pull request failed on
  `pr-body.md: no Generated-by line in the pull request body` and none of them could merge.
- `bench` and `sim` name a malformed input file instead of panicking (#357).

## [10.0.1] - 2026-09-28

Correctness fixes on paths the gate could not reach, the release and maintenance workflows that were
configured but inert, and two build and dashboard corrections. Nothing here changes the decision
path or the stored data, and the golden snapshot is unchanged.

### Fixed

- **A resumed hand panicked on a non-ASCII card pair** (#97). `resume_hand` sliced a saved `hole` by
  byte offset behind a byte-length check, so a four-byte string holding a two-byte character sliced
  a char boundary. The payload comes off the wire, so any hand being resumed could carry it; the
  guard now requires ASCII as well as the length.
- **A pot with no live seat panicked instead of being refunded** (#33). `split_pots` took the best
  hand among the live seats without checking that there was one, so an all-folded pot reached
  `max().unwrap()` on an empty list. Play cannot reach the state — the hand ends at one live seat —
  but settlement and the luck adjustment of stored hands call it directly; it now refunds the seats
  that paid in.
- **A zero runout budget returned NaN equity** (#31). `expected_net` averaged the sampled runouts by
  the count it took, and a budget of zero ran the loop no times: `0.0 / 0.0` for every seat. A zero
  budget is now one runout.
- **The combo-index sentinel was reported as an index** (#99). Slot 65535 marks a card pair that is
  not a hand; `combo_index` returned it as one, so a caller indexing a 1,326-entry table read
  whatever the allocation held, or panicked.
- **A denied `localStorage` blanked the dashboard** (#30). Where the origin is opaque or site data
  is blocked — a private window, a sandboxed iframe, a `file://` page — the accessor raises instead
  of returning `null`, and the reads ran before the first paint under no error boundary, so the
  throw unmounted the root and left a blank page. Every read is now guarded.
- **A failed hand-class load left no trace** (#28). The range grid gated its main chart on a list
  whose fetch threw its failure away, so a failed request deleted the panel's chart and looked
  exactly like a bot with no range. The failure now surfaces in the grid.
- **State hashes could disagree with the venue's on a float** (#34). The hash covers compact JSON
  written by CPython's `repr`, and Rust's `{:e}` breaks a tie between two equally short spellings the
  other way; about 1 in 4,600 frames diverged. The Rust side now spells floats as CPython does.
- **The provenance gate never read the pull request body in CI** (#27). `scripts/check.sh` takes the
  `--pr-body` branch when `SVANBOT_PR_BODY` names a file, and no step set it, so a description
  without a `Generated-by:` line passed green while `AGENTS.md`, `CONTRIBUTING.md` and the pull
  request template all said the gate read it.
- **A `v*` tag that predates the Dockerfile failed the image build** (#26). It died inside
  `build-push-action` reading a Dockerfile that does not exist at that tag, which reads like a broken
  build rather than an old tag. The step is now skipped for a tag with no Dockerfile.

### Changed

- **A local build now matches the one that plays** (#24). This tree ships the `target-cpu=native`
  `.cargo/config.toml` the working repository builds with, so a plain `cargo build --release` is
  built the same way on whatever machine runs it. Portable and container builds are unaffected: both
  set `RUSTFLAGS` explicitly, which replaces the file's.
- **The dashboard's Playwright specs are type-checked** (#29). `web/tests`, `playwright.config.ts`
  and `vite.config.ts` were outside every tsconfig, so the check CI runs compiled the app and
  stopped. They are now covered by a second config that the `tsc` step runs alongside the app's.

### Added

- **Every release attaches binaries and publishes a container image** (#23). A `v*` tag attaches the
  portable x86-64-v2/v3 bundle with its SHA-256 and an attestation, and pushes an attested image to
  `ghcr.io/svanlabs/svanbot`. A pull request that touches the image builds it without pushing.
- **New issues are triaged and answered** (#22). A workflow labels every opened issue and new
  comment, answers it, closes duplicates, and adds `agent-friendly` when the issue is specified well
  enough to be fixed unattended.
- **The repository is maintained on a schedule** (#21). Twice a day the oldest open `agent-friendly`
  issue is fixed on a branch, gated and opened as a pull request with auto-merge, so fixes land when
  no session is open.

### Removed

- **The private saga's static mount** (#25). `/saga/` served an unrelated private project from a
  directory that never exists in this tree; the route served nothing, and the name had no business
  being public.

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

[Unreleased]: https://github.com/SvanLabs/SvanBot/compare/v10.0.1...HEAD
[10.0.1]: https://github.com/SvanLabs/SvanBot/releases/tag/v10.0.1
[10.0.0]: https://github.com/SvanLabs/SvanBot/releases/tag/v10.0.0

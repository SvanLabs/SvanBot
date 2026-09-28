# Contributing to SvanBot

This file has two parts. [**Your first pull request**](#your-first-pull-request) is the path from
an issue to a merged change, and it is short. [**The standard**](#the-standard) below it is every
rule a change is held to, with the enforced ones marked **MUST**.

| | |
|---|---|
| 🚀 **Getting started** | [Your first pull request](#your-first-pull-request) · [Commit messages](#commit-messages) · [What gets a change refused](#what-gets-a-change-refused) |
| 📏 **The standard** | [0 AI provenance](#0-ai-provenance-must) · [1 Verification](#1-verification-must) · [2 Formatting](#2-formatting-must) · [3 Linting](#3-linting) · [4 Naming and organization](#4-naming-and-organization) · [5 Error handling](#5-error-handling) · [6 Safety](#6-safety) · [7 Documentation](#7-documentation) · [8 Testing](#8-testing) · [9 Dependencies and performance](#9-dependencies-and-performance) · [10 Changes and review](#10-changes-and-review) · [11 CI](#11-ci) |

## Your first pull request

Every change here is made by an AI coding agent that you direct. You choose the goal and judge the
result; the agent writes the code, the tests, the commits and the pull request.

1. **Pick an issue.** The
   [**good first issue**](https://github.com/SvanLabs/SvanBot/issues?q=is%3Aissue+is%3Aopen+label%3A%22good+first+issue%22)
   and [**agent-friendly**](https://github.com/SvanLabs/SvanBot/issues?q=is%3Aissue+is%3Aopen+label%3Aagent-friendly)
   labels mark issues that say where the problem is, why it matters and the fix they expect. Comment
   on the one you take, so two agents do not race for it. Issues labelled `blocked-on-decision` wait
   on a maintainer's choice; a comment there is welcome, a pull request is not.
2. **Fork, and make one branch per change off `dev`** (the default branch; pull requests go into
   `dev`, and `main` is only ever updated from it), named for it: `fix/split-pots-all-folded`,
   `fix/localstorage-guard`.
3. **Brief your agent.** Give it [`AGENTS.md`](AGENTS.md) and the issue. For a bug, ask for a
   failing test first: a fix lands with a test that fails on the old code (section 8).
4. **Iterate on the fast loop.** `python3 scripts/test.py <filter>` runs only the tests you touched,
   in seconds, and `scripts/check.sh commit` is what the pre-commit hook runs.
5. **Run the whole gate** before you push. It is the same command CI runs.

   ```sh
   scripts/check.sh full
   ```

6. **Open the pull request** from the template. Put `Generated-by: <tool>/<model>` in the body, as
   well as in every commit footer, and `Closes #<issue>` so the issue closes when it merges.
7. **Answer the review.** CI runs the gate on every pull request, and pull requests from this
   repository's own branches also get an automatic Claude review. Fix what they find with new
   commits rather than a force-push, so the conversation still points at the right lines.

## Commit messages

The history is read on GitHub more often than in a terminal, so every commit has the same shape:

```
<area>: <what the commit does, imperative, at most 72 characters>

<Why the change was needed, and anything a reviewer should know. Wrapped
at 72 columns.>

Closes #<issue>

Generated-by: <tool>/<model>
```

- **`<area>`** is where the change lives: a crate without its `sv10-` prefix (`venue`, `policy`,
  `store`, `bot`), or `web`, `scripts`, `ci`, `docs`.
- **The subject says what the commit does**, not what you did: `venue: reject non-ASCII hole cards
  instead of panicking`, not `fixed bug`. No trailing period, no quotation marks, no issue number.
- **`Closes #<issue>` gets its own paragraph.** A line that is not a trailer inside the trailer
  block stops git reading the block as trailers at all, and `Generated-by:` disappears from
  `git log --format=%(trailers)` and GitHub's commit view.
- **The trailers come last**, one per line: `Generated-by:` and any `Co-Authored-By:` your tool adds.

A complete one:

```
engine: return an empty payout when every seat folded

split_pots unwrapped the best hand among the live seats, and a hand in
which every seat folded has none, so replaying one from the store took
the process down. Both empty cases now return the zero vector, and the
all-folded hand is a test in settle.rs.

Closes #4

Generated-by: claude-code/claude-opus-5-5
```

## What gets a change refused

These are the ones a reviewer refuses rather than fixes, because each is cheaper to get right than
to argue about:

- **No `Generated-by:`** in a commit footer or the pull request body (section 0).
- **More than one change.** A bug fix and a rename are two pull requests (section 10).
- **A regenerated golden snapshot** with a changed line nobody explained (`AGENTS.md` section 4).
- **A behaviour change without a paired simulation**, or **a speed claim without a measurement**
  taken on the same machine against the previous commit (sections 8 and 9).
- **A test adjusted until it passes** when nobody understood why it failed. Say so in the pull
  request instead.

## The standard

The coding standard for this workspace. It is written for people and for AI agents working in
the code; both read files in pieces, so small, single-purpose files and explicit reasons in comments
matter here more than usual.

**Mandatory** rules (marked **MUST**) are enforced by `scripts/check.sh` (pre-commit and CI) or by
review; a change that breaks one does not merge. **Recommendations** (marked *should*) are the default;
depart from one when you can say why, in the change.

The project in one line: a Rust 2024 workspace (MSRV 1.98, toolchain pinned to 1.98.1 in
`rust-toolchain.toml`) of foundation crates (`crates/deps/`), poker and data libraries (`crates/libs/`)
and programs (`crates/apps/`: `sv10-core` with the simulation tools, `sv10-bot` with the fleet, learner,
analyst and CLI tools), plus a React dashboard (`web/`). Nothing is published (`publish = false`).
Read `AGENTS.md` and `docs/LESSONS.md` before a first change.

## 0. AI provenance (MUST)

Every artifact in this repository is generated by an AI system, and it says so. Code, tests,
documentation, commit messages, issues, pull requests and review comments all name the system that
produced them. A person directs the work — writes the prompt, chooses the goal, approves the result —
and the machine produces the artifact. Hand-written contributions are declined however good they are;
the way to fix something here is to open an agent and have it done here.

Name the system in the footer of every commit, and in the body of every issue and pull request:

```
Generated-by: <tool>/<model>
```

`<tool>` alone is enough when the model is not known, so `Generated-by: claude-code` is valid. This is
a git trailer rather than a line of prose so that `git log --format=%(trailers)` finds it and GitHub
renders it in the commit view.

`scripts/provenance.py check` is the gate: it walks the commits in a range and fails listing the ones
that name no system, and `scripts/check.sh` runs it, so a commit or a pull request that skips the line
does not merge. Merge commits are skipped — they carry no content of their own — and a commit whose
author is already `<name>[bot]` is exempt, because the author field has already named it.

`AI-PROVENANCE.md` is the roster of systems on record, read from the same trailers, and
`scripts/provenance.py report --check` keeps it current — it fails when a system has commits that the
document does not list, so a system's first commit is a red gate until the file is regenerated. It
compares the set of systems and not the counts beside them: a count moves with every commit, and a
document every pull request had to regenerate would be a document nobody regenerates.

**What the gate enforces is declaration, not authorship.** A trailer is self-reported, and a
determined person can add one to hand-written code. The rule makes the convention mandatory and
visible; only review makes it true. A green check here is not a guarantee about who wrote the code,
and it is not meant to be read as one — the roster says so at the top of the page as well, because a
list of systems with commit counts beside it is exactly the shape of a document that invites the
wrong reading.

The engineering brief an agent works from is `AGENTS.md`; this file is the standard it is held to.

## 1. Verification (MUST)

Before every commit that touches `crates/`:

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features            # or: python3 scripts/test.py (same tests, parallel)
cargo doc --workspace --no-deps                  # 7 known warnings today; see section 11
cargo metadata --locked --format-version 1       # Cargo.lock matches the manifests
scripts/check-file-size.sh
```

In practice run `scripts/check.sh full` (all of the above plus cargo-deny, the golden snapshot, the
docs-drift check, tool tests and `tsc`; ~20 s after a one-file edit). The pre-commit hook runs
`scripts/check.sh commit`. Build with `CARGO_TARGET_DIR=target/dev` (the default of every script) —
never into `target/release`, which the live fleet runs from (LESSONS 9). No crate defines features,
so `--all-features` changes nothing today; keep it so a future feature cannot hide from the gate.

## 2. Formatting (MUST)

- `rustfmt` is the source of truth; never hand-format against it. `rustfmt.toml` sets
  `max_width = 140` and `use_small_heuristics = "Max"` — wider than rustfmt's 100. That is a
  project decision (dense numeric code reads better on one line); do not change it in a feature
  change, because it would reformat every file.
- Indentation is rustfmt's (4 spaces). No trailing whitespace (rustfmt removes it in Rust; editors
  must not add it elsewhere).
- *Should*: keep expressions readable within the width — name intermediate values rather than
  chaining past ~3 method calls in one expression.

## 3. Linting

Current workspace configuration (`Cargo.toml`, every crate opts in with `[lints] workspace = true`):

```toml
[workspace.lints.rust]
unsafe_code = "deny"            # allowed per crate only in sv10-rt and sv10-mmap (see §6)
unused_qualifications = "warn"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
dbg_macro = "deny"
todo = "deny"
unimplemented = "deny"
```

Library crates add `#![warn(missing_docs)]` in `lib.rs`. Warnings are errors in the gate (`-D warnings`).

- **MUST**: every `#[allow(...)]` / `#![allow(...)]` carries a comment on the same or the previous line
  saying why it is safe (today 19 of 20 do).
- **MUST**: no crate-wide or workspace-wide `allow` without that comment and a ticket reference.
- *Recommended additions*, in order, each landing with its violations fixed in the same change (they
  would fail the gate today):
  1. `clippy::allow_attributes_without_reason = "warn"` — move the reason comments into
     `#[allow(lint, reason = "...")]` (Rust ≥ 1.81), so the rule above is checked, not reviewed.
  2. `rust.missing_docs = "warn"` at workspace level (today per library crate; `sv10-bot` lacks it).
  3. `clippy::unwrap_used` and `clippy::expect_used` = `"warn"` with `allow-unwrap-in-tests = true`
     and `allow-expect-in-tests = true` in `clippy.toml` (81 production uses to triage first).
  4. `clippy::print_stdout` / `print_stderr` = `"warn"` in library crates only (the CLI binaries print
     by design).

## 4. Naming and organization

- **MUST**: standard Rust naming — `snake_case` functions, variables, modules and files; `CamelCase`
  types and traits; `SCREAMING_SNAKE_CASE` constants and statics. (rustc warns on violations; the
  gate makes the warning fatal.)
- **MUST**: respect the crate layers (`AGENTS.md`, Layout): `deps` depend on nothing of ours, `libs`
  do no network or database I/O (except the store and the named file mappings), apps own the I/O.
  Internal paths and third-party versions live once, in `[workspace.dependencies]`.
- *Should*: one clear responsibility per module; split by responsibility, domain concept or
  subsystem, and re-export deliberately so callers keep their paths.
- *Should*: keep public APIs deliberate: `pub(crate)` or `pub(super)` unless another crate needs it.

### File and function size

Because agents and reviewers read this code in pieces:

- **MUST**: no Rust source file over **500 physical lines** (code, comments, blank lines and inline tests
  all count). `scripts/check-file-size.sh` enforces it. The files over the limit when it was adopted
  are listed in `scripts/file-size-baseline.txt`: they may shrink, never grow, and each is a refactor
  backlog item. A file that comes back under the limit leaves the list, and the check fails until its
  line is deleted — an entry is a ceiling, so one left behind re-grants the lines the file gave up.
- *Should*: target **300** lines; past **400**, split before adding functionality.
- Exceptions (file named here with the reason, or a comment at the top of the file): generated code,
  large test fixtures, and naturally cohesive units whose split would hurt clarity (for example a
  single state machine or a codec table). Do not split into fragments that must be read together to
  make sense, and do not create files to dodge the count.
- *Should*: move a large inline `#[cfg(test)] mod tests` into a sibling `tests.rs`
  (`#[cfg(test)] mod tests;`), as `sv10-venue::tracker` does, before splitting production code.
- *Should*: functions under **50** lines; a function over **100** lines needs a reason in review
  (27 exceed it today, led by the learner's and review's `main`).

## 5. Error handling

- **MUST**: no `unwrap()`/`expect()` in production code unless the condition is impossible by
  construction, with the reason in the `expect` message or a comment ("a qualifying reading has a fleet
  top four"). Input from the network, files, the database, the environment or the operator is never
  an invariant.
- **MUST**: never silently ignore an error. `let _ = fallible()` needs a comment saying why the failure
  does not matter (cleanup of a temporary file, a send to a receiver that may be gone). Store writes
  are never discarded silently (LESSONS 20).
- **MUST**: error messages say what failed and with what (`format!("season identity not stored: {e}")`,
  `.context("reading params.v1")`).
- Error types, adapted to this project:
  - Applications (`sv10-bot` binaries and modules, `sv10-core` bins) use `anyhow` with context.
  - Libraries whose callers branch on the failure define their own error enum with `Display` and
    `std::error::Error` by hand, as `sv10-pack::Error` does. **Not `thiserror`**: the foundation rule
    replaces third-party crates with our own where the value is small, and a hand-written enum
    is a few lines. `sv10-store` currently returns `anyhow::Result`; converting it is a recommendation,
    not a requirement, because only the apps consume it.
  - Pure computation (`cards`, `equity`, `engine`, `policy`, `model`) should not fail: take validated
    inputs and return values; use `Option` for "no answer".
- *Should*: prefer `?` with context over matching and re-wrapping by hand.

## 6. Safety

- **MUST**: `unsafe` only in `sv10-rt` (environment, `statvfs`, `posix_fadvise`) and `sv10-mmap`
  (`mmap`, `munmap`, `madvise`) — the two crates with `#![allow(unsafe_code)]`; everywhere else the
  workspace denies it. (The template's "low-level parser module" does not exist here: the parsers are
  safe Rust.)
- **MUST**: every `unsafe` block has a `// SAFETY:` comment directly above it naming the invariant it
  relies on (all 11 do today).
- **MUST**: unsafe code stays behind a small safe API in its crate; no `unsafe` for convenience or
  unmeasured speed.

## 7. Documentation

- **MUST**: public items of library crates are documented (`missing_docs` is on for them).
- **MUST**: documentation changes with the code in the same commit: `docs/` (ARCHITECTURE, OPERATIONS,
  SPEC-*, LESSONS), `AGENTS.md` where it states a fact, and module docs. `scripts/docs-check.py` fails
  the gate when a doc names a path or command that does not exist.
- *Should*: explain *why* non-obvious code exists, citing the ticket (``) or lesson that forced it.
- *Should*: examples (doc tests) for the important public APIs of `deps` and `libs` crates.

## 8. Testing

- **MUST**: a bug fix lands with a test that fails on the old code (the audit rule of 2026-09-26);
  a behaviour change updates the golden snapshot deliberately (`UPDATE_GOLDEN=1`, AGENTS.md).
- **MUST**: speed-only changes reproduce `SIM_B='{"call_margin":0.005}' sim paired 12 600` exactly.
- *Should*: unit tests for core logic next to it; integration tests (`tests/`) for crate-level
  behaviour; error cases and boundaries, not only the happy path; test names that read as the
  behaviour (`a_renamed_bot_is_one_accuracy_row`).
- **MUST**: tests never touch the network, the live databases or `.env`; they use temporary
  directories and fixed seeds. Timing-dependent tests use a fake clock (see
  `experiment::mode` tests). Tests that read the real host (`hostcheck`'s
  `reading_the_real_host_never_fails`) may only assert that reading does not fail.

## 9. Dependencies and performance

- **MUST**: a new third-party dependency needs a stated reason, goes into `[workspace.dependencies]`,
  passes `cargo deny` (licenses, bans, sources, advisories) and updates `THIRD-PARTY-NOTICES.md`
  (`scripts/notices.py`). Prefer std or our own `deps/` crates.
- **MUST**: no performance change without a measurement (LESSONS 1, 3, 6): state the benchmark and the
  numbers in the commit or ticket; document performance-sensitive decisions where they live.
- *Should*: mind ownership and allocation on hot paths (the decision search, the learner's paired
  simulations); no lock held across `.await`; snapshot shared state instead of holding locks in
  blocking work.

## 10. Changes and review

- **MUST**: one focused change per commit; no formatting-only edits mixed with behaviour changes.
- **MUST**: preserve behaviour unless the change says otherwise (and then the golden snapshot and
  the tickets say so too).
- **MUST**: no dead code, commented-out code, debug prints or placeholder markers — `check.sh` rejects
  the marker words, and clippy denies `dbg!`, `todo!` and `unimplemented!`. Unfinished work is a ticket.
- **MUST**: commits that touch `crates/` pass `scripts/check.sh commit` (the hook) and batches pass
  `scripts/check.sh full` before a release (`scripts/release.sh`).

## 11. CI

`.github/workflows/check.yml` runs on every pull request and every push to `main`, in three jobs:

- **`web`** — `npm ci`, `npm run typecheck` (both tsconfigs: the app's `src`, and the Playwright specs
  plus the web root's Node-side config files) and a production build, so a change under `web/` gets an
  answer in about a minute instead of waiting behind a Rust build.
- **`gate`** — `scripts/check.sh full`, the same command you run locally. That is the whole gate in
  one place: AI provenance, placeholder markers, rustfmt, the lockfile check, the file-size check,
  clippy `-D warnings`, cargo-deny, the third-party notices check, the tool tests, docs drift, the
  workspace tests and `tsc`. It checks out the full history, because the provenance step reads the commits a pull request
  adds against its base branch, and it writes the pull request description to a file and names it in
  `SVANBOT_PR_BODY`, so the same step reads the description too. On a push to `main` there is no
  description, the variable is not set, and the step checks the commits alone.
- **`check`** — one aggregator that fails unless both jobs succeeded. It is the only status check
  branch protection requires, so a job added to the workflow is covered the moment it exists rather
  than the next time someone remembers to list it.

Three additions still worth making, all deliberately not done yet:

- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` as a step, once the 7 doc warnings are
  fixed. Until then it would only add a red job.
- `scripts/check-file-size.sh --report` in the job summary, so files approaching the limit are visible.
- An MSRV job (`cargo hack check --rust-version`). The `rust-version` fields say 1.98, which is what
  the workspace has always compiled on rather than a tested minimum; do not claim support for an older
  toolchain until a job actually proves it.

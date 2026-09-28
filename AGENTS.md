# SvanBot — agent brief

If you are an AI agent working in this repository, read this file first. It is the short version:
what the project is, how to build it, and the rules that a change has to satisfy. The full standard
is `CONTRIBUTING.md`; the reasons behind the rules are in `docs/LESSONS.md`.

## Your loop, start to finish

1. **Read the issue, then the code it names.** Issues here say where the problem is, why it matters
   and the fix they expect. If yours does not, say what is missing before you guess.
2. **Find the layer** the change belongs in (section 3). One change, one layer, one pull request.
3. **Write the failing test first** when it is a bug. Run just that test while you work:
   `python3 scripts/test.py <filter>`.
4. **Make the change**, and update every document that names what you changed (section 5).
5. **Run `scripts/check.sh full`.** Green here is green in CI.
6. **Commit in the shape `CONTRIBUTING.md` describes** — `<area>: <what it does>`, a body that says
   why, `Closes #<issue>` in its own paragraph, and `Generated-by:` last (section 0).
7. **Open the pull request** from the template, with the same `Generated-by:` line in the body.

## Branches

`dev` is the default branch and where all work lands: cut your branch from `dev` and open the pull
request into `dev` (squash merge). `main` is the released line — what everyone who installs SvanBot
runs, and where their Update fetches from — and moves only when `dev` is promoted to it by a pull
request from `dev` into `main`, merged with a merge commit so the two histories stay one. Promotion
happens only when `dev` is green, so `main` never carries a build that failed. Never open a feature
pull request into `main`.

The reference fleet is the exception: it tracks `dev` (`SVANBOT_UPDATE_BRANCH=dev`), so every change
is played before it is promoted.

## Which issue to take

Every open issue carries one readiness label. Take an `agent-friendly` one: it names the files, says
what the fix is and needs no decision from anyone. `blocked-on-decision` means the diagnosis is done
but a maintainer or the operator has to choose first (a comment on the issue says what), and
`needs-triage` means something has to be checked or added before it can be specified. Do not start
either of those; comment on what you found instead. Comment on the one you take, so two agents do not
race for it. The same view is a board: <https://github.com/orgs/SvanLabs/projects/1> (its
**Readiness** field mirrors these labels).

## 0. Every artifact here is machine-generated, and says so

Code, issues, pull requests, review comments and commit messages in this repository are produced by
AI systems. Name yours. A commit that does not carry the trailer fails the gate:

```
Generated-by: <tool>/<model>
```

`Generated-by: claude-code/claude-opus-5` and `Generated-by: aider/gpt-5` are the shape — the tool,
a slash, the model. Where the model is not known, the tool alone is acceptable. The trailer goes in
the commit footer, with the other trailers, so `git log --format=%(trailers)` reads it. **Pull
request and issue bodies carry the same line**, in the body rather than the title.

`scripts/provenance.py check` enforces this: it walks the commits in range and fails naming the
ones that are missing it. `scripts/check.sh` runs it, so CI rejects a pull request that skips it. It
runs in the pre-commit hook too, which is where you want to find out.

One thing this gate does not do, stated plainly so nobody mistakes a green check for a guarantee:
it enforces *declaration*, not *authorship*. A trailer is self-reported. The gate makes the
convention mandatory and visible; only review makes it true.

## 1. What this is

A self-hosted, fully autonomous poker bot fleet for [openpoker.ai](https://openpoker.ai) — 6-max
no-limit hold'em, virtual chips, 14-day seasons. It plays up to five portfolio bots, learns every
opponent it meets, and reports live in a web control room. Rust workspace plus a React dashboard.
**CPU-only is a hard constraint**: there is no GPU path and no fallback that assumes one.

## 2. Build, test, gate

```
cargo build --profile release          # the fleet binary
python3 scripts/test.py                # the test suite; add a filter to run one thing
scripts/check.sh commit                # pre-commit: fmt, clippy, golden, tsc
scripts/check.sh full                  # the gate. everything below assumes this is green
```

**Build into `target/dev`, not `target/release`, while a fleet is running.** A release build lands
in the directory the live processes hot-swap from, so building there swaps untested code into a
running bot. That is the one genuinely dangerous mistake available here:

```
CARGO_TARGET_DIR=target/dev cargo build --profile release
```

Run only the tests you touched while iterating — `python3 scripts/test.py <filter>` — and the whole
suite once at the end. The full suite takes minutes; a single file takes seconds, and the loop
matters more than the coverage when you are mid-change.

## 3. Where the code goes

`crates/` has three layers, and a change belongs in exactly one of them:

- **`crates/deps/`** — our own replacements for third-party crates (`sv10-rng`, `sv10-digest`,
  `sv10-rt`, `sv10-mmap`, `sv10-pack`). No third-party dependency except `libc`.
- **`crates/libs/`** — poker and data libraries: `sv10-cards`, `sv10-equity`, `sv10-engine`,
  `sv10-nn`, `sv10-model`, `sv10-policy`, `sv10-stats`, `sv10-venue`, `sv10-store`.
- **`crates/apps/`** — the programs: `sv10-bot` (the live fleet and dashboard API), `sv10-core`
  (re-exports the libraries and holds the tool binaries).

**Poker logic does not touch the network, the database or the clock.** A library crate that opens a
socket or a SQLite handle has broken the architecture the whole design rests on — the libraries are
what make the engine testable and the simulations reproducible, and both stop being true the moment
one of them can reach outside the process. I/O belongs in `crates/apps/`.

Third-party versions live once, in the root `[workspace.dependencies]`; a crate names them with
`x.workspace = true`.

## 4. The invariants

Two are hard, in the sense that violating one is a bug and not a style question:

- **Never send an action that is not in `valid_actions`**, and always echo `hand_id` and
  `turn_token` back with it. The server rejects anything else, and a rejected action at the wrong
  moment is a lost hand.
- **Every behaviour change passes a paired simulation**, and **every performance change carries a
  measurement** taken on the same machine against the previous commit. A speed claim with no number
  behind it is not a change, and neither is a strategy change with no simulation.

The golden snapshot is the determinism check. If your change is intended to alter behaviour, it will
fail, and that is it working:

```
UPDATE_GOLDEN=1 python3 scripts/test.py      # regenerate, then read the diff and justify it
```

Never regenerate the golden file to make a red test go green. If you cannot explain why each changed
line should change, the change is wrong.

Promotion of a learned parameter is gated on a 95% lower bound clearing +1 bb/100 in sequential
fresh-deal confirmation. Do not loosen that to make an experiment fit.

## 5. Style that is enforced, not suggested

- **`scripts/check.sh` runs `cargo fmt --check` and `clippy --workspace --all-targets --all-features
  -D warnings`.** Both must be clean.
- **500 lines per Rust file**, enforced by `scripts/check-file-size.sh` against a baseline that may
  only shrink — a file that comes back under the limit leaves the list, and the check fails until
  its line does. A file crossing it gets split, not exempted.
- **Documents name only paths that exist.** `scripts/docs-check.py` checks every backticked
  repository path in the live documents, listed in `scripts/docs-check.live`. If you move a file,
  the documents that name it are part of the change.
- **No placeholder markers.** `TODO`, `FIXME`, `XXX` and `HACK` fail the gate — finish the work or
  open an issue.
- **Every dependency must be permissive.** `deny.toml` fails the build on anything else. If you
  reach for a new crate, explain why one in the workspace will not do.
- **`THIRD-PARTY-NOTICES.md` is generated.** Run `python3 scripts/notices.py` after a dependency
  change; the gate fails when it is stale.

## 6. Working in this repository

- Read `docs/LESSONS.md` before a first change. It is a ledger of mistakes that were already paid
  for, each with the rule that came out of it, and most of the rules above are in there with a much
  better explanation of why.
- `docs/ARCHITECTURE.md` covers the processes and the decision path;
  `docs/OPERATIONS.md` is the runbook, including the reference build this software is tuned for.
- The web dashboard's API contract is `web/src/types.ts`. Change it there first.
- Prefer a small diff that a reviewer can verify over a large one that has to be trusted.
- `.claude/` holds the parts of this process that are not worth re-deriving. `/gate`, `/golden`,
  `/measure` and `/new-file` wrap the four steps that are easiest to get wrong, and two subagents are
  there to delegate to: a reviewer that knows the invariants and the layering, and a boundary checker
  for the layering on its own. `.claude/settings.json` runs the provenance check as soon as a commit
  lands, which is a better moment to find out than CI. None of it overrides what is written here —
  where the two disagree, this file and `CONTRIBUTING.md` are the rule.

## 7. If you are an agent opening a pull request

- Keep the change to one thing. A patch that fixes a bug and renames a module is two pull requests.
- Run `scripts/check.sh full` before you push. A red gate costs a round trip; the same failure
  locally costs a minute.
- Put `Generated-by:` in the pull request body as well as the commits. The gate reads both.
- If a test fails and you do not understand why, say so in the pull request rather than adjusting the
  test until it passes.

# SvanBot v11: agent brief

SvanBot plays 6-max no-limit hold'em on openpoker.ai with five bots under one owner. It learns from
every hand they play and changes how it plays only on evidence. It runs on CPU only, on one box
that it shares with the live fleet. Its language is in `GLOSSARY.md`: name things the way the
glossary does, and add a term there the moment a new one settles.

## Where v11 stands

v11 is written from scratch, on `main` (`docs/adr/0002-v11-from-scratch.md`). The work is
charted on GitHub as the map **V11-00** and its tickets **V11-01** to **V11-16**. Every change
starts from a ticket. v10 is reference only: read it with `git show archive/v10/final:<path>`, and
write the v11 version fresh, test-first.

`main` is v11 (operator, 2026-11-23). v10 is the branch `v10` and the tag `archive/v10/final`.
The live v10 fleet's checkout still fetches `main`, and since the two share no history, v10's
updater refuses to move onto it; `SVANBOT_UPDATE_BRANCH=v10` points it back at v10. Branch from
`main`, and open every PR with `--base main`. Name branches `v11-<nn>-<slug>` after the ticket, for example `v11-02-foundation`.

**The venue's docs come first.** <https://docs.openpoker.ai/llms-full.txt> is the most important
document in this project. The venue client, fair play, the export and the game rules are all
written against it. Read it, and check its revision line, before you touch any of them. Where it
disagrees with v10, it wins.

## The crate graph

`core` → `sim` → `strategy` → `learn` → `runtime` → `apps`, with one crate per layer under
`crates/<layer>/` (`docs/adr/0003-crate-graph.md`). A crate depends only on its own layer or lower
ones, and the four layers below `runtime` are pure. Two checks enforce it, and the gate's commit
tier runs both:
- `cargo clippy --workspace --all-targets -- -D warnings`: `clippy.toml` bans I/O and the clock
- `bun tools/layering.ts`: the edges and the third-party crates

## The gate

`mise.toml` pins every tool, and `tools/package.json` pins TypeScript. Set a clone up once with
`mise run setup`: it installs the tools and the TypeScript packages, and turns the git hooks on.

- `mise run commit` runs the fast checks: fmt, Biome, the 500-line limit on Rust files, the secret
  scan of staged changes, provenance, the layering check and clippy. The pre-commit hook runs it.
- `mise run full` adds nextest, the doc tests, `tsc`, the tool tests and the dashboard page's
  checks, then reports every `target/` in the clone, worktrees' included, against one disk
  budget (`tools/disk.ts`). Run it before you push.
- `mise run ci` runs cargo-deny (`deny.toml`).

The commit-msg hook refuses a message without a `Generated-by:` line. Cargo builds into
`target/dev` at low priority, on four jobs.

CI (`.github/workflows/check.yml`) runs the same tiers on every pull request into `main`: the
commit tier, the test tier and cargo-deny, side by side. A PR merges only when all three are green.

## Hooks and rules

Some rules below are held by a machine, in any Claude Code session (`.claude/settings.json`,
`tools/hook.ts`):
- Before a shell command, the hook refuses a push to `main`, a forced push, a build outside
  `target/dev`, and a release build.
- After an edit of a `Cargo.toml`, it runs the layering check.
- Before the agent stops, a changed tree must pass the gate's commit tier.

A refusal comes with its reason. Do what the reason says: do not look for a way round it.
`.claude/rules/` holds the rules of one part of the tree, loaded when you work there.

## The loop

1. **Claim the ticket:** add `agent-claimed`, working as `svanlabs[bot]`
   (`docs/agents/issue-tracker.md`).
2. **Agree the seams**, then work in **tracer bullets** with the `tdd` skill. Each slice is one test
   going red, then the least code that turns it green.
3. **Review** the branch with the `code-review` skill, inline, before you open the PR.
4. **Open the PR** with the `pr` skill. Merge it once the gate is green, the review has nothing
   blocking, and, for any behaviour change, a paired sim passes.

## Handing off

Claude Code, Codex, opencode and openclaude all work this map as `svanlabs[bot]`. Any session can
stop at any moment when its usage limit runs out, so work is left where the next agent can take it:

- **Push after every green slice.** Work that lives only on the box is lost to the next agent.
- **When you stop mid-ticket**, or feel the limit coming:
  1. Commit and push what you have.
  2. Write the note: `bun tools/handoff.ts <ticket> --done "…" --next "…" --traps "…" --by <tool/model>`.
  3. Post it with `gh issue comment <ticket> --body-file -`.
  4. Add the label `agent-handoff`, and keep `agent-claimed`.
- **To take a ticket over**, pick a ticket labelled `agent-handoff`. It is yours even though it is
  claimed:
  1. Remove `agent-handoff`.
  2. Read the newest handoff note.
  3. `git fetch origin && git switch <branch>`, and carry on from **Next**.

  A claimed ticket without the label still belongs to another session.

## The hard invariants

- **Poker logic is pure.** Only `runtime` and `apps` touch the network, the store or the clock.
  Everything below them is deterministic given its inputs, which is what keeps the sims reproducible.
  When work has a time limit, the caller passes the limit in as an input.
- **Decisions stay in-process** (`docs/adr/0001-decision-path-stays-in-process.md`).
- **Only legal actions reach the venue,** echoing the venue's hand and turn ids, inside the action
  clock.
- **Fair play is absolute:** two stablemates never share a table. A shared hand is an incident.
  Each bot's reads are its own, from its own store (`docs/adr/0004-reads-are-per-bot.md`).

## Our own code

v11 is the operator's own work, built from scratch (operator, 2026-11-16). Add no third-party
crate, npm package or premade framework: write what the work needs, in the layer that needs it.
A ticket that names one (proptest, cargo-fuzz, Next.js) is done with our own code. The crates
`runtime` had before the rule (rustls, tungstenite, ureq, rusqlite, serde_json, sha2) stay, since
a home-made TLS would be a risk: ask the operator before you add to that list or replace one.

## On this box

- Everything lives in one folder, `~/NewSvanBot` (operator, 2026-11-25, V11-55): the scripts the
  fleet runs, the cron line, the work. Keep no second checkout, and no scripts or tools anywhere
  else. The folder stays on `main`, moved only by `git pull --ff-only`, and is never built in or
  edited: the running supervisors read `scripts/`.
- Work in a worktree under `.claude/worktrees/`, named after the ticket, and build there with
  `CARGO_TARGET_DIR=target/dev`.
- Run one build at a time at low priority, with
  `nice -n 15 ionice -c3 cargo … -j4`. Concurrent builds have OOM-killed the fleet.
- The box runs v11 from `scripts/`, one supervisor (`svanbot-bot`, `scripts/bot.sh`) per bot,
  with no systemd, and `deploy/README.md` is the runbook. Agents run the fleet: start, stop and
  swap bots with `scripts/` and `svanbot-bot` (operator, 2026-11-25, V11-54). Never start a bot with
  a fleet bot's key while another connection plays it: the new connection takes over that bot's
  live session.
- Every bot runs the newest release (operator, 2026-11-25, V11-56): cron runs
  `scripts/auto-update.sh` every half hour. To hold an older build on purpose, for example after a
  `svanbot-release rollback`, `touch ~/.local/state/svanbot/auto-update.pause`; remove the file to
  resume.
- A test of a fleet script puts a refusing `systemctl` first on its PATH, and stubs every tool the
  script runs. A red run once ran the old `stop.sh` against the real fleet and stopped every bot.
- The repo is public. Keys, `.env` contents, opponent data and store contents stay out of it: out
  of code, logs, issues and PRs alike.

## Provenance

Every commit carries `Generated-by: <tool>/<model>` in its trailers. Every issue and PR body carries
the same line. `tools/provenance.ts` checks the commits, and being authored by a bot exempts none.

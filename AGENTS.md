# SvanBot v11: agent brief

SvanBot plays 6-max no-limit hold'em on openpoker.ai with five bots under one owner. It learns from
every hand they play and changes how it plays only on evidence. It runs on CPU only, on one box
that it shares with the live fleet. Its language is in `GLOSSARY.md`: name things the way the
glossary does, and add a term there the moment a new one settles.

## Where v11 stands

v11 is written from scratch on the branch `v11` (`docs/adr/0002-v11-from-scratch.md`). The work is
charted on GitHub as the map **V11-00** and its tickets **V11-01** to **V11-16**. Every change
starts from a ticket. v10 is reference only: read it with `git show archive/v10/final:<path>`, and
write the v11 version fresh, test-first.

`main` is v10, and the live fleet updates from it. Branch from `v11`, and open every PR with
`--base v11`. Name branches `v11-<nn>-<slug>` after the ticket, for example `v11-02-foundation`: git
cannot hold a `v11/…` branch beside the `v11` branch.

## The loop

1. **Claim the ticket:** assign yourself and add `agent-claimed`.
2. **Agree the seams**, then work in **tracer bullets** with the `tdd` skill. Each slice is one test
   going red, then the least code that turns it green.
3. **Review** the branch with the `code-review` skill, inline, before you open the PR.
4. **Open the PR** with the `pr` skill. Merge it once the gate is green, the review has nothing
   blocking, and, for any behaviour change, a paired sim passes.

## The hard invariants

- **Poker logic is pure.** Only `runtime` and `apps` touch the network, the store or the clock.
  Everything below them is deterministic given its inputs, which is what keeps the sims reproducible.
- **Decisions stay in-process** (`docs/adr/0001-decision-path-stays-in-process.md`).
- **Only legal actions reach the venue,** echoing the venue's hand and turn ids, inside the action
  clock.
- **Fair play is absolute:** two stablemates never share a table. A shared hand is an incident.
  Each bot's reads are its own.

## On this box

- Build into `target/dev`. The live fleet runs from `target/release`.
- Run one build at a time at low priority, with
  `nice -n 15 ionice -c3 cargo … -j4`. Concurrent builds have OOM-killed the fleet.
- The repo is public. Keys, `.env` contents, opponent data and store contents stay out of it: out
  of code, logs, issues and PRs alike.

## Provenance

Every commit carries `Generated-by: <tool>/<model>` in its trailers. Every issue and PR body carries
the same line.

# Changelog

v11 starts this changelog afresh. v10's history is at the tag `archive/v10/final`.

## 11.0.0 "The Deep Mass-ReWorkStructure": unreleased

A new codebase written from scratch (`docs/adr/0002-v11-from-scratch.md`), tracked as the map
V11-00.

- v10 archived. The code is at `archive/v10/final` and the 61 `archive/v10/*` branch tags. The store
  is a local archive that never goes to GitHub.
- Foundation: `GLOSSARY.md`, ADRs 0001–0002, the research notes behind the plan, the agent brief, and
  the Matt Pocock skills pinned to the repo.
- Workspace: six layers, `core` → `sim` → `strategy` → `learn` → `runtime` → `apps`, one crate per
  layer (ADR 0003). `clippy.toml` keeps I/O and the clock out of the pure layers, and
  `tools/layering.ts` checks the edges and the third-party crates.
- Gate: `mise run commit` runs fmt, Biome, the 500-line limit, the secret scan of staged changes,
  provenance, the layering check and clippy. `mise run full` adds nextest, the doc tests, `tsc`
  and the tool tests. Both build into `target/dev` at low priority. The hooks in `.githooks/` run
  the commit tier and check each commit message.
- Tools: `mise.toml` pins Rust 1.100.0, Bun 1.4.4, nextest 0.9.152 and Biome 2.6.7, and
  `tools/package.json` pins TypeScript 7.0.2 and `@types/bun` 1.4.4, each the latest stable
  release on 2026-11-15. `mise run setup` installs them all.
- CI: `.github/workflows/check.yml` runs the commit tier, the test tier and cargo-deny side by side
  on every pull request into `v11`, with the tools mise pins and our own crates cached. Every
  action is pinned to a commit.
- Disk: the dev profile keeps line tables only, and the full tier ends by reporting `target/`
  against a 9 GiB budget, failing over it.
- Cards: `svanbot-core` reads and writes the venue's two-character notation (`Ah`, `Ts`, `2c`),
  and a `CardSet` holds a board, a player's hole cards or what is left of the deck. With tests in
  the workspace, nextest now fails a run that finds none.
- Evaluator: the strength of the best five of five to seven cards, after OMPEval. Cards fold
  into a key by addition, so a board's key is made once and each player's two cards cost two adds
  and one table read. The tables are built in-process on first use. Checked against the published
  five-card counts (7,462 distinct strengths) and against a best-of-21 reference.
- Seeded generator: xoshiro256++ seeded through SplitMix64, written in `core`, with a stream per
  block index so the same seed gives the same draws on any machine and any thread count, and an
  unbiased `below(n)`. Checked against the reference implementation's published outputs.
- Rules of a hand: the blinds (heads-up the button posts the small blind), whose turn it is, the
  legal actions with raise-to ranges, short all-ins that reopen the betting only once they add up
  to a full raise, side pots, odd chips to the first winner left of the button, and uncalled chips
  returned. Checked by worked examples and 3,000 seeded random hands that must keep every chip.
- Equity: `svanbot-sim` enumerates every runout or samples seeded deals against weighted ranges,
  on 64 fixed blocks with a stream each. Shares count in whole sixtieths, so the same seed gives
  the same bits on any machine. Checked by counted outs, suit relabelling, and about 82% for aces
  against kings.

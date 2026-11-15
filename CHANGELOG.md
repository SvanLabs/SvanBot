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

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
- Gate: `bun tools/gate.ts commit` runs fmt, the 500-line limit, the secret scan of staged changes,
  provenance, the layering check and clippy. `bun tools/gate.ts full` adds the workspace tests and
  the tool tests. Both build into `target/dev` at low priority. The hooks in `.githooks/` run the
  commit tier and check each commit message.

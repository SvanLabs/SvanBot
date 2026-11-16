---
paths:
  - "tools/**"
  - ".githooks/**"
  - ".github/**"
  - "mise.toml"
---

# Tools, the gate and CI

- A tool is one TypeScript file run by Bun, with its test beside it (`x.ts`, `x.test.ts`). It
  exports the function that decides and keeps the I/O under `import.meta.main`, so the test calls
  the decision with plain values.
- A new check joins a tier in `tools/gate.ts`, and `tools/gate.test.ts` says which. The commit
  tier stays fast: it runs on every commit.
- `mise.toml` pins every tool to an exact version. A workflow action is pinned to a commit.
- Cargo runs through the gate's `cargo()` wrapper: low priority, four jobs, `target/dev`.
- The agent hooks are `tools/hook.ts`, wired in `.claude/settings.json`. A hook that cannot read
  its input lets the call through.

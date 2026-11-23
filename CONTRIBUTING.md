# Contributing

SvanBot is the operator's own work. A change comes from a ticket on the map
[V11-00](https://github.com/SvanLabs/SvanBot/issues/1040), and lands through a pull request into
`main` that the gate has passed. [`AGENTS.md`](AGENTS.md) is the full brief. This is the short one.

## The loop

1. **Start from a ticket.** Every change has one. Name the branch after it, `v11-<nn>-<slug>`,
   branch from `main`, and open the PR with `--base main`.
2. **Agree the seams, then work test-first.** A seam is the public interface the tests sit at.
   Each slice is one test going red, then the least code that turns it green.
3. **Review the branch before the PR.** A behaviour change also needs a paired sim that passes,
   and the PR quotes its line (`svanbot paired`, `svanbot propose`, `svanbot paired compare`).
4. **Merge** once CI is green and the review has nothing blocking.

## The rules a machine holds

- **The pure layers stay pure.** `core`, `sim`, `strategy` and `learn` touch no network, file,
  store or clock. A time limit comes in as an input (`clippy.toml`, `tools/layering.ts`).
- **Our own code.** No third-party crate, npm package or premade framework. What the work needs
  is written in the layer that needs it.
- **Fair play is absolute.** Two stablemates never share a table, and each bot's reads are its own.
- **Provenance.** Every commit, issue and PR carries a `Generated-by: <tool>/<model>` line, and the
  commit-msg hook refuses a message without one.
- **Rust files stay under 500 lines.** A file that grows past it is split.
- **Nothing private goes in.** The repo is public: keys, `.env` contents, opponent data and store
  contents stay out of the code, the logs, the issues and the PRs.

## Setting up a clone

```sh
mise run setup    # the pinned tools, the TypeScript packages, the git hooks
mise run commit   # the fast checks; the pre-commit hook runs them
mise run full     # adds the tests: run it before you push
```

Build into `target/dev`, one build at a time, at low priority:
`CARGO_TARGET_DIR=target/dev nice -n 15 ionice -c3 cargo … -j4`. The box is shared with the live
fleet, and concurrent builds have run it out of memory.

## Words

Name things the way [`GLOSSARY.md`](GLOSSARY.md) does, and add a term there the moment a new one
settles. A decision that will be argued about again gets an ADR in [`docs/adr/`](docs/adr/).

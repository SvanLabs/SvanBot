---
paths:
  - "crates/core/**"
  - "crates/sim/**"
  - "crates/strategy/**"
  - "crates/learn/**"
---

# The pure layers

These four crates are poker logic. Given the same inputs they give the same outputs, on any
machine and any number of threads (`docs/adr/0003-crate-graph.md`).

- No network, no file, no store, no clock, no entropy. `clippy.toml` bans them and
  `tools/layering.ts` rejects an `allow` of a ban under `src/`. A test may turn a ban off to read
  a fixture.
- A time limit or a work limit comes in as an input: iterations, samples, a seed. Randomness is
  `core::rng`, seeded by the caller, with a stream per block or per deal.
- No third-party crate. What a layer needs, it writes: the digest is FNV-1a in `strategy::params`,
  the generator is xoshiro256++ in `core::rng`.
- Sum in a fixed order. A result must not depend on how the work was cut across threads.
- A change to how the bot decides is a behaviour change: it merges only with a paired sim that
  passes (`svanbot paired`, `svanbot propose`), and the PR quotes the sim's line.
- Generative tests are seeded walks over `core::rng`, with the seed in the failure message.

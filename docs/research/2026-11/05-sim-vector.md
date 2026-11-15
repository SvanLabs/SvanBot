# 05 — Simulator, evaluator and vectorization (v11.0.0 research)

Date 2026-11-15. I only read the repo. Nothing was built or benchmarked, so every speedup below is an estimate taken from published numbers, not something measured on our code.

## What we have

- **Cards** (`crates/libs/cards`). A card is `suit*13+rank`, and a hand is a 52-bit `u64` mask where each suit is its own 13-bit lane. `NUM_COMBOS = 1326`, and the combo tables and masks are built at compile time.
- **Evaluator** (`cards/src/eval.rs`). It works on bit-planes: four suit words, flush detection by popcount, quads, trips and pairs from AND/OR, straights from four shift-ANDs and an LZCNT, and kickers by repeated leading-zeros. It uses no lookup table and is full of branches. The result is a `u32` value with the category in bits 20..24. It is checked exhaustively (7462 classes, and the 7-card category counts).
- **Equity** (`crates/libs/equity`):
  - `SharedDeals` is already **SoA**: `combos: Vec<u16>`, `hero_rank: Vec<u32>` and `opp_rank: Vec<u32>` (n×k), plus `prior` for exact deals. Deals are drawn once and each candidate re-weights them through importance ratios (common random numbers).
  - Heads-up flop/turn/river uses exact enumeration. Hero's rank is memoized per runout, which cuts a flop from about 990k evals to 1,081.
  - The sampler is an inverse CDF with a 1024-bucket guide and a fixed 8-wide compare window that has no branches.
  - `river_strengths` uses a 4-pass radix sort plus per-card removal counters, which is an O(n) showdown. That is the same idea postflop solvers use.
  - Flop and turn strengths are precomputed once per suit-canonical board (1,755 + 16,432 rows × 1326 `f32`, 96 MB), memory-mapped (`deps/mmap`), and read through a suit-permutation combo map.
- **Parallelism and determinism.** `new_parallel` draws one seed per chunk from the caller's RNG, and each chunk gets its own `SmallRng` (xoshiro256++, `deps/rng`, bit-exact with rand 0.10). Chunks are joined in chunk order. The exact path uses `par_chunks_mut` with fixed blocks. **Chunk count = logical cores**, so a result is reproducible on one machine but not across machines with different core counts.
- **SIMD.** There are no explicit intrinsics in any library crate, only a BMI2 `pdep` experiment in `bench/draw.rs`. The code relies on `.cargo/config.toml` `target-cpu=native` plus auto-vectorization. The sampler's comment assumes "two AVX2 compares".
- **`nn`.** A small CPU MLP (thousands of parameters, `Vec<f32>` layers, scalar loops). `engine`: rules, side pots, `expected_net`. `pack`: our own DEFLATE and CRC-32 (slicing-by-8).
- **Tests.** There are hand-written differential and reference tests (`deals/reference.rs`, `tracker/differential.rs`) and an exhaustive evaluator test. I found no `proptest` dependency and no `cargo-fuzz` target.

## This box

`nproc` = 6. `lscpu` reports **"QEMU Virtual CPU version 2.5+"** on KVM, and the flags stop at `sse4_2 popcnt aes`. There is **no AVX, AVX2, BMI1/2, LZCNT (abm), FMA or MOVBE**, and of course no AVX-512. So `target-cpu=native` here gives x86-64-v2 code: `leading_zeros` becomes BSR+CMOV, the sampler window takes four SSE2 compares, and auto-vectorization is capped at 128 bits.

The docs use the i7-4770K (Haswell, AVX2) as the reference, and `scripts/install.sh` would pick bundle **v2** on this guest. If the fleet runs in this VM, it gets none of the instruction set the docs describe. I cannot tell from inside whether this sandbox is the fleet host.

## State of the art (2025–2026)

**Hand evaluators**

| Evaluator | Method | Tables | Published rate (1 thread) |
|---|---|---|---|
| OMPEval (C++) | additive rank keys + perfect hash, separate flush path | ~200 kB | 775 M/s sequential, 262–272 M/s random, i7-3770K |
| b-inary `holdem-hand-evaluator` (Rust) | OMPEval-style keys + magic constants, ~10-line `evaluate()` | ~212 kB | ~1.2 G/s sequential, Ryzen 9 5950X |
| HenryRLee PokerHandEvaluator | perfect hash | ~100–144 kB | 56–60 M/s |
| ACE_eval | bit tricks | tiny | 72 M/s |
| Two Plus Two | state-machine LUT | 123 MB | fast sequential, cache-hostile random |

The trick that matters for us is in OMPEval and b-inary: a hand key is a **sum of per-card keys**. The board's partial key can be computed once, and then each combo, or each combo plus runout card, costs one add, one hash and one or two L1/L2 loads. Our `SharedDeals` has exactly this shape, with a fixed board and thousands of combos. "Sequential" numbers are 3× the "random" ones because incremental keys stay in cache, and our enumeration loops are sequential.

- OMPEval also caches equity enumeration by suit isomorphism. We already do that for strengths.
- The aya_poker crate ports OMPEval to Rust with compile-time perfect-hash tables.

Sources: <https://github.com/zekyll/OMPEval>, <https://github.com/b-inary/holdem-hand-evaluator>, <https://github.com/HenryRLee/PokerHandEvaluator>, <https://github.com/ashelly/ACE_eval>, <https://github.com/dtrifuno/aya-poker>

**SIMD in Rust, late 2026**

- `std::simd` (portable_simd, tracking issue #86656) is **still nightly-only**. Mask semantics are still unresolved, and the 1.98 stable toolchain we pin cannot use it.
- On stable, most `std::arch` intrinsics are **safe to call inside `#[target_feature]` functions** (target_feature 1.1 in 1.86, safe intrinsics in 1.87).
- Library choices:
  - `wide`: mature, fixed width, no multiversioning.
  - `pulp`: runtime dispatch built in, and it powers faer.
  - `macerator`: a pulp fork with more instruction sets.
  - `fearless_simd`: modelled on pulp, built for vello.
- **For us:** either keep `target-cpu=native` with auto-vectorization over SoA, or put the 2–3 hot kernels behind `#[target_feature(enable="avx2")]` with `is_x86_feature_detected!` dispatch. The second keeps portable bundles fast and needs no third-party dependency, which matters given the `deps/` policy.

Sources: <https://doc.rust-lang.org/std/simd/index.html>, <https://github.com/rust-lang/rust/issues/86656>, <https://blog.rust-lang.org/2025/05/15/Rust-1.87.0/>, <https://blog.rust-lang.org/2025/04/03/Rust-1.86.0/>, <https://shnatsel.medium.com/the-state-of-simd-in-rust-in-2025-32c263e5f53d>, <https://docs.rs/crate/pulp/latest>, <https://faer-rs.github.io/dev-simd.html>, <https://docs.rs/fearless_simd>

**SoA for 1326-combo vectors**

- What postflop solvers do: keep `[f32; 1326]` (or a live-filtered list) per player, sort by strength once per board, then do O(n) showdown sweeps with per-card blocker counters. postflop-solver compresses node storage to `u16` plus an `f32` scale, adds up in `f64`, and turns on rayon by default.
- We already do the sorted sweep (`river_strengths`). The remaining cost is in `SharedDeals::equity`:
  - an AoS walk of `chunks_exact(k)`;
  - a `ratio[c]` gather per deal and opponent;
  - an `f64` accumulator chain, which runs serially because the order is fixed for bit-exactness.

Source: <https://github.com/b-inary/postflop-solver>

**Deterministic parallel simulation**

- Reproducibility needs two things. Seeds must be tied to fixed-size work blocks, not to threads. Reductions must happen in a fixed order: indexed `par_chunks().collect()`, then a sequential or tree combine. A bare `par_iter().sum()` is not reproducible.
- salib-core bans `sum`, `reduce` and `fold` with clippy `disallowed_methods`, and reports about 5–10% cost.
- Our seeds are per chunk, but the **chunk count follows the hardware**, so the same seed produces different deals on a 6-core and an 8-core box.
- Robust-design practice:
  - property tests (proptest) for engine invariants: chip conservation, side-pot sums, legal-action closure;
  - differential tests of the evaluator against a naive 21-subset reference;
  - `cargo-fuzz`/libFuzzer on the tracker's frame parser and the `SVZ` decoder;
  - metamorphic tests: suit relabelling and seat rotation must leave equity unchanged.

Sources: <https://docs.rs/crate/salib-core/latest>, <https://docs.rs/rayon/latest/rayon/iter/trait.IndexedParallelIterator.html>, <https://rust-fuzz.github.io/book/cargo-fuzz.html>, <https://proptest-rs.github.io/proptest/>

**GPU**

- The published poker GPU numbers are about 300–400 M hands/s, claimed 30–50× over the author's own CPU code (mgavriliu/poker-probability). That is roughly what one modern core reaches with OMPEval-class sequential evaluation.
- Our live equity costs about 6 ms of a 95 ms p50 decision. Kernel launch plus a PCIe round trip (about 10–50 µs each, many per decision) eats much of that 6 ms.
- The architecture is CPU-only by decision, and this box has no GPU.
- **Verdict: no for live play.** It would only make sense for an offline learner or table-building farm, and the precomputed tables already remove most of that work.

Sources: <https://github.com/mgavriliu/poker-probability>, <https://naml.us/post/exact-poker-probabilities/>

## Gap vs SvanBot

1. The evaluator is branchy bit-twiddling with no incremental key. The best evaluators are about 1 G/s sequential with a 200 kB table, and our hot loops (exact deals, the MC opponent ranks, `river_strengths`) all fix the board and vary two cards, which is exactly the case additive keys make cheap.
2. This guest exposes x86-64-v2 only, so the AVX2/BMI2/LZCNT the code and docs assume is absent.
3. `SharedDeals::equity` scans scalar AoS with a gather and a serial `f64` chain. It is repeated per candidate.
4. Reproducibility is per machine, not global, because chunk count = cores.
5. There are no property or fuzz harnesses. The tests are hand-written reference tests, which are good but not generative.
6. No explicit SIMD dispatch, so the portable v2/v3 bundles depend on whatever the compiler auto-vectorizes.

## Options, ranked

1. **Expose the host CPU to the VM (`-cpu host` / `host-passthrough`) and rebuild native.** This is a configuration change only. Expected: **1.1–1.4×** on eval and sampling (LZCNT/BMI, 256-bit compares), more where auto-vectorization kicks in. First, confirm this guest is the fleet host.
2. **OMPEval/b-inary-style evaluator in `sv10-cards`.** That means additive card keys, a perfect-hash rank table (about 200 kB, built in a `const` or by `tables`, checked bit-exact against the current evaluator on all 133M hands), and an incremental API `eval_with(board_key, combo_key)`. It changes `HandValue`, so either map it back with a 7462-entry remap or regenerate goldens. Expected: **3–6× on evaluation**, which is about **1.5–2.5×** on `SharedDeals::new` and exact flop and turn, and faster `tables` rebuilds.
3. **Vectorize `SharedDeals::equity`.**
   - Precompute a per-deal `w` column per candidate subset as `f32`/`f64` SoA.
   - Make win/tie a branch-free compare on `u32` lanes.
   - Accumulate in 4–8 fixed lanes and combine them in a fixed order. This stays deterministic but changes the floats, so it needs a golden decision.
   - Use an AVX2 path via `#[target_feature]` with a scalar fallback.

   Expected: **2–4× on the per-candidate scoring loop**, about **1.2–1.5×** on a live decision's equity phase.
4. **Fix the chunk count (for example 64 blocks, with seeds tied to block index) regardless of cores**, and ban `par_iter().sum/reduce` with clippy. Expected speed **~1.0×** (±5%), but the same seed then gives bit-identical deals on every machine. That makes analyst re-solves and goldens portable.
5. **Generative tests:**
   - proptest for engine invariants and suit/seat metamorphic equity;
   - cargo-fuzz for the tracker and `SVZ`;
   - a differential evaluator test against a 21-subset reference.

   No speedup (**1.0×**). It is the safety net options 2–3 need, since both rewrite bit-exact hot code.

Not recommended: GPU (latency, no hardware, the CPU-only invariant); nightly `std::simd` (we pin stable); a 123 MB Two Plus Two table (cache-hostile next to the 96 MB strength mmap).

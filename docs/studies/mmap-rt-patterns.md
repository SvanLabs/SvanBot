# mmap and async/threading runtime patterns for the SvanBot workload

Generated-by: opencode/muse-spark-1.3-contributor-free

## Question

What are better `mmap` usage patterns and better async/threading runtime designs
for our workload, grounded in primary sources — subject to the hard constraints:
CPU-only, Linux x86_64 reference build (i7-4770K, THP `always`, swappiness 5),
decision p99 ~130 ms isolated / ~0.6 s under contention with an 8 s hard cap
(`DECISION_CAP` in `crates/apps/bot/src/client/decide.rs`), vendored mini-crates
with no third-party deps except `libc` under `crates/deps/` (notably `sv10-mmap`,
read-only `MAP_SHARED` maps of the 96 MB board-strength tables, see
`crates/deps/mmap/src/lib.rs`), async work as tokio `spawn_blocking` + `timeout`
in `crates/apps/bot/src/client/decide.rs`, and rayon pools sized in
`crates/libs/policy/src/hardware.rs`? See also `docs/OPERATIONS.md` (reference
build, benchmarks, Linux settings) and `crates/deps/rt/src/lib.rs` (`sv10-rt` is
env/uuid/statvfs helpers, not an async runtime).

Every claim below links the primary source that owns it. Prices are given as
(code size, dependency weight, which measured number moves).

## Verified local facts (read before recommending)

- `crates/deps/mmap/src/lib.rs`: `PROT_READ`, `MAP_SHARED`, whole-file map; fd
  may be closed after mapping. Safety contract: tables are replaced only by
  write-new-file + rename-over (`Table::write`); nothing truncates in place.
  Empty files map to an empty slice with no mapping.
- `crates/apps/bot/src/client/decide.rs`: each `your_turn` runs `decide_with`
  inside `tokio::task::spawn_blocking` with `tokio::time::timeout(DECISION_CAP)`
  (8 s). The job snapshots (`shared2.models.read().clone()`) so an overrunning
  decision never holds the models lock past the cap. Decisions take ~0.1–0.4 s.
- `crates/libs/policy/src/hardware.rs`: pool/pool-adjacent sizing uses
  `std::thread::available_parallelism` (cgroup-aware) plus `/proc/cpuinfo` and a
  ~2 ms equity benchmark; `live_deal_chunks = learner_threads`.
- `docs/OPERATIONS.md`: THP `always`, swappiness 5 left as is; learner runs at
  `nice 15` / idle I/O; release builds use 256 codegen units, no LTO; paired
  `scripts/bench-ab.py` is the required measurement method; live decision p50 ~68 ms,
  p95 ~190 ms, p99 ~211 ms in the Phase 1 baseline.
- Workspace `Cargo.toml`: tokio 1.53.1 (`full`), rayon 1.12.0, parking_lot
  0.12.5, libc 0.2. Already on parking_lot locks; tokio itself depends on
  parking_lot and mio.
- `crates/libs/equity/src/tables.rs`: tables load once per process via
  `OnceLock`, with a plain file read (deliberately no rayon inside the lazy
  initializer); lookups are per-board canonical-key random access
  (`Table::lookup`), not sequential scans.
- Store: SQLite with 10 s busy budget and retry discipline
  (`crates/libs/store/src/store/slow.rs`, `LOCKED_WRITE_WAITS` in `crates/apps/bot/src/client/decide.rs`);
  backups note the live DB runs `synchronous=FULL`.

## Ranked recommendations

### 1. Prefetch + RANDOM advice on the table mappings (mmap)

Access is random per-board lookup into 96 MB shared read-only maps, faulted in
on demand and shared across fleet/learner/analyst/tools via the page cache.
Two small additions to `sv10-mmap`, both pure `libc`, both consistent with the
current design:

- (a) `MADV_RANDOM` after mapping. `madvise(2)` defines `MADV_RANDOM` as
  "expect page references in random order (hence, read ahead may be less useful
  than normally)", versus `MADV_SEQUENTIAL`, which the kernel answers with
  aggressive readahead and early reclaim
  (https://man7.org/linux/man-pages/man2/madvise.2.html). Random board lookups
  are exactly the RANDOM case; default readahead on each fault pulls
  surrounding pages that may never be touched, wasting I/O under contention.
  Note: do NOT use `MADV_SEQUENTIAL` here — the "sequential table scans" framing
  in the research question does not match `Table::lookup`, which is keyed
  random access.
- (b) `MADV_WILLNEED` (or `MADV_POPULATE_READ` where available) once at table
  load / process startup, off the decision path. `MADV_WILLNEED` means "expect
  access in the near future (hence, it might be a good idea to read some pages
  ahead)" (https://man7.org/linux/man-pages/man2/madvise.2.html). This moves
  cold-start major faults out of the first decisions: `docs/OPERATIONS.md`
  records "cold flop strengths 2.40 ms" as a real cold cost. `MAP_POPULATE` at
  `mmap` time is the inferior variant: `mmap(2)` states the call "doesn't fail
  if the mapping cannot be populated", i.e. errors are hidden
  (https://man7.org/linux/man-pages/man2/mmap.2.html), while
  `MADV_POPULATE_READ` (since Linux 5.14, present in any stock Ubuntu LTS
  kernel ≥ 5.15) "does not hide errors, can be applied to (parts of) existing
  mappings and will always populate (prefault) page tables readable"
  (https://man7.org/linux/man-pages/man2/madvise.2.html). Prefer the madvise
  form with a fallback to `WILLNEED` so behavior is identical on older kernels.

Price: ~20–30 lines in `sv10-mmap` (+ a `madvise` wrapper over `libc`, the
crate's stated purpose already includes `madvise(2)`); zero new dependencies.
Moves: cold-start / first-decision p99 and post-restart p99; steady-state p50
unchanged. Measure with `bench live` before/after on a cold page cache
(`evict_cache`-style drop via `POSIX_FADV_DONTNEED`, cf. `sv10_rt::evict_cache`
in `crates/deps/rt/src/lib.rs`).

### 2. Bound concurrent `spawn_blocking` decisions with a semaphore sized from the hardware profile (runtime)

Today every `your_turn` spawns one unbounded `spawn_blocking` job; with up to
five bots plus the learner's rayon pool on 4 cores / 8 threads, CPU-bound
searches queue behind each other and p99 stretches ~130 ms → ~0.6 s under
contention. Tokio's own `spawn_blocking` documentation calls this out
explicitly: "When you run CPU-bound code using spawn_blocking, you should keep
this large upper limit in mind. When running many CPU-bound computations, a
semaphore or some other synchronization primitive should be used to limit the
number of computations executed in parallel. Specialized CPU-bound executors,
such as rayon, may also be a good fit."
(https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html). The blocking
pool default limit is 512 and idle threads exit only after `thread_keep_alive`
(10 s default)
(https://docs.rs/tokio/latest/tokio/runtime/struct.Builder.html) — i.e. today
there is effectively no backpressure on concurrent 1.6M-sample searches.

Recommendation: wrap the decision `spawn_blocking` in a tokio `Semaphore`
sized from the existing `HardwareProfile` (`live_deal_chunks` /
`learner_threads` in `crates/libs/policy/src/hardware.rs`, derived from
`available_parallelism`), so at most N searches compute at once and the rest
wait *before* consuming a blocking thread. The 8 s `timeout` in `crates/apps/bot/src/client/decide.rs`
already bounds the wait + compute envelope, so semaphore queuing cannot break
the hard cap — it converts contention stretch into earlier, cheaper queue time
with the same safe-action fallback. Alternative within the same rank: run the
decision compute on the already-sized rayon global pool instead of
`spawn_blocking` (rayon's default pool size is the logical CPU count unless
`num_threads`/`RAYON_NUM_THREADS` overrides it:
https://docs.rs/rayon/latest/rayon/struct.ThreadPoolBuilder.html); but that
mixes live-latency work with learner batch work in one pool, so the semaphore
is the smaller, more isolated change.

Price: ~15 lines in `crates/apps/bot/src/client/decide.rs` (one `Semaphore`
in `Shared`, `acquire` around the job); zero new dependencies. Moves:
contention p95/p99 toward isolated numbers; p50 roughly unchanged. Measure
with `bench live` while the learner runs (the runbook's contention condition).

### 3. Confirm and keep the rename-over replacement discipline (mmap correctness)

The current discipline — write a complete new file, `fsync`, rename over, and
never truncate a mapped file in place — is the right one, confirmed by two
independent sources. `mmap(2)`: "Use of a mapped region can result in these
signals: SIGBUS — attempted access to a page of the buffer that lies beyond
the end of the mapped file" and "the effect of changing the size of the
underlying file of a mapping on the pages that correspond to added or removed
regions of the file is unspecified"
(https://man7.org/linux/man-pages/man2/mmap.2.html). I.e. shrinking/truncating
under a live mapping is a SIGBUS (or worse, unspecified) hazard, while
rename-over leaves existing mappings pinned to the old inode — which is
exactly what `Mmap`'s doc comment and the `replacing_by_rename_leaves_the_mapping_intact`
test assert. The atomic-write side is already correct too: `sv10-rt`'s
`write_atomic` + `sync_dir` in `crates/deps/rt/src/lib.rs` matches the
durability reasoning (rename needs a directory fsync to survive power loss).
No code change; keep this as the documented invariant and keep rejecting any
future "update tables in place" proposal on these grounds.

Price: zero. Moves: no crash class ever materializes (SIGBUS in a decision
thread = lost hand at best).

## Explicit non-recommendations (with reasons)

- **`mlock` / `MAP_LOCKED` the 96 MB tables.** `mmap(2)` warns `MAP_LOCKED`'s
  populate semantic "is not as strong as mlock(2)" and suggests `mmap` +
  `mlock` only "when major faults are not acceptable"
  (https://man7.org/linux/man-pages/man2/mmap.2.html). Here major faults *are*
  acceptable after warmup (recommendation 1 handles the cold edge), and the
  crate's whole design point is "paged in on demand and dropped under memory
  pressure without swap" (`crates/deps/mmap/src/lib.rs`). Locking 96 MB ×
  several processes works against the host's swappiness-5 / THP-`always`
  tuning in `docs/OPERATIONS.md` and risks pressuring the fleet's decision
  memory under contention. Cost without a measured need; revisit only if
  warm p99 still shows major-fault stalls (check via `/proc/<pid>/smaps`
  or `mincore(2)` — `mmap(2)` itself points at `mincore` for residency checks).
- **Explicit `MADV_HUGEPAGE`.** With THP mode `always` on the reference host
  (`docs/OPERATIONS.md`), the kernel already collapses eligible ranges; and
  `madvise(2)` states "most common kernels configurations provide
  MADV_HUGEPAGE-style behavior by default, and thus MADV_HUGEPAGE is normally
  not necessary"
  (https://man7.org/linux/man-pages/man2/madvise.2.html). It is also easy to
  waste memory on sparse access ("a 2 MB mapping that only ever accesses 1
  byte will result in 2 MB of wired memory"). Zero-line change wins: do
  nothing. (Also out of reach: `MADV_COLLAPSE` needs Linux 6.1 — fine on
  24.04 but not a lever worth pulling for random 4 KB-granular lookups.)
- **`MAP_POPULATE` at map time.** Same error-hiding objection as in
  recommendation 1 (`mmap(2)`: the call doesn't fail when population fails),
  plus it stalls every process startup — including every test binary — on a
  full 96 MB read. The madvise-after-open form dominates it. Ruled out.
- **Thread-per-bot dedicated decision threads.** Tokio's guidance draws the
  line clearly: "Use spawn_blocking for short-lived blocking operations; use
  dedicated threads for long-lived or persistent blocking workloads"
  (https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html). Decisions
  are ~0.1–0.4 s short-lived jobs; the current `spawn_blocking` + snapshot +
  8 s timeout design is the idiomatic shape, and dedicated threads would add 5
  mostly-idle stacks with no elasticity gain. Keep `spawn_blocking`; add the
  semaphore from recommendation 2 instead.
- **Replacing the sleep/timeout timer usage with a hand-rolled loop.**
  `LOCKED_WRITE_WAITS` sleeps and the 8 s decision `timeout` ride tokio's
  hierarchical timing-wheel timer: six levels × 64 slots, 1 ms lowest
  granularity, all of create/cancel/fire constant-time
  (https://tokio.rs/blog/2018-03-timers). A handful of outstanding timers is
  noise; a sleep-loop would be strictly worse (a parked worker + repeated
  wakeups). Nothing to change.
- **io_uring runtimes (tokio `enable_io_uring`, tokio-uring, monoio).** Tokio
  does expose an io_uring driver (`enable_io_uring`, crate feature `io-uring`:
  https://docs.rs/tokio/latest/tokio/runtime/struct.Builder.html), and the
  multi-thread scheduler is a work-stealing pool defaulting to one worker per
  core (https://docs.rs/tokio/latest/tokio/runtime/index.html). But io_uring
  buys batched-syscall throughput for high-IOPS server workloads; this fleet
  holds a handful of WebSocket client connections plus SQLite, and every
  measured bottleneck is CPU (equity sampling) and lock contention (SQLite
  writer, models lock) — neither is an I/O-submission bottleneck. It would add
  an unstable feature flag and kernel-version sensitivity for no measured
  number. Ruled out while the workload stays WS + SQLite + compute.
- **Switching parking_lot locks back to `std`.** Already on parking_lot
  0.12.5 (workspace `Cargo.toml`), which is also what tokio itself uses
  internally. The decision path already follows the correct high-contention
  pattern — snapshot under a short read lock (`models.read().clone()`) and
  compute outside it (`crates/apps/bot/src/client/decide.rs`) — so lock *hold time*, not lock
  implementation, is the lever, and it has already been pulled. A mutex-swap
  is churn with no measured number behind it.
- **SQLite `mmap_size` / memory-mapped I/O for the analytics corpus.** The fleet's
  discipline is read-only access (`mode=ro`, WAL-aware read transactions per
  `docs/OPERATIONS.md`), and SQLite documents WAL's own tradeoffs precisely:
  readers don't block writers but read performance "falls off with increasing
  WAL file size", hence regular checkpoints; the default auto-checkpoint is
  1000 pages (https://www.sqlite.org/wal.html). The live pain here is writer
  contention (10 s busy budget, retry queue), which the WAL + busy-handler +
  retry design already addresses. Turning on SQLite's own file mmaping adds a
  second paging layer over the same pages for the analytics corpus with no
  identified fault cost. Ruled out absent a measured read-stall profile.
- **Growing rayon pools to "logical cores × N" or oversubscribing for
  latency.** Rayon's default is already the logical CPU count
  (https://docs.rs/rayon/latest/rayon/struct.ThreadPoolBuilder.html), and
  `crates/libs/policy/src/hardware.rs` sizes from `available_parallelism`, which respects cgroup
  limits where a raw CPU count would not. More threads than cores on a
  compute-bound Monte Carlo workload adds stealing/scheduling overhead without
  adding throughput — the Phase 1 baseline in `docs/OPERATIONS.md` shows the
  box already at a ~1.6 load factor under contention. If anything the direction
  is fewer, reserved cores (recommendation 2), not more threads.

## Suggested measurement protocol (per AGENTS.md invariants)

Every performance change carries a same-machine measurement against the
previous commit via `scripts/bench-ab.py` (paired ratio with 95% t-interval;
gain counts only when the interval excludes 1), and behavior-affecting
changes need a paired `sim` run. Concretely: recommendation 1 → `bench live`
cold-cache repeats + `bench micro` guard; recommendation 2 → `bench live`
with the learner running (the contention condition), plus the `sim paired`
identity check. Reference numbers to beat: live p50 67.6 ms / p95 190.0 ms /
p99 211.0 ms post-pass baseline (`docs/OPERATIONS.md` Phase 1 table).

## Primary sources

- `mmap(2)` — MAP_SHARED/MAP_POPULATE/MAP_LOCKED semantics, SIGBUS on access
  past end of file, mincore reference:
  https://man7.org/linux/man-pages/man2/mmap.2.html
- `madvise(2)` — RANDOM/SEQUENTIAL/WILLNEED, HUGEPAGE default-on,
  POPULATE_READ/WRITE (5.14+) vs MAP_POPULATE error hiding, COLLAPSE (6.1):
  https://man7.org/linux/man-pages/man2/madvise.2.html
- tokio `Builder` — worker_threads default = cores, max_blocking_threads
  default 512 + queue behavior, thread_keep_alive, enable_io_uring:
  https://docs.rs/tokio/latest/tokio/runtime/struct.Builder.html
- tokio `spawn_blocking` — CPU-bound guidance (semaphore or rayon), no-abort
  once started, short-lived vs dedicated threads:
  https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html
- tokio runtime module — multi-thread scheduler is work-stealing, one worker
  per core by default:
  https://docs.rs/tokio/latest/tokio/runtime/index.html
- tokio timer design — 6-level × 64-slot hierarchical wheel, O(1) ops:
  https://tokio.rs/blog/2018-03-timers
- rayon `ThreadPoolBuilder::num_threads` — default = RAYON_NUM_THREADS or
  logical CPUs:
  https://docs.rs/rayon/latest/rayon/struct.ThreadPoolBuilder.html
- SQLite WAL — readers/writers concurrency, 1000-page auto-checkpoint,
  read-performance vs WAL-size tradeoff:
  https://www.sqlite.org/wal.html

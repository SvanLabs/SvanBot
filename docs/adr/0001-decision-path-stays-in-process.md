# Keep decision work in-process; split only behind a measured gate

The decision path's 8 s `DECISION_CAP` already bounds one turn's exposure — an overrun takes the
legalized safe action and the turn is answered — and what a per-decision worker process would buy is
killability of that overrun, which measured 0 `decision_timeouts` in 63,510 live decisions over 24
hours (2026-10-03). Against that, every split cost is concrete today: the only turn-path IPC channel
that exists is store polling, each process keeps its own model/NN copies, and SQLite's single write
lock — the current cross-process bus — measurably loses rows (`crates/libs/store/src/store/slow.rs`:
38 audits, 17 decisions and 6 hand rows in ten days). Decision work therefore stays in-process, and
a split is reconsidered only when the gate below trips, in this order: size the fleet process's own
rayon pool and stop double-counting `deal_chunks` against learner threads; land cooperative
abandonment and tighter budgets (#720); then measure.

## Status

accepted (2026-10-03, #719).

## The gate

All three metrics come from instruments that exist; the thresholds are deliberately coarse tripwires,
retunable by the operator without reopening the decision.

- **Overruns**: `decision_timeouts` (surfaced by `/api/compute`) — 0 today — at a sustained ≥1 % of
  decisions over a rolling 7 days.
- **Near-cap tail**: share of `decisions.latency_ms` above 4 s (half the cap) — one 7.3 s event in
  24 h today — at ≥0.1 % over 7 days.
- **Contention attribution**: live p99 more than 2× the frozen bench `live`-suite p99 on the same
  box during learner-active hours (the bench is sequential and idle; divergence is contention, not
  model cost).

If the gate trips, the pilot is one bot in a split-mode worker — the mode already exists
(`docs/ARCHITECTURE.md`, `SVANBOT_FLEET=split`), so nothing is re-implemented: same five bots, and
the pilot's tail and CPU share are compared against the in-process bots for a week. No turn-path
split without those numbers.

## Considered options

- **Per-decision worker processes now** (killable overruns, fault isolation): rejected until the
  gate. The missing IPC channel, duplicated models and an extra writer per turn on an already-lossy
  lock outweigh an overrun problem measured at zero; the cap already bounds the damage.
- **cpuset/cgroup partitioning now**: rejected. No measured contention between the fleet, learner
  and analyst pools beyond the existing `nice`/`ionice`/`oom` handling, and partitioning trades
  total search throughput for isolation the numbers have not earned.
- **Do nothing**: rejected. The over-parallelization (`live_deal_chunks` is set to `learner_threads`
  in `crates/libs/policy/src/hardware.rs`) and the uncancellable overrun in
  `crates/apps/bot/src/client/decide.rs` are real — they are just not split-shaped, and they are the
  first two steps.

## Deliberately not split

- The decision path stays in the fleet process; there are no decision workers.
- The fleet stays one all-in-one process by default; split mode remains built, documented and off.
- Learner, analyst, calibrate and monitor stay separate processes, unchanged.
- No cpuset or thread affinity is introduced; per-box sizing stays the `hardware.rs` derivation.

## Consequences

- An abandoned overrun stays uncancellable until a split; its blast radius remains CPU and rayon-pool
  contention, bounded per turn by the cap, and invisible to `decisions.latency_ms` (a timed-out turn
  writes no row) — the gate's first metric exists for exactly that blind spot.
- Any future split proposal without the gate's numbers is refused by this ADR.
- Terms are `docs/ARCHITECTURE.md`'s (head, worker, fleet process); no new vocabulary.

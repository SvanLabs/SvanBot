# Bound imported neural evidence without dropping the incumbent

Generated-by: codex/gpt-6

[The chronology repair](https://github.com/SvanLabs/SvanBot/issues/907) excludes imported hands
unless their completion predates a verified start for every selected live hand. The prior loader
called its newest 60,000 summaries past-season evidence regardless of their times. A read-only
census found 13,767 imported starts later than the first live completion, including five later
than the validation frontier. This proves future profile evidence, not duplicated validation labels
(the census found zero directly overlapping validation hand IDs).

The shared neural/residual loader now preserves live identity and completion metadata and checks
export starts and decoded completion times. Missing live starts select a cold-profile baseline;
unknown export completion times cannot supply warm-up. The current operator snapshot selects no
history warm-up because it cannot verify starts for every live hand. Live training continues.
The loader logs missing boundaries, counts malformed exports separately from valid exclusions,
and warns when a database error forces cold profiles.
Networks under the old contract cannot warm-start or be reused as fresh training results; the
new training contract is bounded-history-before-hand-v2.

The offline experiment uses a separate frozen copy captured 2026-10-07 21:09:00 UTC, with 194,633
stored hand rows. Treatment rows are excluded through the production loader. Training uses cycle
907 (fresh initialization seed 918), the production 39-48-24-3 network and ten epochs. The repaired
loader yields 1,318,068 training decisions and 246,304 held-out decisions. The candidate passes the
unchanged predictive threshold: log-loss 0.665357 versus statistical baseline 0.754367. These scores
are measured under the repaired chronology, not compared with the old artifact's contaminated
validation score.

Paired complete-hand simulations hold stored champion parameters fixed (2,500 samples, one deal
chunk), use 16 profile clones from the frozen models and 256 observed six-seat stack layouts, and
compare 32 tables of 1,500 hands each. All arms settle through the engine with the same deals.
The incumbent and candidate arms actually load their respective networks. The fallback arm has
no network. Per-opponent live corrections are absent, as in learner simulations.

| Challenger minus stored incumbent | Seed | Hands | bb/100 | 95% interval |
|---|---:|---:|---:|---|
| Clean network | 90711 | 48,000 | -1.15 | [-12.60, +10.30] |
| Clean network | 90713 | 48,000 | +0.96 | [-9.62, +11.55] |
| No network | 90711 | 48,000 | -10.35 | [-37.30, +16.60] |
| No network | 90713 | 48,000 | -22.31 | [-44.34, -0.28] |

[Derived results, snapshot hashes and exact parameters](neural-history-paired.json) retain both
seeds, result sums and candidate/model identities. Raw hands, opponent identities and weights stay
in the operator's frozen experiment. These are fixed diagnostic comparisons, not sequential learned
parameter confirmation and not proof of profit improvement. The clean candidate changes decisions
and neither comparison demonstrates poker harm; production still runs its own approval gate.

The migration therefore keeps an already active, paired-approved v1 incumbent serving while a
clean candidate earns approval, as it already does between ordinary neural refreshes. Unapproved,
predictively failed candidates stay outside the live slot during this migration. Ineligible,
unknown-contract or malformed old artifacts remain unexposed. A v1 incumbent and its matching
residuals are retained as existing play, never accepted as new chronological evidence. Residual
refitting requires the new contract; replacing the network invalidates old residuals by network
identity before refitting. This avoids a forced fallback that the second seed found harmful.
No gate threshold changes, manual network installation or learned-parameter promotion is involved.

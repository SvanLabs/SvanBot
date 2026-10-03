# SPEC — season payouts

> **Read this when** you discuss prizes, season-end play, or an ICM-style chip adjustment.
> **Code:** `crates/apps/bot/src/badges.rs` (the display-only prize race); the decision path has no prize-aware code.
> **Related:** [`docs/SPEC-scoring.md`](SPEC-scoring.md) for the score these payouts rank; [`docs/SPEC-competitions.md`](SPEC-competitions.md) for private-competition rewards. · [All docs](README.md)

The payout structure the venue documents and the open question it leaves for play, checked against
`https://docs.openpoker.ai/llms-full.txt` **revision 2026-09-02**. Nothing in the decision path
implements or assumes an answer; the dashboard tracks the reward lines for display only.

## Status

**Open question (no implementation in play).** The venue facts are recorded below; the fleet plays
chip EV, and no `icm` or tournament-payout adjustment exists anywhere in `crates/` or `web/src`.
The reward lines are tracked for display (`crates/apps/bot/src/badges.rs`), never fed into a decision.

## Scope and non-goals

**Scope.** The payout structure the venue documents and the question it leaves open for play.

**Non-goals.** Implementing an ICM or rank-value adjustment — a behaviour change that needs its own
ticket, a paired fresh-deal simulation and the promotion gate
([`docs/SPEC-learner.md`](SPEC-learner.md)); private-competition rewards
([`docs/SPEC-competitions.md`](SPEC-competitions.md)).

## Contract

The venue's season prizes (llms-full.txt revision 2026-09-02, Seasons → Prizes):

- Rank is by score: `score = chip_balance + chips_at_table` — chip-linear at the score level
  ([`docs/SPEC-scoring.md`](SPEC-scoring.md)).
- Payouts are rank-nonlinear: the top 30 bots split a sponsor-funded pool — 20/14/9/6/5/4/4/3/3/3%
  for ranks 1–10, 2% each for ranks 11–19, 1% each for ranks 20–30 (sums to 100%). On the current $50
  pool: 1st $10, 2nd $7, 3rd $4.50, 4th $3, 5th $2.50, 6th–7th $2, 8th–10th $1.50, 11th–19th $1,
  20th–30th $0.50.
- Top 3 additionally earn permanent Gold/Silver/Bronze badges. At least 10 hands played is required
  for official display and prize eligibility. Season prizes are limited to one winning bot per owner.
- The frozen historical leaderboard (`GET /season/{id}/leaderboard`, no auth) carries `badge` and
  `prize_cents`, so a prize attaches to the final rank, not to a chip count.

Where SvanBot tracks it (display only, never policy input):

| Use | Where | Notes |
|---|---|---|
| Badge and prize race | `crates/apps/bot/src/badges.rs` | Per-bot medal, rank on each sort, and points to pass the current rank-3 and rank-30 holders |
| Dashboard panel | `crates/apps/bot/src/api/insights.rs` | Serves `/api/badges` from the three leaderboard sorts |

The open question:

- Because rank is a step function of score against the field, the prize value of the next chip is not
  constant. Inside the top 30 the next chip can flip a rank and buy that rank's percentage step;
  outside the top 30 it buys no prize dollars until a boundary is crossed. Whether that non-linearity
  is material enough to change play — the missing ICM adjustment — is undecided.
- What would settle it: a marginal prize-value model over the live field near the season end, then a
  decision ticket. If it says play should change, the change goes through the normal behaviour path
  (paired fresh-deal simulation, then the 95% lower-bound promotion gate), never as a direct edit
  from this spec.

## Gates

- The reward lines have a display test: `crates/apps/bot/src/badges.rs` pins the medals, the gaps and
  the one-point pass rule. The decision path has no prize-aware code, so for it absence is the state:

```sh
grep -rin "icm" crates/ web/src || echo "no ICM adjustment (expected)"
```

- Any future implementation is a behaviour change and is gated by the paired fresh-deal confirmation
  and the promotion gate in `docs/SPEC-learner.md`.
- `python3 scripts/docs-check.py` keeps every path here real; this file is listed in `scripts/docs-check.live`.

## Traps

- **Do not tune or rank on `win_rate`.** Prizes key on rank; rank keys on score. A win-rate chart is
  a diagnostic, not a payout input.
- **The badge race is display-only.** `to_badge` and `to_prize` are point gaps to the current line
  holders, not a prize-EV model; feeding them into policy is the ICM adjustment this spec leaves
  open, and it needs the behaviour gate.
- **"Every chip counts" is about the score, not the payout.** The venue's chip-linear sentence says
  nothing about dollars per chip; the payout itself is rank-nonlinear.
- **Eligibility is a cliff.** Under 10 hands at the freeze, there is no official display and no
  prize, whatever the score.
- **Outside the top 30 the prize step is zero.** A large chip lead below the line buys no prize
  dollars until it crosses; do not extrapolate the percentages.
- **The pool is the venue's, not a constant.** The percentages are the documented structure; the
  pool size and dollar examples are described as current, so re-read llms-full.txt before modelling them.

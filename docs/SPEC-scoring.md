# SPEC — season scoring and the leaderboard

> **Read this when** you read, display or compare a season score, rank or leaderboard row.
> **Code:** `crates/apps/bot/src/client/rest.rs`, `crates/apps/bot/src/api/insights/leaderboard.rs`, `crates/apps/bot/src/api/insights.rs`, `crates/apps/bot/src/experiment.rs`, `crates/apps/bot/src/reputation.rs`.
> **Related:** [`docs/SPEC-protocol.md`](SPEC-protocol.md) for the REST surface these reads use; [`docs/SPEC-payouts.md`](SPEC-payouts.md) for what rank pays. · [All docs](README.md)

The public season score and leaderboard as the venue documents them and as SvanBot reads them, checked
against `https://docs.openpoker.ai/llms-full.txt` **revision 2026-09-02**. SvanBot never computes a
score: it reads the venue's integer and the two balances behind it.

## Status

**Supported (read-only).** The client reads `/season/me` and the leaderboard, and passes the venue's
`min_hands` qualifier on every prize-facing read; no code adds, transforms or recomputes a score.

## Scope and non-goals

**Scope.** What the season score is, which entries qualify for the official board and prizes, and
where SvanBot reads both.

**Non-goals.** Private-competition scoring, which uses a different formula
([`docs/SPEC-competitions.md`](SPEC-competitions.md)); prize money ([`docs/SPEC-payouts.md`](SPEC-payouts.md));
the dashboard's derived gap and velocity fields ([`docs/SPEC-dashboard.md`](SPEC-dashboard.md)).

## Contract

The venue's season scoring (llms-full.txt revision 2026-09-02, Seasons → Scoring):

```text
score = chip_balance + chips_at_table
```

- `score` is integer leaderboard points; `chip_balance`, `chips_at_table` and the other game fields
  are integer virtual chips. Never treat a game stack as cents.
- No rebuy penalty in the default configuration: every chip the account controls, in the account or at
  a table, counts toward its score.
- Rank is by score. `win_rate` and `hands_played` are alternate `sort_by` views, not alternate
  ranking keys.
- The official display and prize eligibility require at least 10 hands. The leaderboard API defaults
  to all entries, so a prize-facing read passes the inclusive `min_hands` qualifier (use 10).
- `GET /season/leaderboard`: public, 30/minute per IP; params `sort_by` (`score`|`hands_played`|`win_rate`),
  `limit` (max 1,000), `offset`, `min_hands`.
- `GET /season/me` (auth): `chip_balance`, `chips_at_table`, `score`, `rank`, `total_participants`,
  `rebuy_penalty`, `pro_tier`, `auto_rebuy`, `starting_chips` (5,000).

Where SvanBot reads it:

| Read | Where | Request |
|---|---|---|
| Dashboard leaderboard panel | `crates/apps/bot/src/api/insights/leaderboard.rs` | `/season/leaderboard?min_hands=10&limit=200`, cached 60 s |
| Insights sorts | `crates/apps/bot/src/api/insights.rs` | three `sort_by` calls, each `min_hands=10&limit=1000` |
| Experiment-pair reading | `crates/apps/bot/src/experiment.rs` | `/season/leaderboard?sort_by=score&min_hands=10&limit=1000` |
| Funding decision | `crates/apps/bot/src/client/rest.rs` | `/season/me`, both fields from the same payload |
| Opponent reputation book | `crates/apps/bot/src/reputation.rs` | `limit=1000`, no `min_hands` (history, not a prize rank) |

## Gates

- `python3 scripts/docs-check.py` keeps every path here real; this file is listed in `scripts/docs-check.live`.
- The funding read is covered by tests in `crates/apps/bot/src/client/rest.rs`: a failed `/season/me`
  read is unknown, not a default balance, and the buy-in plan defers to a scheduled auto-rebuy.
- The score itself has no local gate because there is no local computation: it is read back from the
  venue. A local recomputation would need its own tests and this Contract to move.

## Traps

- **`win_rate` is not the rank key.** Rank and prizes key on score; a fleet can lose win rate while
  gaining rank (and the reverse). Do not report a win-rate change as a rank change.
- **The default leaderboard is unqualified.** It includes entrants under 10 hands; only a
  `min_hands=10` read matches the official display and prize eligibility. The reputation book is the
  one read that skips the qualifier, and it is not prize-facing.
- **`chips_at_table` counts.** A seated bot's table chips are score; the funding plan waits for them
  to return before a top-up rather than treating the off-table balance as everything.
- **Score is chips, never money.** No `*_cents` arithmetic applies to it.
- **`min_hands` is inclusive.** "At least 10 hands" includes an entry with exactly 10; the API
  default is all entries, not all eligible entries.

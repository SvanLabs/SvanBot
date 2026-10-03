# SPEC — private competitions

> **Read this when** you consider connecting SvanBot to a private competition (`competition_id`).
> **Code:** none — no `competition_id` in the client, config, dashboard or scripts.
> **Related:** [`docs/SPEC-protocol.md`](SPEC-protocol.md) for the public connection this one sits beside; [`docs/SPEC-scoring.md`](SPEC-scoring.md) for the public season score. · [All docs](README.md)

The venue's private-competition surface, checked against `https://docs.openpoker.ai/llms-full.txt`
**revision 2026-09-02**, and SvanBot's position on it: **not supported**. The client connects to the
public league only, and this file exists so the next change starts from facts instead of a re-audit.

## Status

**Unsupported.** `competition_id` appears nowhere in `crates/`, `web/src`, the config or the scripts;
there is no scope, key or flag for it. Support is deferred until private competitions are actually
entered — no operator order to do so exists.

## Scope and non-goals

**Scope.** The venue's private-competition connection scope, scoring and lifecycle, recorded as the
baseline any future support would build on.

**Non-goals.** Creating or entering competitions; implementing the connection scope, competition
scoring or rebuys; public season play ([`docs/SPEC-scoring.md`](SPEC-scoring.md)).

## Contract

Connection (llms-full.txt revision 2026-09-02, Private Competitions):

```text
wss://openpoker.ai/ws?competition_id=<competition-uuid>
Authorization: Bearer <api_key>
```

- Omitting `competition_id` always selects the public league. Private scopes reject query-string
  credentials and hosted/saved-strategy bots.
- Participants do not need Pro, but must run their own client with an active standard API key owned
  by the accepted account. The invite is accepted in the dashboard, the current rules version is
  confirmed, and a rule change before enrollment closes invalidates the prior confirmation.
- The healthy connection check goes stale after 10 minutes.
- Connection policy: one connection per owner in each exact competition scope, regardless of Pro. A
  second connection in the same scope is rejected with `scope_connection_already_active`; public plus
  Competition A plus Competition B may run concurrently. Same-owner public bots are never seated together.
- After `connected`, the normal V2 protocol and `join_lobby` apply, with a buy-in inside the
  organizer-defined range; competition tables contain only entries from that competition.
- Scoring: `score = (chip_balance + chips_in_play) - rebuy_penalty_total`; eligibility requires the
  configured minimum hands (default 0) and no disqualification; tie-breakers are hands played
  descending, rebuys ascending, acceptance time ascending.
- Rebuys are manual only; reuse the same `request_id` after a lost response. When enabled: 1–20
  rebuys, 1,000–5,000 chips, score penalty 0–5,000, cooldown 0–86,400 seconds.
- Organizer defaults: participant cap 24 (range 2–2,000), 5,000 starting chips, buy-ins
  1,000/2,000/5,000, 10/20 blinds, six seats, rebuys disabled.
- Lifecycle: `draft → enrolling → enrollment_closed → active ↔ paused → ending → results_pending →
  finalized`; cancellation can end a non-finalized competition without ranked results.
- Spectating: `GET /api/public/competitions/{slug}` (no auth, no collection endpoint; private
  competitions are unlisted and need the direct URL or slug).

Where SvanBot stands:

| Question | Answer |
|---|---|
| Is the scope configured? | No — no `competition_id` reference exists |
| Can the client connect? | It always connects to the public league; no competition parameter is sent |
| Is the fleet eligible? | Self-hosted clients with a standard key are, per the venue; hosted/saved-strategy bots are not |
| Why deferred? | No operator order to enter one; entry is recorded out of scope in the season map (#698) |

## Gates

- No code, no test: absence is the state, and it is checked by hand:

```sh
grep -rn "competition_id" crates/ web/src || echo "no hits (expected: unsupported)"
```

- Support arrives only with a ticket that adds the connection scope, config validation and tests;
  that change moves this file's Status and `docs/SPEC-protocol.md` together.
- `python3 scripts/docs-check.py` keeps every path here real; this file is listed in `scripts/docs-check.live`.

## Traps

- **Omitting the parameter is silent.** It selects the public league; a typo or dropped query field
  does not error, it changes where the bot plays.
- **Private scopes reject query credentials.** The Bearer header is required there even though the
  public WebSocket still accepts the legacy query token.
- **One connection per owner per scope.** The five-bot Pro fleet cannot all enter one competition;
  the public portfolio limit does not apply per competition.
- **Hosted bots are excluded.** SvanBot is self-hosted, so eligibility is account-level; do not
  assume the venue's hosted product and this client share rules.
- **Competition score is not the season score.** It subtracts `rebuy_penalty_total` and its minimum
  hands is configured (default zero), so public leaderboard logic does not carry over.
- **The connection check goes stale after 10 minutes.** Silence is not health.

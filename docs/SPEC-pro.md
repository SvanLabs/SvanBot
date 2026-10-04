# SPEC — Pro account and Portfolio endpoints

> **Read this when** you touch account tiers, API keys, the fleet-size cap or hand-history export limits.
> **Code:** `crates/apps/bot/src/setup.rs` (the cap), `crates/apps/bot/src/seasons.rs` (the `pro_tier` read), `crates/apps/bot/src/history/download.rs` (the export cap).
> **Related:** [`docs/SPEC-protocol.md`](SPEC-protocol.md) for the endpoints the client does call; [`docs/SPEC-scoring.md`](SPEC-scoring.md) for what rank pays. · [All docs](README.md)

The venue's Pro tier and management API, checked against `https://docs.openpoker.ai/llms-full.txt`
**revision 2026-09-02**, and what SvanBot uses of it: the `pro_tier` flag and the five-bot fair-play
cap — not the purchase, hosted-bot or portfolio surfaces.

## Status

**Partially supported.** SvanBot runs as a Pro account (five concurrent public bots), reads
`pro_tier` to lift the hand-history export cap, and can renew Pro from the credit balance when it
lapses (`crates/apps/bot/src/proauto.rs`, opt-in, off by default). Hosted-bot control, portfolio
management and the token purchase are unsupported: no call to any of them exists.

## Scope and non-goals

**Scope.** What Pro buys, the management endpoints the venue documents, and the one flag SvanBot reads.

**Non-goals.** Buying Pro any way but the opt-in renewal below (no token purchase, no single-season
endpoint); automating `/portfolio/*` or `/bot/*` management; hosted-bot
mode (SvanBot bots are self-hosted local processes); the fleet configuration UI itself
(`crates/apps/bot/src/api/setup.rs`).

## Contract

Limits and features (llms-full.txt revision 2026-09-02, Seasons → Pro):

- Free: one public bot. Pro: up to five distinct playable portfolio bots concurrently; same-owner
  bots are never seated together. Both tiers can play; Pro is not required to play.
- Pro adds: unlimited hand-history export (Free is capped at 20,000 hands), a shorter rebuy cooldown
  (2 minutes vs 5), the Custom Bot builder, and the leaderboard Pro badge.
- Purchases (auth, USDC credit balance): `POST /api/season/pro` (1 season, $5),
  `POST /api/season/pro-bundle` (3 seasons $12, 6 seasons $20), `POST /api/season/pro/token`
  (ERC-20, 10% discount). Bundles are repeatable; use a stable `request_id` only when retrying the
  same purchase.
- Bot Control (Pro API; Free gets 403): `GET`/`PUT /bot/strategy/api`, `POST /bot/deploy/api`,
  `POST /bot/stop/api`, `GET /bot/status/api` — hosted-bot management.
- Portfolio API (owner level): `GET /portfolio`, `POST /portfolio/bots`, per-bot strategy
  read/save/review, deploy, runtime switch, stop, status, season-entry, analytics, hand-history and
  export, key rotation. A child key can act only as that child bot.

Where SvanBot uses it:

| Use | Where | Notes |
|---|---|---|
| Five-bot cap | `crates/apps/bot/src/setup.rs`, `crates/apps/bot/src/config.rs` | `MAX_BOTS = 5` enforces the fair-play limit on dashboard saves and on `.env` keys (beyond the cap an error names the rule and the key is ignored) |
| `pro_tier` read | `crates/apps/bot/src/seasons.rs` | `GET /season/me` per key; fails closed (`false`) |
| Export cap lift | `crates/apps/bot/src/history/download.rs` | Pro keys ignore `SVANBOT_EXPORT_CAP` (0 = unlimited) and download ended seasons |
| Renewal on lapse | `crates/apps/bot/src/proauto.rs` | `SVANBOT_AUTO_RENEW_PRO=1` (default 0) and `SVANBOT_AUTO_RENEW_SEASONS` (1, 3 or 6; default 3). When the owner key (`SVANBOT_API_KEY`) reads `pro_tier` false on a fresh `GET /season/me`, on a box with more than one key, outside dry run: `POST /season/pro-bundle {seasons, request_id}`, widest bundle first, a `402` (nothing charged) steps down. `request_id` is saved before the request and reused to retry a request of unknown fate; nothing more is bought for six hours after a purchase, and nothing for a week if the owner still reads Free after one (the read is then wrong, not the account). Needs `SVANBOT_API_KEY` as the first bot: a child key reads Free under Pro. **Credits only, never real money:** the one purchase call debits the account's existing credit balance (a `402` charges nothing); no wallet, deposit, card or token payment exists in the code, and a test fails if one is added. Off: no purchase call and no extra venue request, only a warning once an hour when the owner reads Free |
| Everything else | — | No call to `/bot/*/api`, `/portfolio/*`, `/season/pro` or `/season/pro/token` exists |

## Gates

- `crates/apps/bot/src/seasons.rs` tests pin `is_pro`; `crates/apps/bot/src/history.rs` tests pin the
  export cap (0 means unlimited); `crates/apps/bot/src/setup.rs` tests cover the plan validation.
- The management endpoints stay absent; check before claiming support:

```sh
grep -rn "portfolio/bots\|/bot/strategy\|/bot/deploy\|/bot/stop\|season/pro/token" crates/ web/src || echo "no management-API calls (expected)"
grep -rn "season/pro" crates/ | grep -v "proauto"   # expected: nothing but docs comments
```

- `python3 scripts/docs-check.py` keeps every path here real; this file is listed in `scripts/docs-check.live`.

## Traps

- **Purchases spend real USDC.** `/season/pro*` debits the credit balance. The only caller is the
  opt-in renewal in `proauto.rs` (off unless `SVANBOT_AUTO_RENEW_PRO=1`); never call a purchase
  endpoint from anywhere else, and never from a bot loop.
- **The Portfolio API is not this fleet's control plane.** It manages OpenPoker-hosted bots; SvanBot
  bots are self-hosted processes started from `.env`, so deploy, stop and runtime switch do not apply.
- **Same-owner bots never share a table.** The five-bot cap is a fair-play limit, not five seats at
  one table; do not treat the fleet as self-play practice.
- **Do not add accounts or agents to bypass the cap.** The venue documents monitoring for collusion
  and ban or prize-disqualification risk.
- **`request_id` is retry-only.** Reusing it retries one purchase; a fresh id is a new purchase.
- **The `pro_tier` read fails closed.** A transient error must not lift the export cap; keep that
  direction if the read is ever changed.

# SvanBot fair-play compliance audit

Date: 2026-11-15. Branch `fix/bot-learning-1014` at `4df58e6`. Read-only: no builds, no edits to the
repo, no requests to openpoker.ai. Databases were opened with `sqlite3` URI `mode=ro` through
Python, because the `sqlite3` CLI is not installed. Helper scripts are in this folder
(`scan_raw.py`, `overlap.py`).

## Verdict: AT RISK (not violating)

I found no evidence that two of our bots ever shared a table or a hand. There is also no code that
passes information between seats, and no code for chip transfers or soft play. The fleet is 5 bots
under one Pro owner account, which the rules allow. The risk is in the code: the guard that keeps
our bots apart has gaps, the code comments now treat a shared hand as normal, and the hand-history
client ignores `Retry-After`.

---

## 1. Bots, owner accounts, and whether the code assumes Pro

- `.env` (gitignored; values not reproduced) sets 5 keys: `SVANBOT_API_KEY` plus
  `OPENPOKER_API_KEY_2..5`. The bot names are SvanBotV10, Svanar, Svanism, SuraGunnar and SurSvan.
  `SVANBOT_FLEET=all` means one process (`target/release/sv10-bot`, pid 462407) plays all five.
- **One owner account.** The cached leaderboards (`kv` keys `season.lb.*` in `artifacts/svanbot10.db`)
  list all five under `account_name=SvanBotV10`, `account_agent_id=f562e3cd-…`,
  `account_bot_count=5`. SvanBotV10 is `primary` and the other four are `child`. The earlier names
  SvanBotV2 and SvanBotV7 are the same account's primary under older names.
- **Accounts that look similar but are not ours.** Four separate one-bot accounts on the
  leaderboards have names close to ours. Nothing in the repo, its git history or `.env` refers to
  them, and the operator confirmed on 2026-11-15 that none of them is theirs. They are not named
  here because this repository is public.
- **The code assumes Pro.**
  - `crates/apps/bot/src/setup.rs:13` and `crates/apps/bot/src/config.rs:95-142` hard-code
    `MAX_BOTS = 5`. Keys beyond 5 are refused with an error.
  - Nothing enforces a 1-bot limit for Free accounts. If Pro lapses, the venue refuses the extra
    bots (`portfolio_pro_required`). `crates/apps/bot/src/proauto.rs` then warns, or renews Pro from
    credits if you opt in. So a Free account cannot run 5 bots, but only because the server refuses.
  - `crates/apps/bot/src/history/download.rs` lifts the export cap when `pro_tier` is set.
  - `docs/SPEC-pro.md` states the rules correctly: a 5-bot cap, same-owner bots never seated
    together, and no extra accounts.

## 2. Have two of our bots ever shared a table or a hand?

**No evidence in any data I could check.**

| Check | Data | Result |
|---|---|---|
| Same `hand_id` stored for more than one bot | `svanbot10.db` `hands`, 18,450 rows, 2026-11-13 → 11-15, primary key `(bot, hand_id)` | **0** |
| Another of our bots listed among a hand's `players` | same 18,450 hand summaries | **0** |
| Seated at the same table at the same time (sessions split on 10-minute gaps, overlap tested per table) | `hands` table | **0**. 58 table ids were used by more than one bot, but at different times. |
| Another of our bots among a hand's winners | `history.db` `raw`, 166,603 server-exported hands, 2026-05-28 → 11-15, all decompressed | **0** |
| Seated at the same table at the same time | `raw` (`table_id`, `started_at`) | **0** |
| Insert collisions in the season export | `history.db` `meta` `season:*`. Rows go in with `INSERT OR IGNORE` on a unique `hand_id`, so a shared hand would count as "known" rather than "new". | **0 collisions.** For every season in progress, `new_hands == offset` (about 75k hands). For season e06fc5ca, each child bot's stored count equals its leaderboard `hands_played` exactly: 5475, 5561, 5220, 5081. |
| Fair-play leave events (`fair play: same-owner …`) | `events` table and all logs | **0** |

**What commit eaf3551 actually is.** It did not come from an observed incident. It fixes finding 1
of issue #1014 ("low, plausible findings" from sweep #1002). Its test inserts made-up rows A/h1 and
B/h1 into a scratch store. `Store::recent_fit_hands_of` (`crates/libs/store/src/store/provenance.rs`)
and its callers (`playersize.rs:23`, `neural.rs:151`, `bin/calibrate.rs:35`, and
`pacing_study.rs:26`, which predates the commit) are defensive deduplication. Their comments state
"two of our seats at a table store the same hand_id" as if it were a normal event. A shared hand is
a fair-play violation, so the code should treat it as an alarm, not quietly deduplicate it.

**Limits of this evidence.**
- `history.db` keeps one row per `hand_id`. A bot that sat **only** in hands shared with another of
  our bots would leave no row of its own.
- The live store only goes back to 2026-11-13, and the backups are all from 2026-11-15.
- The export lists only winners' names, not every seat.

The season collision counts are the strongest evidence for the period before November.

## 3. Information shared between our seats, or anything that looks like collusion

**Nothing that works within a single hand or table.**

- **Decisions.** `crates/apps/bot/src/client/*` decides from one tracker per bot (its own table).
  Policy and engine code never reads another bot's hole cards or seat state. The cross-slot reads
  (`experiment.rs`, `installs.rs`, `main.rs:205`) handle parameters, lineages and shutdown.
- **Shared opponent models.** All five bots feed and read `models.v1`, `reputation.v1` and
  head-to-head records (`tasks.rs:257`). This pools what each bot saw of opponents at **different**
  tables. Rules commonly allow this, but auditors could read it as "information sharing". Our own
  names are left out of opponent modelling (`playersize.rs:17`, `api/players.rs:155`).
- **Table selection.** `client/quality.rs:60` never "seeks" a table because one of our bots is
  there. Seeking targets top-ranked other bots.
- **Chips.** Nothing transfers, tips or dumps chips. "Banking" (`seat.rs` `Move::Bank`, leave and
  rejoin with a fresh buy-in) and top-ups move chips only between a bot's own table stack and its
  own balance.
- **The same-owner guard has gaps** (`client/seat.rs:126-188`, `client/handler.rs:174`):
  1. **It acts too late.** `stablemate_at_table` runs only in `between_hands`, after a hand
     *finishes*. Nothing checks at join time or when the table snapshot arrives. A pairing the
     server let through would therefore play at least one full hand together before the bot leaves.
  2. **It cannot see the other bots in split mode.** `config.rs:145` keeps only the `SVANBOT_ONLY`
     bots in `config.bots`. `scripts/supervisors.sh:102` starts each split worker with
     `SVANBOT_ONLY=<its own name>`. Each worker's fleet list then holds only itself, so
     `stablemate_at_table` (which skips its own seat) can never fire. The live fleet runs
     `SVANBOT_FLEET=all`, so this is a latent gap.
  3. **It misses old names.** It matches only the names configured now. Bots renamed or aliased
     (`SVANBOT_ALIASES`, `fleet_names()`), or run from another checkout, are not recognised.
- `pacing_study.rs:26`, `neural/shared_hands.rs` and the new `recent_fit_hands_of` normalise the
  shared-hand case (see section 2).

## 4. How hand history is downloaded, and whether it respects the rate limits

The download is one background task, `crates/apps/bot/src/history/download.rs`, spawned at
`tasks.rs:95`. It runs bots one after another, in 3 stages per bot each pass:

1. **Newest hands.** `GET /me/hand-history?limit=200&offset=N`, with requests spaced by
   `REQUEST_GAP = 2.6 s` (`history.rs:23`). That is about 23 requests a minute at most.
2. **Backfill by offset.** Same endpoint and spacing. A failing page is retried every 10 minutes.
3. **Past seasons** (Pro only, `seasons.rs`).
   `GET /me/hand-history/export?format=json&season_id=…&limit={200,100,50}&offset=N`, 6 s apart
   (`EXPORT_GAP`), 2 pages per bot per pass.

Between passes it sleeps 60 s, or 1 h once every bot is complete. It never calls the
`/portfolio/bots/{id}/hand-history` endpoints (`docs/SPEC-pro.md` confirms this).

| Rule | Status |
|---|---|
| `limit` ≤ 200 | **Met** (`PAGE = 200`; the export falls back to 100, then 50). |
| Paging by offset | **Met.** |
| 30 requests/min | **Met in practice.** All calls go through one sequential task with 2.6 s or 6 s gaps, so it stays under about 23/min. But: (a) there is no shared limiter across `download.rs`, `seasons.rs`, `reputation.rs` and the CLIs. (b) After a stage ends, or when moving to the next bot, the next request goes out without a gap. (c) In split mode every worker runs its own downloader, so up to 5 run at once. If the limit is per account, that could exceed 30/min. |
| Honour 429 / `Retry-After` | **Not met to the letter.** No code reads the `Retry-After` header. After a 429, `download.rs:22` waits a fixed 60·(n+1) s over 4 tries, `seasons.rs:187` waits 60·(n+1) s, and `reputation.rs:56` waits 20·(n+1) s. Those waits are usually longer than the header would ask for, but a longer `Retry-After` is ignored. `seasons.rs:25` records that the export endpoint did hit 429s at the 2.6 s pace on 2026-09-15. |
| Repeated failing requests | `svanbot10.log` has **3,471** "season backfill failed: export page is not JSON … EOF" lines since 2026-11-13, about 65–80 an hour. All are for season `4aec0e01` across the 5 bots. A 200 with an empty body is retried every pass, with no backoff and no ceiling. That volume is well under the rate limit, but it is pointless load that a venue's monitoring would see. |

## Fixes needed

1. **Act on a same-owner pairing at once, not after the hand.** Run `stablemate_at_table` when the
   table snapshot or seat list changes, and before taking any action. Fold or leave immediately,
   and log it as an `error` event that sets off an alert. Count these events on the dashboard.
2. **Split mode: give every worker the whole fleet's names**, including aliases. Use the full
   configured key and name list, or `fleet_names()`, for `stablemate_at_table` and
   `quality.rs:60`, not the `SVANBOT_ONLY` subset.
3. **Treat a shared `hand_id` as a fair-play incident.** Keep the deduplication in
   `recent_fit_hands_of`, but have it (or the hand insert) log or alert when it finds two of our
   bots in one hand. Reword the comments in `playersize.rs`, `neural.rs`, `calibrate.rs`,
   `pacing_study.rs` and `neural/shared_hands.rs` so they describe a forbidden state that is being
   guarded against, not a normal one.
4. **Respect `Retry-After`.** Parse it in `download.rs::fetch_page`, `seasons.rs::fetch` and
   `reputation.rs::get_json`, and wait `max(Retry-After, current backoff)`.
5. **Add one rate limiter per key/account** (a token bucket at 25/min or less), shared by every
   hand-history caller, including the gap after a stage and between bots. In split mode, run the
   downloader only in the head process.
6. **Back off on empty 200 export pages.** Treat them like a ceiling (retry hourly, then give up)
   instead of every pass.
7. **Done 2026-11-15:** the operator confirmed that none of the four lookalike accounts is theirs.
   Optionally, record in `docs/SPEC-pro.md` that pooling opponent models
   across our bots is a deliberate decision.

## Note on handling secrets

While listing the `.env` keys, my redaction printed the **first 3 characters** of each key value to
the tool output. Nothing beyond that was shown, and no key characters appear in this file. Three
characters do not make a key usable, but rotating the keys is cheap if you are cautious.

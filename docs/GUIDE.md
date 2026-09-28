# SvanBot Documentation

SvanBot is a self-hosted, fully autonomous poker bot fleet for **openpoker.ai** (6-max No-Limit
Hold'em, virtual chips, 14-day seasons). It plays up to five portfolio bots at once, learns every
opponent it meets, improves its own strategy in the background, and reports everything live in a
web control room. It runs on CPU only.

## Where to start

| You want to… | Start with |
|---|---|
| Get a fleet running on a new machine | Section 1, Quick start, then section 13 when something looks wrong |
| Understand what the control room is showing you | Section 10, The control room |
| Understand how one decision is made | Sections 2 and 3, then sections 4 to 6 for the models and the learner |
| Change a setting | Section 12, Configuration reference |
| Change the code | Section 14, Development, and `CONTRIBUTING.md` in the repository, which walks a first pull request from issue to merge |

Every section stands on its own, and the contents below are in reading order for a newcomer.

## Contents

1. Quick start
2. How it works (overview)
3. How the bot decides
4. Opponent modeling
5. Neural opponent-response model
6. The autonomous learner
7. Playing on openpoker.ai (connection lifecycle)
8. Data, persistence and backups
9. Hardware auto-tuning
10. The control room (dashboard guide)
11. HTTP API reference
12. Configuration reference
13. Operations and troubleshooting
14. Development
15. Glossary

---

## 1. Quick start

### New machine

```
cd SvanBot
scripts/setup.sh
```

`setup.sh` checks for Rust and Node.js, creates `.env` from `.env.example` with a random dashboard
password, builds everything optimized for your CPU, builds the dashboard, runs the tests and prints
the hardware profile the bot will tune itself to.

**Without a Rust toolchain** (any x86-64 Linux with glibc at least the bundle's `MANIFEST`): unpack
a bundle built by `scripts/portable.sh` and run `scripts/install.sh`. It installs the x86-64-v3
build on CPUs with AVX2/BMI2/FMA (within ~1% of a native build) and x86-64-v2 otherwise, and creates
`.env`. **Offline source builds**: `scripts/vendor.sh` once while online, then
`cargo build --release --offline --config .cargo/vendor.toml`.

Then open `.env` and fill in:

| Key | What to put |
|-----|-------------|
| `SVANBOT_API_KEY` | API key of your main bot (openpoker.ai dashboard → bot → Self Host) |
| `SVANBOT_MAIN_NAME` | That bot's name |
| `OPENPOKER_API_KEY_2..5` / `BOT_2..5_NAME` | Extra portfolio bots (Pro accounts, optional) |
| `SVANBOT_WEB__OPERATOR_TOKEN` | Dashboard password (setup generates one) |

### Everyday commands

| Command | Effect |
|---------|--------|
| `scripts/start.sh` | Start the fleet and the learner under a restart-on-crash supervisor |
| `scripts/stop.sh` | Stop everything gracefully (models are saved) |
| `scripts/status.sh` | Show each bot's mode, session hands, net, rank and database totals |
| `scripts/release.sh` | Build, test and install a new release; the running fleet and learner hot-swap to it without stopping play |
| `scripts/restart-bot.sh` | Restart only the fleet process (the learner keeps running) |
| `./target/release/review all 10` | Our own tendencies plus the 10 biggest losing hands with decision EVs |
| `./target/release/calibrate [samples]` | Re-fit the range model to showdowns now (`CALIBRATE_DRY=1` to only report) |

The dashboard is at `http://<SVANBOT_WEB__HOST>:<SVANBOT_WEB_PORT>` (default port 5000). Unlock it
with the operator token.

---

## 2. How it works (overview)

```diagram:processes
                        dashboard (web/, React)
                                 ▲ state (SSE + API)
                                 │
 openpoker.ai ◄─ WebSocket+REST ─► sv10-bot ◄──── params.v1 ──── learner
 (tables, leader-                 5 bot tasks: seat, track,       fits, clone population,
  board, histories)               decide, act; hourly backup      challenger search
        │ exports                 │ ▲                               │ reads hands
        ▼                         ▼ │                               ▼
   history.db ◄──────────────  svanbot10.db  ◄──── audit queue ───► analyst
   (server exports,           (hands, decisions, kv,                deep re-solve of big
    training corpus)           calibration, audits)                 decisions; wiring table
                                  │
                                  ▼
                           archive (nightly) ──► /backup-disk (HDD): daily / weekly / monthly
```

- **`crates/libs/`** — the poker and data libraries with no network: hand evaluator, NLHE engine, equity,
  opponent statistics, range reconstruction, the decision policy, the neural network, the simulator,
  the protocol tracker and the SQLite store. **`crates/apps/core`** re-exports them and holds the tools.
- **`crates/apps/bot`** — the live fleet (`sv10-bot`), the background `learner` and `analyst`, the
  `review` tool and the dashboard API.
- **`web/`** — the React control room.
- **`artifacts/`** — runtime data: database, logs, backups.

---

## 3. How the bot decides

Every decision follows the same pipeline. It takes about 0.1 s at the median and 0.3 s at the 95th
percentile with the CPU shared, against a 45-second deadline.

```diagram:decision-path
 your_turn ──► situation ──► opponent ranges ──────────► shared deals
 hand_id,      seats, stacks, their stats, our image,     Monte Carlo on every core,
 turn_token    pot, board     size and timing tells       or exact runouts
                                                                │
      ┌─────────────────────────────────────────────────────────┘
      ▼
 price candidates ──► self-calibration ──► choose, legalize ──► send + record
 fold, check/call,    measured bias per    best EV, mixing      action with hand_id and
 raise sizes: stats   category, bounded    only near-equal      turn_token; decision,
 + network, fold      by margin evidence   options; only valid  replay and audit queue
 offsets, live fits                        actions
```

A legal check or fold is prepared before any of this runs, and is sent if a step fails or runs out of
time.

### 3.1 Situation

The table tracker keeps the authoritative server state (`table_state` snapshots) plus the action
history (`player_action` events). On `your_turn` it builds a *situation*: hole cards, board, pot,
every player's stack and street bet, the legal actions and the full action history.

### 3.2 Opponent ranges

For every opponent still in the hand the bot replays their actions and weights each of the 1,326
possible hole-card combos by how likely *that player* (per their learned profile) would have taken
those actions with it:

- **Preflop** — opening from early/late/blind positions narrows to their positional open rate;
  calls, 3-bets and 4-bets narrow to their calling and re-raising frequencies.
- **Postflop** — bets and raises keep the strong part of the range plus a bluff share learned from
  their showdowns; calls remove the part they would have folded; checks cap the range.

Hand strength per combo comes from an exact made-hand ranking blended with Monte Carlo equity,
cached per board and shared by every thread.

**Showdown-fitted range model.** The shapes of these likelihoods (how soft the "top x% of hands"
cut-offs are, how much a big bet polarizes, how strongly a call removes raising hands, how much a
check caps a range, how wide flat calls and limps are, and so on — 23 constants in
`oprange::RangeParams`) were originally hand-set. `calibrate` fits them by **maximum likelihood**
to real showdowns: for every hand an opponent showed, it replays that opponent's full action line
with their learned profile and scores the probability the range model gave to the exact combo they
showed. Coordinate ascent maximises the mean log-likelihood on older showdowns; the newest quarter
is held out, and the fitted set is used only if it beats the defaults there. The learner re-fits
once a day. The first fit showed the hand-set cut-offs were far too sharp: real bot ranges are
fuzzier, and the fitted model extracts roughly twice as much information from an action line.

### 3.3 Candidate actions and expected value

The policy scores fold, check/call, and several bet/raise sizes:

- preflop: open 2.5bb (+1bb per limper), 3-bet 3x in position / 3.8x out of position, 4-bet 2.3x
- postflop: 33%, 55%, 80% and 120% of the pot, plus all-in when allowed

For a bet or raise it models every opponent's **response**:

1. Opponents perceive our range from our own actions.
2. Each opponent continues with the hands whose equity against that perceived range beats the
   price they are offered, shifted by how light they are known to call.
3. How often they continue is capped by their learned fold statistics, blended with the neural
   response model when it is active.

EV combines the chance everyone folds (we win the pot) with the called outcomes (one caller or
several), using equity against the continuing ranges and a realization factor that depends on hand
strength and position.

**Shared deals (common random numbers).** Equity is sampled once per decision: deals are drawn
from every opponent's full estimated range and evaluated once. Each candidate's equity against
the *continuing* part of those ranges reweights the same deals by `continuing weight / full
weight` of the dealt combos (importance sampling), falling back to fresh samples when the
continuing range carries too little weight (effective sample size under 50). Heads-up with a flop or later, when every opponent combo times every board completion fits the budget, the deals are the exact enumeration instead, each weighted by the combo's range weight, so every candidate's equity is exact. Every candidate is
compared on identical deals, so sampling noise no longer differs between options, and a decision
costs 1.5 ms instead of 2.55 ms with no measurable change in strength (a paired simulation on
identical deals moves the result by less than its own confidence interval). `reuse_deals` in the
parameters switches it.

### 3.4 Safety rails

- **Jam limits** — all-in is only considered at low stack-to-pot ratios, or with near-nut equity
  and at most five times the pot.
- **No bluff raise wars** — players who already raised a street are modeled as rarely folding; a
  street with two raises needs 55%+ equity to raise again; river raises need 50%+.
- **Raise-back risk** — when pricing a postflop bet, the share of continuing opponents who would
  *raise* (from their raise frequency and the neural model) is priced separately: weak hands lose
  the bet in that branch, strong hands keep their equity. Its weight (`raise_risk`) is one of the
  learner's tunable knobs.
- **Crash-proof play** — a malformed legal range can never panic the action path; a bot whose
  session task panics is restarted automatically within 5 seconds; shared state uses
  non-poisoning locks so one failure cannot freeze decisions.
- **Legal action guard** — the chosen action is mapped onto exactly what the server offers; a raise
  is clamped into range; an unavailable raise becomes a call or check. If a decision ever fails or
  takes more than 8 seconds, the bot checks (or folds).

Near-equal options are mixed slightly so the bot is not perfectly predictable.

### 3.5 Self-calibration

Every decision stores its **spot category** (street, action family and bet-size class, e.g.
`turn:bet:big`) and the EV the model predicted relative to folding. When the hand ends, the chips
actually won from that point are stored beside it. Every five minutes the fleet averages the gap
per category; once a spot has 60+ decisions and a gap wider than its 95% band, a shrunk
correction is added to that spot's EVs, capped asymmetrically — **+3 bb** upward, because a
residual is measured only where we already took the action, and **−15 bb** downward, because
over-optimism is what spends chips. The bot therefore corrects spots where its own model is
systematically over- or under-optimistic, with no human input. The Self-calibration panel lists
every category and the correction in use.

---

## 4. Opponent modeling

Every completed hand updates statistics for every opponent at the table, keyed by their public
name (names are stable across tables and sessions).

| Statistic | Meaning |
|-----------|---------|
| VPIP / PFR | Voluntarily put money in / raised preflop |
| Open, limp, 3-bet, call-open, fold-to-3-bet, 4-bet, fold-to-4-bet | Preflop decision frequencies |
| VPIP / PFR / open by position | Early (EP/MP), late (CO/BTN), blinds |
| Bet when checked to (flop/turn/river) | Postflop initiative |
| Fold / raise vs a bet (by street and bet size) | How they respond to pressure |
| C-bet, fold-to-c-bet | Continuation-bet habits |
| WTSD, won at showdown | How often they go to showdown and win there |
| River bluff rate | Share of river aggression that showed down weak |

**Shrinkage** — with few hands a player's rates are pulled toward the population; the population
itself is the aggregate of every opponent seen (replacing hand-tuned defaults once large).

**Reputation from past seasons** — every season's public leaderboard (cached once a season has
ended) is folded into a reputation book keyed by the stable agent id, so renamed bots
keep their history. Each opponent gets seasons played, best rank, top-10 finishes, lifetime hands
and a hands-weighted *strength* (1.0 = always first).

**Every past hand our bots played, every season** — the server's hand-history export
(`GET /me/hand-history` for the active season, newest first every pass; the per-season
`/me/hand-history/export?format=json&season_id=` for every ended season, a few pages per bot
per pass) downloads into `artifacts/history.db`. No operator action beyond the keys in `.env`:
Pro keys are detected via `pro_tier` and download without limit; Free keys keep the
`SVANBOT_EXPORT_CAP` backfill stop. The export lists every seat's actions, the board and shown cards but no
blinds, stacks, call amounts or seat names, so `core::history` replays each hand: blinds from the
button, street bets and pots rebuilt from raise-to amounts (the rebuilt pot matches the server's
total in 95% of hands; the rest involve unknown short stacks), streets corrected (each exported
action is labelled with the street *after* it). Seat names are recovered from pot-winner names:
a seat is attributed to a player when the nearest winner sightings on that seat before and after
agree (or one lies within 40 hands). Unattributed seats update only the population statistics.
Older hands count less (weight halves every 30 days, never below a quarter). Imports are
crash-safe with their own watermark, hands already recorded live are skipped, and the download
resumes from its checkpoint and then stays current hourly. The export times out on pages deeper
than ~18,000–19,000 hands (server-side, load-dependent), so each pass reads the newest hands
first and then retries the deepest page every 10 minutes indefinitely; the backfill frontier is
shifted by hands played meanwhile so nothing is skipped. The replayed hands also feed neural
training (the newest 60,000) and the range-model fit.

**Styles** shown in the dashboard: Maniac, Calling station, Overfolder, Sticky, Nit,
Tight-aggressive, Loose-aggressive, Loose-passive, Balanced — each with exploit advice. The
decision engine does not use the label; it uses the underlying numbers directly.

---

## 5. Neural opponent-response model

A small multilayer perceptron (38 inputs → 48 → 24 → 3 outputs) predicts whether an opponent will
**fold, call/check or bet/raise** in a given spot.

- **Features** — street, facing-bet flag, bet size relative to pot, pot size, stack-to-pot ratio,
  players in the hand, position, preflop raise count, whether they were the preflop aggressor,
  all-in pressure, board texture (paired, flush, straight, high card), the player's stat profile,
  and the responder's calls on earlier postflop streets.
- **Training** — every learner cycle rebuilds the dataset from all stored hands (each opponent
  decision is a sample) and trains with Adam.
- **Activation gate** — the model is used only if its log-loss on the most recent 15% of hands
  beats the hand-built statistical model by at least 0.01. Otherwise the bot falls back to the
  statistical model automatically.
- **Use** — when active it provides 70% of each opponent's fold estimate inside the EV search.

The status appears in the Autonomy panel (Neural EV: AVAILABLE / VALIDATING).

---

## 6. The autonomous learner

The learner is a separate low-priority process that keeps improving the strategy without human
input.

```diagram:learner-loop
 new live hands ─────► evidence refresh ─────────► clone population
 paced by the          fold and range fits, live   profile clones of the
 dashboard cooldown    fits, per-opponent, network 16 most-seen opponents
       ▲                                                   │
       │ next cycle                                        ▼
 promote params.v1 ◄── pass ── fresh-deal gate ◄──── challenger search
 the fleet reloads     95% lower bound above         successive halving on
 it between hands      +1 bb/100 on new deals        identical (paired) deals
                              │ not settled
                              ▼
      experiment mode: bots #4 and #5 play an unresolved challenger live
```

Each cycle:

1. **Population clones** — for the most frequently seen opponents, fit a simulated player whose
   behaviour matches their real statistics (cached; refit when their sample grows 25%).
2. **Neural model** — retrain and re-validate (section 5).
3. **Challengers** — every small one-knob variation of the current champion parameters
   (fold-probability scale, preflop fold scale, passive-line fold bonus, initiative, open and 3-bet
   sizes, 3-bet call margin, raise fold bonus, realization weight, call margin, jam ratio, raise
   risk, 4-bet, limper sizing, bet-size scale).
4. **Paired evaluation** — challenger and champion play simulated hands against the clones on
   **identical cards and identical opponent random choices** (each seat has its own random
   stream per hand), so luck cancels. When everyone is all-in before the river, the hand is scored
   by its average over the possible runouts instead of the one that happened (unbiased, less noise).
5. **Successive halving** — all challengers get a short screening run on common deals; those that
   never change an outcome, or whose upper bound is below +1 bb/100, are dropped; the better half
   doubles its budget each round until one survivor remains.
6. **Promotion gate** — the survivor is promoted only if the 95% lower bound of its advantage is
   positive and the result is confirmed again on fresh deals.
7. **Hot reload** — promoted parameters are written to the database; every live bot picks them up
   within 30 seconds.

Every evaluation (promoted or rejected) is listed in the Experiments panel with its confidence
interval.

---

## 7. Playing on openpoker.ai (connection lifecycle)

- **Connect** with the bot's API key; cold starts first ask `GET /me/active-game` and resync an
  existing seat instead of joining twice.
- **Join** the lobby with a buy-in of `min(balance, SVANBOT_BUY_IN)` (1,000–5,000). If the
  off-table balance is below 1,000 the bot requests the free 1,500-chip rebuy first.
- **Auto-rebuy** is enabled on every join; after a bust the bot rejoins automatically.
- **Top-up** — when the stack falls under 35% of the maximum buy-in and the balance can at least
  double it, the bot leaves after the hand and rejoins deeper.
- **Resync** — after any reconnect the bot requests the table's replay window and snapshot;
  missing preflop raises are rebuilt from table bets if needed.
- **Table closed / season ended** — rejoin the lobby, with exponential backoff if joins keep
  failing (e.g. during season wind-down).
- **Watchdog** — a seated bot with no table traffic for 3 minutes reconnects (not during the
  season wind-down, when tables are silent by design); an unseated idle
  bot re-issues the join after 2 minutes.
- **Seeking top bots** — with `SVANBOT_SEEK_TOP_RANK` (default 30), a table where no opponent
  ranks in the current season's top 30 is left after 15 hands (checked every 5, at most once every
  10 minutes) and the bot re-queues. The server has no table list or table choice — `join_lobby` is
  pure matchmaking — so re-queueing is the only lever. Our own bots never count, nor do bots with
  fewer than 300 hands this season (early-season ranks are noise) or bots that beat us
  head-to-head.
- **Table selection** — every 10 hands after the first 30 at a table, the bot rates each opponent
  as soft (loose, overfolding or sticky by live stats, or historically weak by reputation) or tough
  (proven top finishers or solid tight-aggressive stats). With no soft opponent and mostly tough
  ones it leaves after the hand and rejoins the lobby (at most once every 20 minutes).
  **Head-to-head results** (the chips that moved between us and each opponent in the hands they
  were dealt into, recomputed every 15 minutes) override style guesses: an opponent who
  beats us over 300+ hands at 95% confidence, corrected for the number of opponents tested, is
  always tough. `review` prints the table.
- **Controls** — Start / Pause (finish the hand, then leave) / Stop from the dashboard.

Protocol facts worth knowing: blinds appear only as bets in `table_state`; `player_action.street`
names the street *after* the action; raise amounts are raise-to totals; `leave_table` answers
`leave_pending`.

---

## 8. Data, persistence and backups

Everything lives in `artifacts/svanbot10.db` (SQLite, WAL, `synchronous=FULL`).

| Table / key | Contents |
|-------------|----------|
| `hands` | Every completed hand per bot: hole cards, board, pot, net chips, winners and the full hand summary (players, stacks, action history, shown cards) |
| `decisions` | Every decision with equity, pot, price, latency and all candidate EVs |
| `events` | Activity log |
| `kv: models.v1` | Opponent models (checkpoint every 30 s) |
| `kv: params.v1` | Live strategy parameters (promoted by the learner) |
| `kv: nn.response.v1` | Trained neural response model and its validation scores |
| `kv: learner.*` | Learner status, experiments, lineage, cycle counter, clone cache |
| `kv: hardware.profile` | Detected hardware and derived tuning |
| `calibration` | Predicted vs realized chips per decision and spot category |
| `kv: calibration.v1` | Aggregated self-calibration corrections |
| `kv: reputation.v1`, `kv: season.lb.*` | Reputation book and cached past-season leaderboards |
| `kv: range_params.v1` | Showdown-fitted range model, its fit report and whether it is in use |
| `kv: history.status` | Past-hand download and import progress |
| `artifacts/history.db` `raw` | Downloaded server hand histories and their replayed summaries (re-downloadable, not backed up) |
| `artifacts/history.db` `corpus` | Hands from other sources, tagged by source and sealed with a content digest (e.g. `openpoker-archive-frames`: full hands replayed from archived live frames) |
| `hands.digest` | SHA-256 content digest of each stored hand, verified hourly |

**Compressed storage.** Decision details, replay and audit records, and the server's hand
exports and summaries are stored compressed with the bot's own DEFLATE codec. It takes the
databases from 1.8 GB to 0.7 GB, and each backup is ~270 MB smaller. The bot packs older rows by
itself in the background.
- `./target/release/review storage` shows progress.
- `review decisions BOT HAND` and `review export HAND` print the JSON (`sqlite3` shows those
  columns as blobs).

**Backups on a second disk.** With `SVANBOT_MIRROR_HOURLY_BACKUPS` set and `SVANBOT_ARCHIVE_DIR`
on another disk than `artifacts/`, every hourly backup is also copied there and checked against its
seal, and the SSD keeps only the two newest. The mirror is off by default.

**Never losing data**

- A hand is written to the database *before* it updates the opponent models.
- The model checkpoint records the last hand row it contains; on startup every later hand is
  replayed into the models, so even a hard crash loses nothing.
- When new statistics are added, models are rebuilt from the complete hand history.
- **Backups**: a consistent copy every hour in `artifacts/backups/` (3 hourly + 1 daily kept,
  `SVANBOT_HOURLY_BACKUPS` and `SVANBOT_DAILY_BACKUPS`). Each
  backup is integrity-checked after writing and sealed with a `.sha256` sidecar; a backup that fails
  is deleted. The live database is checked before each backup — if it is damaged the bot exits
  instead, so good backups are never rotated out behind a bad copy.
- **Corruption recovery (automatic)**: at startup `svanbot10.db` and `history.db` are checked
  (`PRAGMA quick_check`). A damaged file is moved to `artifacts/quarantine/` and the newest backup
  whose hash and check both pass is restored; without one the database starts empty (history
  re-downloads).
- **Hand digests**: every stored hand carries a content digest; mismatches are logged to the
  activity log.

Manual restore: stop the fleet, copy a sealed backup over `artifacts/svanbot10.db`, start again.

- **Archives** (second disk, nightly at 04:30 by `svanbot10-archive.timer`): `./target/release/archive run`
  writes a weekly full copy of both databases plus a git bundle of the code, otherwise a daily
  differential (live database + history rows added since the week's full), and a monthly
  high-compression copy into `SVANBOT_ARCHIVE_DIR` (here `/backup-disk/svanbot10`). Every file has stored and
  decompressed SHA-256 hashes and row counts in a sealed `MANIFEST.json`; 14 daily, 8 weekly and 12
  monthly archives are kept. `archive list`, `archive verify --deep`, and
  `archive restore daily/YYYY-MM-DD --to /tmp/restore` (verified; never writes into `artifacts/`).
- **Replay records**: big decisions (pot ≥ 250 bb, call ≥ 100 bb or all-in) keep their full inputs for
  14 days; `review replay 20` re-runs them and must report every one bit-identical,
  `review replay 50 --current` shows what today's promoted parameters would do in the same spots — the
  champion's knobs on the record's own budget and prices, and the footer names that basis.

**Other data sources** — `./target/release/ingest archive <dir>` imports archived openpoker frames
(read-only, integrity-checked, resumable, idempotent) into the corpus. Opponent statistics use the
full corpus hands; the range fit and neural model use them only while they improve held-out results
(the range fit has not, so it reads server exports only; `CALIBRATE_CORPUS=1` re-measures).

**External hand histories (PHH)** — `ingest phh <dir> <source>` reads the
[PHH format](https://phh.readthedocs.io) (no-limit hold'em only; a hand is kept only if every
folded seat's finishing stack matches its contributions). Player names are prefixed with the
source, and sources outside `openpoker-*` never reach opponent statistics. Attribution: the
Pluribus hands come from the supplementary data of N. Brown and T. Sandholm, "Superhuman AI for
multiplayer poker", *Science* 365 (2019), as converted by the
[PHH dataset](https://github.com/uoftcprg/phh-dataset) (Universal, Open, Free, and Transparent
Computer Poker Research Group; code MIT, dataset CC-BY-4.0,
[doi:10.5281/zenodo.17136841](https://doi.org/10.5281/zenodo.17136841)).

**Autostart** — `scripts/units.sh` renders `scripts/svanbot10.service` for your checkout and installs
it as a systemd user unit (`systemctl --user status svanbot10`); with user linger enabled the fleet,
learner and dashboard start at boot. The units in the repository name no fixed directory, so an
install under any home directory works and moving one is a re-render. Inside a run, crashed processes
are restarted by the supervisors.

---

## 9. Hardware auto-tuning

**Precomputed tables** — exact flop and turn board strengths for every suit-canonical board
(`artifacts/tables`, 96 MB, built in ~30 s by `start.sh` when missing, checksummed; a damaged file
is ignored and strengths are computed on demand with identical values).


At every start the bot detects the CPU model, logical and physical cores, memory and AVX2 support,
and runs a short equity benchmark. From that it sets:

| Setting | Rule |
|---------|------|
| Simulation Monte Carlo samples (learner) | enough for ~2 ms of one core (600–2,500) |
| Live decision samples | 640× the simulation budget on a reference-speed CPU, scaled down on slower ones (1.6M on the i7-4770K, about 190 ms p50), dealt in parallel, one chunk per logical core. Heads-up spots from the flop on are exact instead whenever every opponent hand × board completion fits (river 990 cases, turn about 45k, flop about 1M), which is faster and has no sampling noise |
| Analyst deep re-solve | 10× the live budget on every core, for expensive spots only: pot ≥ 50 bb, a call of a quarter-pot or more, or an all-in (`ANALYST_MIN_POT_BB`, 0 = every decision; auditing small pots costs most of its CPU for almost no measured gap). Also `ANALYST_SAMPLES`, `ANALYST_THREADS` |
| Opponent memory | each opponent's decision tallies weight recent hands more (half-life 1,000 of their own hands); hand counts and the population average never decay |
| Action deadline | the server auto-folds 45 s after it sends a turn and never extends that clock; a stuck decision takes the safe action at 8 s, and a rejected action is recovered by resync |
| Learner threads | every logical core (the learner runs at low priority, so live play always comes first) |
| Learner tables / hands | scaled to core count and measured speed |

The build's instruction set is chosen for the machine that runs it: `scripts/portable.sh` builds the
x86-64-v2 and x86-64-v3 bundles explicitly (`-C target-cpu=x86-64-v2|v3`), and `scripts/install.sh`
picks the level the target CPU's flags support, preferring v3 (AVX2/BMI2/FMA). A build from source
follows whatever instruction set the checkout's cargo configuration sets.
`./target/release/probe --hardware` prints the profile; the dashboard settings show it too.
Set `LEARNER_THREADS` to override the learner's thread count.

**Compute profiles**. The Live pulse's COMPUTE cell has four buttons:
- **Quiet** leaves the PC free: live budget ×0.25, learner at a quarter of the threads, analyst on 1 thread.
- **Balanced** uses about half the machine.
- **Max** uses every core. It is the default.
- **Custom** sets the live budget share and both thread counts yourself.

A profile changes compute only, never strategy. The live budget never drops below the learner's own simulation budget. The live bots switch at once, within 30 s. The learner restarts with its new thread count between steps (at most about two minutes) and the analyst between audit batches, about 30 s after their supervisors notice the exit. A saved profile wins over `LEARNER_THREADS` and `ANALYST_THREADS`; `POST /api/compute/profile` sets one directly.

---

## 10. The control room (dashboard guide)

| Panel | What it shows |
|-------|---------------|
| Views | Tabs under the header (Live, Opponents, Learning, Results, System, All) show the widgets for one job in your own arrangement; the last tab is remembered, and arrow keys move between tabs |
| Releases & updates | **Update** fetches the latest `main`, runs the full gate, builds and installs with a stage-by-stage progress bar and time left, while showing that the bots keep playing; each process hot-swaps between turns. A failure names the stage and changes nothing. **Roll back to a saved build** uses the same bar; a build that cannot read the compressed databases is marked instead of offered |
| Host check | Read-only facts about this machine (CPU and microcode, huge pages, memory, free space on the SSD and the archive disk, SSD TRIM), each marked ok, attention or context, with the exact command to fix anything off |
| Arrange widgets | Every panel is a widget: **Arrange widgets** (workspace bar) shows a bar on each; drag it (mouse or touch) to any column and position, or use the arrows; hide a widget and bring it back from the bar; **Reset layout** restores the default. The layout is saved per browser and survives releases (new panels land in their default column) |
| Header tabs | Each bot with a live status dot; click to focus it |
| Overview | Table stack, net winnings, hands played, decision latency |
| Signal rail | Decision time, sampling error, strategy source, training state |
| Live table | Seats, stacks, bets, dealer button, board, pot, action labels and countdown; Start/Pause/Stop. Arena (default) follows OpenPoker's charcoal oval, black/red rail, cream cards and dark seat plates, with card deals, chip bets, acting-seat pulses and result-driven winner/payout effects. Felt and Midnight remain available. Selection applies to the focused table and Watch all, persists in this browser, and respects reduced motion. Tables adapt to their panel width, including moved widgets and fleet tiles; narrow panels use two seat rows and a two-column decision-stat layout to keep cards, bets and values readable. |
| Range explorer | Each live opponent's estimated range at our last decision as a 13×13 heatmap (brighter = more likely per combo), our equity against each range and against all of them, their most likely hands |
| Live action | Every action, board card, decision and result pushed the instant it happens (this bot / all bots) |
| Leak finder | All recorded hands analysed: advice ranked by severity, costliest and best lines (our action sequence per street with 95% intervals), how hands ended, results by position, head-to-head by opponent, the exploitation check (do opponents bet into us more while we over-fold vs MDF?) and the cumulative trend |
| Self-calibration | Predicted vs realized value per spot and the corrections the bot applies to itself |
| Decision strip | Latest action and reasoning, equity, pot odds, hand category, opponent model inputs |
| Why this move | Bar chart of the expected chips of every option considered — hover for fold odds |
| Fleet race | Cumulative profit of every bot on one chart; click names to toggle |
| Starting hand library | 13×13 grid of hands the live policy opens by position (BB: defends vs a button open) |
| Recent hands | Last hands with cards and net; click to replay |
| Hand replay | Step through a hand or press play (1x/2x/4x) |
| Opponent intelligence | Every opponent's style, sample size and key stats; click for the full profile |
| Season race | Live leaderboard with our bots highlighted, rank movement, gap to #1, time left |
| Performance | Net winnings chart (showdown vs non-showdown), win rate and 95% interval |
| Champion profile | Live strategy parameters and promotion lineage |
| Season ledger | Server-confirmed score, balances, rebuys and hands |
| Autonomy / Experiments | Learner state, neural model status, every evaluated challenger |
| Highlights | Achievements, biggest wins, monster hands, bad beats — click to replay |
| Activity log | Live events for the selected bot |
| TV mode | One full-screen table chosen by the auto-director, with play-by-play commentary; the header's TV button (or `#tv`). The same view is what the public TV listener serves (`SVANBOT_TV_PORT`), with the commentary, the scouting-report buttons and the exit link off, because its audience has no operator token |
| Docs | This documentation |
| Bot setup (Settings → Open bot setup, or `#setup`) | Add, rename, reorder, switch off or remove bots, paste and check API keys, set the maximum buy-in and table seeking; saves to `.env` and restarts the fleet between turns |

Big wins pop up as toasts in the corner.

---

## 11. HTTP API reference

All endpoints except `/api/health` and `/api/session` require the operator session cookie when an
operator token is configured.

| Method | Path | Returns |
|--------|------|---------|
| POST | `/api/session` | Sets the session cookie when `{"token": ...}` matches |
| GET | `/api/state` | Full snapshot: bots, training, logs, config |
| GET | `/api/events` | Server-sent events: `action`, `board`, `decision`, `result` instantly; `state` snapshots after changes (max 1/s) or every 5 s |
| GET | `/api/bots/{slot}/hands` | Recent hands of a bot |
| GET | `/api/bots/{slot}/hands/{hand_id}` | Replay events for one hand |
| GET | `/api/bots/{slot}/opponents` | Opponent profiles (current table first) |
| GET | `/api/bots/{slot}/opponents/{name}` | One opponent with head-to-head history |
| POST | `/api/bots/{slot}/command` | `{"command": "start" | "pause" | "stop"}` |
| POST | `/api/training/command` | Queue a learner command |
| GET | `/api/setup` | Configured bots (names, `…last4` key hints, enabled), buy-in, seek rank, whether saving is allowed and restarts automatically — never keys |
| POST | `/api/setup/verify-key` | `{"key": ...}` or `{"slot": n}`: checks the key with openpoker.ai `GET /me` and `/season/me`; returns only the registered `name` and `pro_tier` |
| POST | `/api/setup` | `{"bots": [{"name", "key"? or "from_slot"?, "enabled"}], "buy_in", "seek_top_rank"}`: validated, written to `.env` (mode 600, previous copy in `artifacts/.env.previous`); `restart` is `scheduled` under the supervisor, else `manual`. Refused (403) without an operator token unless the dashboard is bound to loopback and addressed as 127.0.0.1/localhost |
| GET | `/api/starting-hands` | Opening guide rows |
| GET | `/api/leaderboard` | Season leaderboard with our bots flagged |
| GET | `/api/fleet` | Cumulative profit series per bot |
| GET | `/api/highlights` | Achievements and memorable hands |
| GET | `/api/calibration` | Self-calibration table |
| GET | `/api/analysis` | Leak finder report (cached five minutes) |
| GET | `/api/bots/{slot}/ranges` | Range explorer for the bot's last decision |
| GET | `/api/hand-classes` | The 169 hand-class names in engine order |
| GET | `/api/raw` | Internal live state (debugging) |
| GET | `/api/health` | `{"ok": true, "public": false, "version": "10.0.0", "commit": "…"}` — `public` says which listener answered |

With `SVANBOT_TV_PORT` set, a second listener carries only `/api/health` (answering `"public": true`),
`/api/tv`, `/api/tv/events` and the built page; every other `/api/` path answers **404** there, not
401. See [`docs/SPEC-dashboard.md`](SPEC-dashboard.md) for the payload and
[`docs/OPERATIONS.md`](OPERATIONS.md) for how to host it.

---

## 12. Configuration reference

| Key | Default | Meaning |
|-----|---------|---------|
| `SVANBOT_API_KEY`, `SVANBOT_MAIN_NAME` | — | Main bot |
| `OPENPOKER_API_KEY_n`, `BOT_n_NAME` | — | Extra bots (n = 2..10) |
| `SVANBOT_SERVER_URL` | `wss://openpoker.ai/ws` | WebSocket endpoint |
| `SVANBOT_REST_BASE` | `https://api.openpoker.ai/api` | REST endpoint |
| `SVANBOT_BUY_IN` | 5000 | Maximum buy-in (1,000–5,000) |
| `SVANBOT_SEEK_TOP_RANK` | 30 | Prefer tables with a current-season top-N bot (0 disables) |
| `SVANBOT_BANK_STACK_BB` | 1000 | Bank winnings: at this table stack (big blinds), leave after the hand and rejoin with a fresh buy-in (0 disables). Banking early builds the balance while it is small |
| `SVANBOT_BANK_UNTIL_CHIPS` | 500000 | Stop banking once the bot's total chips (off-table balance plus table stack) reach this (0 = no ceiling): a bot that far ahead keeps a deep stack on the table for the big hands |
| `SVANBOT_EXPORT_CAP` | 20000 | Deepest hand the server exports per bot for Free keys; Pro keys (`pro_tier`) are unlimited automatically, and every ended season backfills on its own — keys in `.env` are the only setup |
| `SVANBOT_WEB__HOST` / `SVANBOT_WEB_PORT` | 127.0.0.1 / 5000 | Dashboard address |
| `SVANBOT_WEB__OPERATOR_TOKEN` | — | Dashboard password |
| `SVANBOT_TV_HOST` / `SVANBOT_TV_PORT` | 127.0.0.1 / 0 (off) | Public TV listener — an unauthenticated table view; binding beyond loopback publishes it. Bind loopback and reverse-proxy it |
| `SVANBOT_RUNTIME__DRY_RUN` | false | `true` starts without connecting |
| `SVANBOT_RUNTIME__FLEET_SIZE` | all keys | Limit how many bots run |
| `SVANBOT_ONLY` | — | Comma-separated bot names to run |
| `LEARNER_THREADS` | auto | Learner thread override |
| `SVANBOT_ARCHIVE_DIR` | `artifacts/archive` | Archive root for `archive run` (point it at a second disk) |
| `SV10_TABLES_DIR` | `artifacts/tables` | Exact board-strength tables (`tables build` creates them) |
| `CALIBRATE_CORPUS` | — | Set to include corpus hands when measuring the range fit |
| `SVANBOT10_ROOT` | current dir | Project root for binaries |

---

## 13. Operations and troubleshooting

| Symptom | Where to look / what to do |
|---------|---------------------------|
| A bot shows offline | `artifacts/logs/svanbot10.log`; connection errors retry with backoff automatically |
| `auth_failed` | The API key was regenerated or mistyped — update `.env`, restart |
| Bot stuck "connecting" / lobby | The watchdog rejoins after 2 minutes; check for `insufficient_funds` or rebuy cooldowns |
| Rejected actions in Runtime health | Should stay at 0; check the log for the rejection code |
| Losing streak | Run `review all 10`; results swing hard at small samples — judge on thousands of hands |
| Learner never promotes | Normal when the champion is already good; see Experiments for intervals |
| Dashboard asks for a token | Use `SVANBOT_WEB__OPERATOR_TOKEN` from `.env` |
| The public TV shows a login form, or a 401 | Wrong port: the TV answers 404 for a dashboard route, never 401. `curl -s <host>/api/health` must say `"public":true`; a 200 from `/api/state` means the dashboard itself was published (`docs/OPERATIONS.md`) |
| Restore data | Stop, copy a file from `artifacts/backups/` to `artifacts/svanbot10.db`, start; older: `archive restore NAME --to DIR` then copy |
| Why did a bot make that big call? | `review replay id=N` (or the newest 20) re-runs the recorded decision; the dashboard hand view shows the EVs |
| Pressure / slowness | `scripts/status.sh` prints CPU/I-O/memory pressure; the learner yields automatically |
| Disk tidiness | `scripts/clean.sh` (report) — it also runs after every nightly archive |

Logs: `artifacts/logs/svanbot10.log` (fleet), `artifacts/logs/learner.log` (learner), `artifacts/logs/monitor.log` (results alerts: big losses, nemesis opponents, stalls).

---

## 14. Development

| Command | Purpose |
|---------|---------|
| `scripts/check.sh full` | The anti-regression gate: rustfmt, clippy with warnings as errors, dependency licenses and advisories, all tests, dashboard type check (the pre-commit hook runs the quick `commit` mode) |
| `scripts/check.sh deep` | The gate with property and fuzz tests at 100,000 cases |
| `cargo test --release --workspace` | All tests (evaluator, engine, model, tracker replay of real captured frames, legal-action mapping, neural net, guide, hardware, property tests, frame fuzzer) |
| `cargo test --release -p sv10-core -- --ignored` | Exhaustive 7-card evaluator check (134M hands) |
| `./target/release/sim 1500 16 100 all` | Policy vs archetype tables with behaviour stats |
| `./target/release/probe` | Candidate EVs for canonical spots |
| `./target/release/probe --bench` | Evaluator and decision benchmarks |
| `SIM_B='{"call_margin":0.005}' ./target/release/sim paired 12 600` | Paired A/B benchmark (fixed seed: identical result across speed-only changes) |
| `./target/release/review leaks` | Leak finder report as JSON |
| `./target/release/ingest archive <dir> [--dry-run]` | Import archived frames into the corpus |
| `./target/release/ingest phh <dir> <source> [--dry-run]` | Import a PHH hand-history tree into the corpus as `<source>` |
| `SVANBOT10_ROOT=<copy> ./target/release/ingest neural-ab <source> [seeds]` | Neural held-out log-loss with and without a corpus source (run on a copy of `artifacts/`) |
| `./target/release/calibrate [samples]` | Refit the range model to showdowns |
| `scripts/portable.sh` / `scripts/install.sh` | Portable x86-64-v2/v3 bundle and its CPU-matched install |
| `scripts/vendor.sh` | Vendor dependencies for offline builds |
| `./target/release/tables preflop-data` | Regenerate the compiled-in preflop tables after an equity-sampling change (a test fails until you do) |
| `cd web && npm run build` | Rebuild the dashboard |
| `python3 scripts/notices.py [--sbom FILE]` | Regenerate third-party notices (and a CycloneDX SBOM) |

Specifications: `docs/SPEC-protocol.md` (openpoker protocol as implemented), `docs/SPEC-data.md`
(databases, formats, backups), `docs/SPEC-learner.md` (search and promotion gates); runbook and
fault drills in `docs/OPERATIONS.md`; `docs/LESSONS.md` (mistakes already paid for — read first);
`docs/RELEASE.md` (release process); `cargo doc --no-deps --workspace` documents every public item.

Planning lives in the issue tracker, one ticket per decision:
<https://github.com/SvanLabs/SvanBot/issues>. The authoritative protocol reference is
<https://docs.openpoker.ai/llms-full.txt>.

---

## 15. Glossary

| Term | Meaning |
|------|---------|
| bb / bb/100 | Big blind (20 chips) / big blinds won per 100 hands |
| EV | Expected value in chips |
| Equity | Share of the pot a hand wins at showdown against a range |
| Range | Weighted set of hole-card combinations a player may hold |
| SPR | Stack-to-pot ratio |
| VPIP / PFR | Voluntarily put in pot / preflop raise frequency |
| WTSD | Went to showdown |
| C-bet | Continuation bet by the preflop raiser |
| Paired evaluation | Two strategies playing identical cards so luck cancels |
| Champion / challenger | Current live parameters / a candidate being evaluated |
| Shrinkage | Pulling small-sample stats toward the population average |

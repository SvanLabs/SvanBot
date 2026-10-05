# When everything has gone to hell

One page for the worst day: the fleet is down, idle, or losing and you don't know why. Work top to
bottom, paste the blocks as-is, stop at the first section that explains what you see. Each section
ends with where the full story lives.

## 0. Is anything alive? (60 seconds)

```sh
scripts/status.sh
curl -s localhost:5000/api/health
tail -5 artifacts/releases.log
```

- `status.sh` shows supervisors, per-bot mode/score/rank, learner/analyst liveness. If processes
  are missing, jump to [hell 3](#3-everything-is-down).
- `/api/health` `commit` is the build the fleet is *playing* — not the checkout. If `commit` is not
  in `git log`, the checkout moved without a release: see `.claude/skills/verify-install/SKILL.md`
  before concluding anything.
- `releases.log` newest-last tells you the last thing installed and whether it finished.

## 1. Are we seated and scoring?

```sh
scripts/status.sh 2>&1 | head -30
```

- `mode` should say `playing` with rising `hands`. `lobby`/`connecting` for minutes means the bot
  cannot get or hold a seat: check `last_error`, then [hell 2](#2-one-bot-is-stuck-not-the-fleet).
- Score falling while hands rise is a strategy problem, not an outage — don't restart it, read it:
  `review recent` answers "are we losing *now*" and `review allin-luck` separates luck from edge
  (`docs/GUIDE.md`). Restarting a losing-but-healthy bot just pays rejoin costs for nothing.
- Rank sliding while chips are flat means rivals moved, not you (`docs/CONTEXT.md`, *rank slide*).

## 2. One bot is stuck, not the fleet

```sh
grep -i "error\|rejected\|cooldown\|throttled\|fatal" artifacts/release.log | tail -20
```

Common, in order of likelihood:

- **Rebuy cooldown.** After a bust the bot waits: 5 minutes Free, 2 minutes Pro, plus 30 s grace.
  There is no dashboard countdown — idleness here is normal. Wait it out; do not restart.
- **`already_seated` / resync loop.** A new socket took over the old one; the bot resyncs from
  sequence 0 and resumes. Normal after any deploy or reconnect (`docs/SPEC-protocol.md`).
- **`auth_failed` / close 4001.** The API key is dead or wrong. One or two in a row retry on the
  backoff path (a blip heals itself); the third within 10 minutes parks the bot and nothing
  reconnects until the key in `.env` is fixed. Fix the key, restart that bot only.
- **`flood_kick`.** 20+ rejected actions in 5 seconds got the bot removed. It rejoins by itself;
  if it recurs, the policy is sending illegal actions — file an issue with the log, don't keep
  restarting into the kick.
- **Three missed hands.** Away 3 consecutive hands and the server removes the seat. The bot
  re-queues on its own; check *why* it missed (decision timeouts point at CPU starvation, and the
  fix is the compute profile, not restarts).

## 3. Everything is down

```sh
ls artifacts/hold-until 2>/dev/null && echo "HELD - a stop hold is active"
scripts/keepalive.sh 2>&1 | tail -5
scripts/start.sh
sleep 30 && scripts/status.sh 2>&1 | head -20
```

- A leftover `stop.sh --hold` (default 30 min) silences keepalive through the very window you need
  it. `artifacts/hold-until` present means *you* told the fleet to stay down.
- `keepalive.sh` respects holds and release locks; it restarts what failed, it doesn't diagnose.
- `start.sh` brings fleet + learner + analyst + dashboard. Give it 30 seconds, then re-run the
  [60-second check](#0-is-anything-alive-60-seconds). If bots connect and immediately die, read the
  supervisor logs before looping the start command.

## 4. An update broke it

```sh
tail -30 artifacts/release.log
scripts/update.sh --rollback <commit>
```

- `release.log` is fresh per run: the failing stage is at the bottom. A red gate keeps the old
  build installed — the fleet plays the last good install until a release *succeeds*.
- Roll back to the last commit in `artifacts/releases.log`, then confirm `/api/health` `commit`
  equals it. Rollback hot-swaps like any install (`scripts/rollback.sh` refuses snapshots the
  installed build cannot read).
- Never "fix forward" on the live box during a season: cut a branch, land a patch through the
  normal path (`.claude/skills/ship-patch/SKILL.md`), install via `scripts/release.sh`.

## 5. Data looks corrupt

```sh
python3 scripts/test.py store
ls -t artifacts/backups/ | head -5
```

- If the store tests fail, stop the fleet *first* (`scripts/stop.sh`) — a running writer will fight
  any repair. Restore from `artifacts/backups/` into a scratch path, verify, then move it in;
  `.claude/skills/restore-store/SKILL.md` is the exact procedure. Never restore into
  `artifacts/` directly and never use immutable mode on a live database (`docs/SPEC-data.md`).
- Missing data is not a measured zero: an empty store on a fresh clone is the normal first-boot
  state, and gaps in hand history are absence of evidence, not evidence of absence.

## 6. Season boundary went wrong

```sh
scripts/season-check.sh after <label>
```

- At the boundary every bot resets and rejoins; 15 minutes of frozen table moves, 5 minutes of no
  new hands, and rejoin backoff are all *normal*. An hour into the new season, `season-check.sh`
  confirms the season id changed, mode is `playing`, and hands are accumulating
  (`docs/OPERATIONS.md`).
- Do not ship, restart, or touch Settings in the last hour before the end or the first hour after
  the start. Every one of those costs the exact hands that decide rank.

## Prompts that work

Paste one of these to an agent with repo access, as-is. Each states the symptom, the read-only
rule, and what a good answer contains.

```text
Diagnose, read-only, no restarts, no config changes: bot <NAME> has been in
<lobby/connecting> for <N> minutes. Use scripts/status.sh, the supervisor logs
and the last 50 lines of artifacts/release.log. Tell me: seated or not, last
error class (auth/cooldown/flood/ban/missed-hands), whether it self-heals and
by when, and the single command that confirms recovery. Evidence before theory.
```

```text
Verify-install: report the commit the fleet is playing (/api/health), the
checkout HEAD, and the newest artifacts/releases.log entry, per
.claude/skills/verify-install/SKILL.md. If they disagree, say which one is
authoritative for measurements and what to run to reconcile them. Change nothing.
```

```text
Pre-boundary check: season ends at <ISO>. Confirm deploy moratorium timing,
no hold file, release locks free, season poller fresh, watchdog green, backups
current. List anything that would interrupt play in the final 2 hours, with the
command that proves each claim. Read-only except scripts/season-check.sh before.
```

```text
Rank-slide triage for flagship <NAME> over the last <N> hours: separate our own
chip change from rivals' movement using the leaderboard and review recent. Is
this our leak, their heater, or both? End with: hold course, tighten risk, or
open an issue — and which review command backs the call.
```

## What not to do, ever

- Don't build into `target/release` on a running box (`AGENTS.md`). Trial builds go to
  `target/dev`.
- Don't edit the fleet checkout to "try something" (`.claude/skills/ship-patch/SKILL.md`).
- Don't regenerate golden snapshots to turn a red test green (`docs/CONTRIBUTING.md`).
- Don't loosen a learning or promotion gate to make an experiment fit (`docs/SPEC-learner.md`).

Full stories live in `docs/OPERATIONS.md` (runbook), `docs/GUIDE.md` (dashboard),
`docs/ARCHITECTURE.md` (processes and decision path), and `docs/LESSONS.md` (why each rule
exists). This page is triage; those are treatment.

#!/usr/bin/env python3
"""Live wins/losses check (0153): one screen of real numbers plus FLAG lines for what needs action.

  scripts/fleet-check.py [--hours 2]

Read-only. Flags: a bot with no stored hand for 15 minutes (outage), a window rate far under the
season-13 #1 line (73 chips/hand) with enough hands to mean something, single hands losing more than
250 bb, and WARN/ERROR lines in the fleet log since the window start. Exit 1 when anything is flagged.
"""
import argparse
import datetime as dt
import json
import math
import re
import sqlite3
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET_CHIPS_PER_HAND = 73.0
BIG_LOSS_BB = 250
STALE_MIN = 15


# A hand running when the fleet hot-swaps or restarts (release, setup save, stop/start) finishes after the old process
# exited, so the new one never records it: a swap gap, not a foreign client (0151).
SWAP_BEFORE = dt.timedelta(minutes=5)
SWAP_AFTER = dt.timedelta(minutes=1)
SWAP_LINE = re.compile(r"(\d{4}-\d\d-\d\d \d\d:\d\d:\d\d\.\d+[+-]\d\d:\d\d)\s+INFO\s+(?:hot swap: exiting|svanbot10 starting)")


def swap_times(lines) -> list:
    """UTC times of the fleet's hot-swap exits in log lines."""
    out = []
    for line in lines:
        m = SWAP_LINE.search(re.sub(r"\x1b\[[0-9;]*m", "", line))
        if m:
            out.append(dt.datetime.fromisoformat(m.group(1)).astimezone(dt.timezone.utc))
    return out


def at_swap(started_at: str, swaps) -> bool:
    """Whether a hand that started at `started_at` (ISO, UTC) was in progress at a hot swap."""
    t = dt.datetime.fromisoformat(started_at[:26] + "+00:00") if "+" not in started_at[19:] else dt.datetime.fromisoformat(started_at)
    return any(s - SWAP_BEFORE <= t <= s + SWAP_AFTER for s in swaps)


def name_groups(kv_lists, aliases: str, names) -> dict:
    """Map every bot name to a group id: names linked by an API-key record (`bot.names.*` kv
    lists) or by SVANBOT_ALIASES (new:old,...) are one bot across renames."""
    parent = {}

    def find(x):
        parent.setdefault(x, x)
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    def union(a, b):
        parent[find(a)] = find(b)

    for n in names:
        find(n)
    for raw in kv_lists:
        try:
            lst = json.loads(raw)
        except ValueError:
            continue
        for other in lst[1:]:
            union(lst[0], other)
    for pair in (aliases or "").split(","):
        if ":" in pair:
            new, old = (x.strip() for x in pair.split(":", 1))
            if new and old:
                union(new, old)
    return {n: find(n) for n in parent}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--hours", type=float, default=2.0)
    a = ap.parse_args()
    now = dt.datetime.now(dt.timezone.utc)
    since = (now - dt.timedelta(hours=a.hours)).isoformat()
    db = sqlite3.connect(f"file:{ROOT}/artifacts/svanbot10.db?mode=ro", uri=True)
    season_start = db.execute("select value from kv where key='season.current'").fetchone()
    start = "2026-09-20T11:46"
    if season_start:
        try:
            start = json.loads(season_start[0]).get("start_date", start) or start
        except (ValueError, AttributeError):
            pass
    flags = []
    print(f"fleet check {now:%Y-%m-%d %H:%M} UTC, window {a.hours:g} h, season since {start[:16]}")
    print(f"{'bot':11} {'win hands':>9} {'win net':>9} {'chips/h':>8} {'± 95%':>7} | {'season hands':>12} {'season net':>11} {'chips/h':>8} | last hand")
    tot = [0, 0, 0, 0]
    # A renamed bot is one bot: group its names (API-key records and SVANBOT_ALIASES; only that
    # line of .env is read) and report the group under its most recently active name.
    stored = [r[0] for r in db.execute("select distinct bot from hands")]
    kv_lists = [r[0] for r in db.execute("select value from kv where key like 'bot.names.%'")]
    alias_line = ""
    try:
        alias_line = next((l.split("=", 1)[1].strip() for l in open(ROOT / ".env") if l.startswith("SVANBOT_ALIASES=")), "")
    except OSError:
        pass
    groups = name_groups(kv_lists, alias_line, stored)
    members = {}
    for n in stored:
        members.setdefault(groups[n], []).append(n)
    ordered = []
    for names in members.values():
        lasts = {n: db.execute("select max(ended_at) from hands where bot=?", (n,)).fetchone()[0] or "" for n in names}
        ordered.append((max(names, key=lambda n: lasts[n]), names))
    for bot, names in sorted(ordered):
        marks = ",".join("?" * len(names))
        w = [r[0] for r in db.execute(f"select net from hands where bot in ({marks}) and ended_at>=? and net is not null", (*names, since))]
        s = db.execute(f"select count(*), coalesce(sum(net),0) from hands where bot in ({marks}) and ended_at>=?", (*names, start)).fetchone()
        last = db.execute(f"select max(ended_at) from hands where bot in ({marks})", names).fetchone()[0]
        n, net = len(w), sum(w)
        m = net / n if n else 0.0
        se = math.sqrt(sum((x - m) ** 2 for x in w) / (n - 1)) / math.sqrt(n) if n > 1 else 0.0
        age = (now - dt.datetime.fromisoformat(last[:26] + "+00:00")).total_seconds() / 60 if last else 1e9
        print(f"{bot:11} {n:9d} {net:+9d} {m:8.1f} {1.96 * se:7.1f} | {s[0]:12d} {s[1]:+11d} {s[1] / max(s[0], 1):8.1f} | {age:5.0f} min ago")
        tot[0] += n; tot[1] += net; tot[2] += s[0]; tot[3] += s[1]
        if age > STALE_MIN:
            flags.append(f"FLAG outage: {bot} has no stored hand for {age:.0f} min")
        if n >= 150 and m + 1.96 * se < 0:
            flags.append(f"FLAG losing: {bot} {m:+.1f} chips/hand over {n} hands, 95% upper bound below 0")
    print(f"{'FLEET':11} {tot[0]:9d} {tot[1]:+9d} {tot[1] / max(tot[0], 1):8.1f} {'':7} | {tot[2]:12d} {tot[3]:+11d} {tot[3] / max(tot[2], 1):8.1f} | target {TARGET_CHIPS_PER_HAND:g}")
    # Hands the server export shows our bots playing that this fleet never saw: something else acts
    # for them while we are disconnected (0151: 2,179 such hands in season 13, -52,391 chips, a
    # strategy folding 40-54% preflop against our ~2%). Exported with a delay, so a window can lag.
    hist = ROOT / "artifacts/history.db"
    try:
        swaps = swap_times(subprocess.run(["grep", "-a", "-E", "hot swap: exiting|svanbot10 starting", str(ROOT / "artifacts/logs/svanbot10.log")],
                                          capture_output=True, text=True).stdout.splitlines())
    except OSError:
        swaps = []
    if hist.exists():
        hdb = sqlite3.connect(f"file:{hist}?mode=ro", uri=True)
        hdb.execute(f"attach 'file:{ROOT}/artifacts/svanbot10.db?mode=ro' as s")
        # The export JSON is stored compressed since 0229; its profit has a column (filled at insert
        # and by the compaction). A row still in text form, or a database from before, reads the JSON.
        has_profit = hdb.execute("select 1 from pragma_table_info('raw') where name = 'profit'").fetchone()
        profit = ("coalesce(r.profit, case when typeof(r.json) = 'text' and json_valid(r.json) then json_extract(r.json, '$.profit') end, 0)"
                  if has_profit else "coalesce(json_extract(r.json, '$.profit'), 0)")
        for label, since_ts in (("window", since), ("season", start)):
            missing = hdb.execute(
                f"select r.bot, r.started_at, {profit} from raw r "
                # By hand id alone: the export labels a renamed bot's older hands with its new name.
                "left join s.hands h on h.hand_id = r.hand_id "
                "where r.started_at >= ? and h.hand_id is null", (since_ts,)).fetchall()
            gaps = [m for m in missing if at_swap(m[1], swaps)]
            per_bot = {}
            for b, _, p in (m for m in missing if not at_swap(m[1], swaps)):
                c, t = per_bot.get(b, (0, 0))
                per_bot[b] = (c + 1, t + p)
            n = sum(c for c, _ in per_bot.values())
            if gaps:
                print(f"unrecorded at a hot swap ({label}): {len(gaps)} hands, {sum(g[2] for g in gaps):+d} chips (not flagged)")
            if n:
                detail = ", ".join(f"{b} {c} ({p:+d})" for b, (c, p) in sorted(per_bot.items()))
                print(f"played without us ({label}): {n} hands, {sum(p for _, p in per_bot.values()):+d} chips: {detail}")
                if label == "window":
                    flags.append(f"FLAG played without us: {n} hands in the window ({detail}); see 0151")
    bb = 20
    for bot, hid, net, hole, board in db.execute(
        "select bot, hand_id, net, hole, board from hands where ended_at>=? and net<=? order by net", (since, -BIG_LOSS_BB * bb)
    ):
        flags.append(f"FLAG big loss: {bot} {net:+d} ({net / bb:+.0f} bb) hand {hid[:8]} {hole} on {board} — check with review")
    log = ROOT / "artifacts/logs/svanbot10.log"
    try:
        tail = subprocess.run(["tail", "-n", "4000", str(log)], capture_output=True, text=True).stdout
    except OSError:
        tail = ""
    cutoff = (now - dt.timedelta(hours=a.hours)).astimezone()
    warn = {}
    for line in tail.splitlines():
        line = re.sub(r"\x1b\[[0-9;]*m", "", line)
        m = re.match(r"(\d{4}-\d\d-\d\d \d\d:\d\d:\d\d)\.\d+([+-]\d\d:\d\d)\s+(WARN|ERROR)\s+(.*)", line)
        if not m:
            continue
        ts = dt.datetime.fromisoformat(m.group(1) + m.group(2))
        if ts < cutoff:
            continue
        key = re.sub(r"[0-9a-f]{8}-[0-9a-f-]{27}|\d+", "N", m.group(4))[:110]
        warn[(m.group(3), key)] = warn.get((m.group(3), key), 0) + 1
    for (level, key), c in sorted(warn.items(), key=lambda x: -x[1]):
        print(f"{level} x{c}: {key}")
        if level == "ERROR" or c >= 10:
            flags.append(f"FLAG log: {level} x{c}: {key}")
    try:
        st = subprocess.run([str(ROOT / "scripts/status.sh")], capture_output=True, text=True, timeout=60).stdout
        for line in st.splitlines():
            if re.search(r"score|running|stopped|not running", line):
                print("status:", line.strip())
            if re.search(r"^\s*\S+\s+(offline|error|stopped)\s", line):
                flags.append(f"FLAG status: {line.strip()}")
    except (OSError, subprocess.TimeoutExpired):
        flags.append("FLAG status: scripts/status.sh failed")
    for f in flags:
        print(f)
    print("OK" if not flags else f"{len(flags)} flag(s)")
    return 1 if flags else 0


if __name__ == "__main__":
    raise SystemExit(main())

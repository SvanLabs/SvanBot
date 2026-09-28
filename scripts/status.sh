#!/usr/bin/env bash
cd "$(dirname "$0")/.."
if [ -f artifacts/fleet.pids ]; then
  while IFS=: read -r name sup; do
    if kill -0 "$sup" 2>/dev/null; then echo "fleet $name supervisor running (pid $sup)"; else echo "fleet $name supervisor NOT running"; fi
  done < artifacts/fleet.pids
  for p in artifacts/head.pid artifacts/worker-*.pid; do
    n="$(basename "$p" .pid)"
    if [ -f "$p" ] && kill -0 "$(cat "$p")" 2>/dev/null; then echo "sv10-bot $n running (pid $(cat "$p"))"; else echo "sv10-bot $n NOT running"; fi
  done
elif [ -f artifacts/bot.pid ] && kill -0 "$(cat artifacts/bot.pid)" 2>/dev/null; then
  echo "sv10-bot running (pid $(cat artifacts/bot.pid))"
else
  echo "sv10-bot NOT running"
fi
set -a; [ -f .env ] && . ./.env; set +a
PORT="${SVANBOT_WEB_PORT:-5000}"
HASH=$(python3 -c 'import hashlib, os; print(hashlib.sha256(("svanbot10:" + os.environ.get("SVANBOT_WEB__OPERATOR_TOKEN", "")).encode()).hexdigest())')
curl -s -m 5 -H "Cookie: svan_session=${HASH}" "http://127.0.0.1:${PORT}/api/raw" | python3 -c '
import json, sys
d = json.load(sys.stdin)
print("SvanBot version:", d.get("version", "?"))
print("known opponents:", d["known_opponents"])
for b in d["bots"]:
    s = b.get("season") or {}
    print("%12s %10s hands %5d net %7d rej %3d score %s rank %s %s" % (b["name"], b["mode"], b["session_hands"], b["session_net"], b["rejections"], s.get("score", "?"), s.get("rank", "?"), b["last_error"] or ""))
# The newest fleet-level error: a failing second-disk mirror lands here (0292), so a status run
# shows it without grepping artifacts/logs.
for line in d.get("log", []):
    if line.get("level") == "error":
        print("last fleet error:", line.get("message"))
        break
' 2>/dev/null || echo "API not reachable on port ${PORT}"
# Host drift (0304): the same rows the dashboard's System view shows, so a status run tells the whole
# truth about swap (ours against other programs'), filesystem errors and free space.
curl -s -m 5 -H "Cookie: svan_session=${HASH}" "http://127.0.0.1:${PORT}/api/host" | python3 -c '
import json, sys
d = json.load(sys.stdin)
warn = [c for c in d.get("checks", []) if c.get("status") == "warn"]
print("host check: everything as recommended" if not warn else "host check: %d warning(s)" % len(warn))
for c in warn:
    print("  %s: %s%s" % (c.get("label") or c.get("key"), c.get("value"), (" -> " + c["advice"]) if c.get("advice") else ""))
' 2>/dev/null || true
# Per bot: hands, net and bb/100, a renamed bot's names merged under its current one (the key's name list,
# identity.rs; SvanBotV7 is SvanBotV10), each hand at its own big blind, read-only (0246).
sqlite3 "file:artifacts/svanbot10.db?mode=ro" "with names as (select json_extract(k.value, '\$[0]') as current, j.value as name from kv k, json_each(k.value) j where k.key like 'bot.names.%')
  select coalesce(n.current, h.bot), count(*), sum(h.net), round(100.0*sum(h.net*1.0/coalesce(json_extract(h.summary, '\$.bb'), 20))/count(*),1)
  from hands h left join names n on n.name = h.bot where h.net is not null group by 1 order by 1;" 2>/dev/null
for p in learner analyst; do pgrep -f "target/release/$p" >/dev/null && echo "$p running" || echo "$p NOT running"; done
sqlite3 "file:artifacts/svanbot10.db?mode=ro" "select 'decision audit 24h: ' || count(*) || ' decisions, ' || coalesce(round(100.0*sum(live_action = deep_action)/count(*),1),0) || '% same action, mean gap ' || coalesce(round(avg(gap_bb),2),0) || ' bb, queue ' || (select count(*) from audit_queue) from decision_audit where ts >= strftime('%Y-%m-%dT%H:%M:%S', 'now', '-1 day');" 2>/dev/null
[ -x target/release/archive ] && echo "archive: $(./target/release/archive list 2>/dev/null | tail -1)"
systemctl --user list-timers svanbot10-archive.timer --no-pager 2>/dev/null | sed -n 2p
# Installed units against the ones this checkout renders (#15): a unit written for another directory,
# or left behind by a `git pull` that changed the template, runs the wrong checkout and says nothing
# until it is needed. Silent unless units are installed here and they disagree.
if [ -f "${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user/svanbot10.service" ] && ! scripts/units.sh --check >/dev/null 2>&1; then
  echo "systemd units: the installed copies are not the ones this checkout renders (scripts/units.sh)"
fi
# Pressure stall information (share of time tasks waited, 60 s average): CPU above ~20% or I/O "full"
# above ~10% sustained means live play competes for the machine.
printf 'pressure avg60: cpu %s%%, io full %s%%, memory full %s%%\n' \
  "$(awk '/^some/{split($3,a,"=");print a[2]}' /proc/pressure/cpu)" \
  "$(awk '/^full/{split($3,a,"=");print a[2]}' /proc/pressure/io)" \
  "$(awk '/^full/{split($3,a,"=");print a[2]}' /proc/pressure/memory)"

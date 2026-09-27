#!/usr/bin/env bash
# Season-boundary check (0029, 0076). Run `before` shortly before a season ends and `after` about an hour
# into the next one; `after` compares and exits 1 on any failure. Results: artifacts/season-checks/.
#   scripts/season-check.sh before|after [label]
set -uo pipefail
cd "$(dirname "$0")/.."
phase="${1:?before|after}"; label="${2:-$(date +%Y%m%d)}"
dir=artifacts/season-checks; mkdir -p "$dir"
./target/release/review season-snapshot "$dir/$label-$phase.json" >/dev/null
[ -s "$dir/$label-$phase.json" ] || { echo "FAIL no snapshot written (is target/release/review current? run scripts/release.sh)"; exit 1; }
set -a; [ -f .env ] && . ./.env; set +a
HASH=$(python3 -c 'import hashlib, os; print(hashlib.sha256(("svanbot10:" + os.environ.get("SVANBOT_WEB__OPERATOR_TOKEN", "")).encode()).hexdigest())')
curl -s -m 10 -H "Cookie: svan_session=${HASH}" "http://127.0.0.1:${SVANBOT_WEB_PORT:-5000}/api/raw" | python3 -c '
import json, sys
d = json.load(sys.stdin)
out = [{"name": b["name"], "mode": b["mode"], "connected": b["connected"], "table_id": b["table_id"],
        **{k: (b.get("season") or {}).get(k) for k in ("season_id", "hands_played", "score", "rank", "chips_at_table")}} for b in d["bots"]]
json.dump(out, open(sys.argv[1], "w"), indent=1)
' "$dir/$label-$phase-live.json" || { echo "FAIL dashboard API unreachable"; exit 1; }
[ "$phase" = after ] || { echo "before snapshot written to $dir/$label-before*.json"; exit 0; }
fail=0
./target/release/review season-compare "$dir/$label-before.json" "$dir/$label-after.json" || fail=1
python3 - "$dir/$label-before-live.json" "$dir/$label-after-live.json" <<'PY' || fail=1
import json, sys
before = {b["name"]: b for b in json.load(open(sys.argv[1]))}
after = json.load(open(sys.argv[2]))
bad = 0
def check(ok, what):
    global bad
    print(("PASS " if ok else "FAIL ") + what)
    bad += not ok
for b in after:
    old = before.get(b["name"], {})
    check(b["season_id"] is not None and b["season_id"] != old.get("season_id"), f'{b["name"]}: on the new season ({str(old.get("season_id"))[:8]} -> {str(b["season_id"])[:8]})')
    check(b["connected"] and b["mode"] == "playing", f'{b["name"]}: playing again (mode {b["mode"]}, table {b["table_id"]})')
    check((b["hands_played"] or 0) < (old.get("hands_played") or 0), f'{b["name"]}: season hands reset ({old.get("hands_played")} -> {b["hands_played"]})')
sys.exit(1 if bad else 0)
PY
echo "season check $label: $([ $fail = 0 ] && echo PASSED || echo FAILED)"
exit $fail

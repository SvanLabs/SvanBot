#!/usr/bin/env python3
"""One-claim rule for the GitHub tracker: an agent holds at most one open ticket at a time.

Claiming three tickets and finishing none starves the frontier and hides stuck work. The rule is
simple — take one, finish or release it, then take the next — and this script enforces it:

  claims.py status [--who NAME]          open claims held by NAME (exit 1 if more than one)
  claims.py take <n> --who NAME          refuse if NAME already holds an open claim,
                                         else assign + label agent-claimed + claim comment
  claims.py release <n> --who NAME       unassign + drop agent-claimed + release comment
  claims.py stale [--hours 24]           agent-claimed issues quiet longer than H hours

Read-only commands never write. `take`/`release` support `--dry-run` to print the `gh` calls.
A claim is an open issue assigned to NAME. Maps (`wayfinder:map`) are never claims.
"""
from __future__ import annotations

import argparse
import json
import subprocess
import sys

CLAIM_LABEL = "agent-claimed"
MAP_LABEL = "wayfinder:map"


def gh(*args: str) -> str:
    out = subprocess.run(["gh", *args], capture_output=True, text=True, check=False)
    if out.returncode != 0:
        print(f"gh failed: gh {' '.join(args)}\n{out.stderr.strip()}", file=sys.stderr)
        sys.exit(2)
    return out.stdout.strip()


def open_claims(who: str) -> list[dict] | None:
    """Open non-map issues assigned to who, or None when who is not assignable."""
    out = subprocess.run(
        ["gh", "issue", "list", "--state", "open", "--assignee", who,
         "--json", "number,title,labels,updatedAt", "--limit", "200"],
        capture_output=True, text=True, check=False,
    )
    if out.returncode != 0:
        return None
    claims = []
    for issue in json.loads(out.stdout.strip() or "[]"):
        labels = [label["name"] for label in issue.get("labels", [])]
        if MAP_LABEL in labels:
            continue
        claims.append(issue)
    return claims


def cmd_status(who: str) -> int:
    claims = open_claims(who)
    if claims is None:
        print(f"Cannot list claims for '{who}' — not an assignable user; check the login.", file=sys.stderr)
        return 2
    if not claims:
        print(f"{who} holds no open claims — free to take one.")
        return 0
    for issue in claims:
        print(f"#{issue['number']} {issue['title']} (updated {issue['updatedAt']})")
    if len(claims) > 1:
        print(f"VIOLATION: {who} holds {len(claims)} open claims — release down to one.", file=sys.stderr)
        return 1
    return 0


def cmd_take(number: int, who: str, dry_run: bool) -> int:
    held = open_claims(who)
    if held is None:
        print(f"REFUSED: '{who}' is not assignable — claim with a real login.", file=sys.stderr)
        return 1
    if held:
        print(f"REFUSED: {who} already holds #{held[0]['number']} — release it first.", file=sys.stderr)
        return 1
    calls = [
        ["issue", "edit", str(number), "--add-assignee", who],
        ["issue", "edit", str(number), "--add-label", CLAIM_LABEL],
        ["issue", "comment", str(number), "--body",
         f"Claimed by {who} (one-claim rule): working this ticket now, releasing on resolve or when stuck."],
    ]
    if dry_run:
        for call in calls:
            print("gh " + " ".join(call))
        return 0
    for call in calls:
        gh(*call)
    print(f"#{number} claimed by {who}.")
    return 0


def cmd_release(number: int, who: str, dry_run: bool) -> int:
    calls = [
        ["issue", "comment", str(number), "--body",
         f"Released by {who}: no longer working this ticket — frontier may take it."],
        ["issue", "edit", str(number), "--remove-assignee", who],
        ["issue", "edit", str(number), "--remove-label", CLAIM_LABEL],
    ]
    if dry_run:
        for call in calls:
            print("gh " + " ".join(call))
        return 0
    for call in calls:
        gh(*call)
    print(f"#{number} released by {who}.")
    return 0


def cmd_stale(hours: float) -> int:
    from datetime import datetime, timezone
    raw = gh(
        "issue", "list", "--state", "open", "--label", CLAIM_LABEL,
        "--json", "number,title,assignees,updatedAt", "--limit", "200",
    )
    now = datetime.now(timezone.utc)
    quiet = []
    for issue in json.loads(raw or "[]"):
        updated = datetime.fromisoformat(issue["updatedAt"].replace("Z", "+00:00"))
        age_h = (now - updated).total_seconds() / 3600
        if age_h >= hours:
            names = ",".join(a.get("login", "?") for a in issue.get("assignees", []))
            quiet.append((issue["number"], issue["title"], names, age_h))
    if not quiet:
        print(f"No claimed issues quiet for {hours:g}h.")
        return 0
    for number, title, names, age_h in sorted(quiet):
        print(f"#{number} {title} — {names}, quiet {age_h:.1f}h")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest="cmd", required=True)
    p_status = sub.add_parser("status")
    p_status.add_argument("--who", required=True)
    p_take = sub.add_parser("take")
    p_take.add_argument("number", type=int)
    p_take.add_argument("--who", required=True)
    p_take.add_argument("--dry-run", action="store_true")
    p_release = sub.add_parser("release")
    p_release.add_argument("number", type=int)
    p_release.add_argument("--who", required=True)
    p_release.add_argument("--dry-run", action="store_true")
    p_stale = sub.add_parser("stale")
    p_stale.add_argument("--hours", type=float, default=24)
    args = parser.parse_args(argv)
    if args.cmd == "status":
        return cmd_status(args.who)
    if args.cmd == "take":
        return cmd_take(args.number, args.who, args.dry_run)
    if args.cmd == "release":
        return cmd_release(args.number, args.who, args.dry_run)
    return cmd_stale(args.hours)


if __name__ == "__main__":
    sys.exit(main())

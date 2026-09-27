#!/usr/bin/env python3
"""Report a missing `Generated-by:` trailer at the moment a commit is made.

`.claude/settings.json` runs this after every Bash call. It returns immediately unless the command
was a `git commit`, so the usual cost is one short-lived interpreter and no output.

**`PostToolUse`, not `PreToolUse`.** `scripts/provenance.py check` reads commits, and before the
commit exists there is no commit to read; running it first would only ever inspect the parent, which
has already passed once. After the commit lands, there is a commit worth checking, so the hook runs
then and names the trailer that commit is missing.

**It checks one commit, not the history.** `scripts/provenance.py check` takes a git revision range
and `HEAD` is not the tip commit — as a revision it means *every commit reachable from HEAD*, so a
repository whose history predates this convention reports its entire past on every commit it makes.
That is a wall of problems nobody can act on, and a gate that cries wolf is one people learn to run
past. The range below is the one commit just made.

This is an early signal, not the gate. Git's own pre-commit hook runs `scripts/check.sh commit`,
whose AI-provenance step is the same `scripts/provenance.py check` over the range CI uses; the gate
that decides whether a change merges is `scripts/check.sh full`. The git hook is not installed by
cloning — `scripts/cloud-setup.sh` installs it, by linking `scripts/pre-commit.sh` into
`.git/hooks/pre-commit` — so on a plain checkout this is the only local warning there is.

Exit 2 is what hands the message back to the agent rather than only writing it to a log.
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

# The message an agent needs is the rule and the fix, in that order; the check's own output above it
# names the commit.
ADVICE = (
    "A commit must name the system that produced it (CONTRIBUTING.md section 0; the short version is\n"
    "AGENTS.md section 0). Amend the commit just made to add a footer trailer:\n"
    "\n"
    "    Generated-by: <tool>/<model>\n"
    "\n"
    "`Generated-by: claude-code/claude-opus-5` is the shape; the tool alone is fine when the model is\n"
    "not known. It is a git trailer rather than prose so that `git log --format=%(trailers)` finds it.\n"
)


def git(root: Path, *args: str) -> subprocess.CompletedProcess[str]:
    """Run git in the project and return the finished process, failure included."""
    return subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True)


def range_of_the_tip(root: Path) -> str:
    """The revision range covering only the commit just made.

    `HEAD~1..HEAD` is that range; `HEAD` is not, because a bare revision means everything reachable
    from it. The fallback is for the first commit in a repository, which has no parent to exclude —
    `HEAD~1` does not resolve there, and the tip is the whole history anyway.
    """
    has_parent = git(root, "rev-parse", "--verify", "--quiet", "HEAD~1").returncode == 0
    return "HEAD~1..HEAD" if has_parent else "HEAD"


def main() -> int:
    try:
        event = json.load(sys.stdin)
    except (OSError, ValueError):
        return 0  # Not the JSON this expects: stay quiet rather than get in the way of the tool.

    command = (event.get("tool_input") or {}).get("command")
    if not isinstance(command, str) or "git commit" not in command:
        return 0

    root = Path(os.environ.get("CLAUDE_PROJECT_DIR") or os.getcwd())
    check = root / "scripts" / "provenance.py"
    if not check.is_file():
        return 0  # Not a SvanBot checkout, or the script moved; CI still runs the same check.

    done = subprocess.run(
        [sys.executable, str(check), "check", range_of_the_tip(root)],
        cwd=root,
        capture_output=True,
        text=True,
    )
    if done.returncode == 0:
        return 0

    # Both streams: the check reports the offending commits on stdout and its summary on stderr, and
    # an agent reading only one of them would not know which commit to amend.
    report = "\n".join(part.strip() for part in (done.stdout, done.stderr) if part.strip())
    print(f"{report}\n\n{ADVICE}", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())

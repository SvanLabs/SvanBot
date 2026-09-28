#!/usr/bin/env python3
"""Close the issues a merged pull request names (#410); run by `close-linked-issues.yml`.

GitHub interprets a closing keyword in a pull request description only when the pull request targets
the repository's *default* branch. That is `main` here, and while work landed on `dev` every pull
request went there instead, so `Closes #<issue>` — which `.github/pull_request_template.md`,
`CONTRIBUTING.md` and the maintainer workflow all ask for — made no link and closed nothing. An issue
whose fix had landed stayed open, and the maintainer's rule, "the oldest open `agent-friendly` issue
that no pull request closes yet", could pick the same work a second time.

The keywords and the references are the ones GitHub documents, so a description that closed an issue
before the default branch moved closes it again here:

    closes #10        close #10        closed #10
    fixes #10         fix #10          fixed #10
    resolves #10      resolve #10      resolved #10
    closes: #10       closes SvanLabs/SvanBot#10

A reference to another repository is left alone, and a `#<number>` that is a pull request is not
closed — GitHub links the two but closes only the issue.

  scripts/close-linked-issues.py --repo R --body-file <path>          close what it names
  scripts/close-linked-issues.py --repo R --body-file <path> --check  print, and change nothing
"""
from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

# A whole keyword, an optional colon, then the reference. `closes#10` is not a closing keyword and
# neither is `recloses #10`; both are GitHub's own rules rather than invented here. The qualified form
# is captured so another repository's issue can be recognised and left alone.
CLOSING = re.compile(
    r"\b(?:clos(?:e|es|ed)|fix(?:e[sd])?|resolv(?:e|es|ed)):?\s+"
    r"(?:([A-Za-z0-9][A-Za-z0-9._-]*/[A-Za-z0-9][A-Za-z0-9._-]*))?#(\d+)\b",
    re.IGNORECASE,
)


def references(text: str, repo: str | None = None) -> list[int]:
    """The issue numbers `text` closes, in the order they appear and each once.

    A qualified `owner/repo#N` naming anything but `repo` is not ours to close, and GitHub does not
    close it either.
    """
    found: dict[int, None] = {}
    for owner_repo, number in CLOSING.findall(text):
        if owner_repo and repo and owner_repo.lower() != repo.lower():
            continue
        found.setdefault(int(number), None)
    return list(found)


def run(*args: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["gh", *args], capture_output=True, text=True)


def kind(repo: str, number: int) -> str | None:
    """`"issue"`, `"pull_request"`, or None when no such number exists.

    A reference to nothing is not a failure: GitHub ignores one too, and a description that names a
    typo'd number should not turn the merge red.
    """
    done = run(
        "api",
        f"repos/{repo}/issues/{number}",
        "--jq",
        'if .pull_request then "pull_request" else "issue" end',
    )
    if done.returncode != 0:
        if "HTTP 404" in done.stderr:
            return None
        raise RuntimeError(done.stderr.strip() or f"gh api repos/{repo}/issues/{number}")
    return done.stdout.strip()


def main(argv: list[str]) -> int:
    body = repo = None
    check = False
    rest = list(argv)
    while rest:
        arg = rest.pop(0)
        if arg == "--body-file" and rest:
            body = rest.pop(0)
        elif arg == "--repo" and rest:
            repo = rest.pop(0)
        elif arg == "--check":
            check = True
        else:
            print(f"close-linked-issues: unknown argument {arg}", file=sys.stderr)
            return 2
    if not body or not repo:
        print(
            "close-linked-issues: usage: scripts/close-linked-issues.py --repo OWNER/NAME"
            " --body-file <path> [--check]",
            file=sys.stderr,
        )
        return 2

    numbers = references(Path(body).read_text(), repo)
    if not numbers:
        print("close-linked-issues: the description names no issue; nothing to close")
        return 0
    if check:
        for number in numbers:
            print(f"close-linked-issues: would close #{number}")
        return 0

    # Anything that stops a named issue from closing is a failure and not a note: the description
    # promised the close, and a green run that closed nothing is the state this script exists to end.
    for number in numbers:
        try:
            found = kind(repo, number)
            if found is None:
                print(f"close-linked-issues: #{number} does not exist; nothing to close")
                continue
            if found == "pull_request":
                print(f"close-linked-issues: #{number} is a pull request, which links rather than closes")
                continue
            done = run(
                "api", "-X", "PATCH", f"repos/{repo}/issues/{number}", "-f", "state=closed", "--jq", ".state"
            )
            if done.returncode != 0:
                raise RuntimeError(done.stderr.strip() or f"gh api PATCH repos/{repo}/issues/{number}")
        except RuntimeError as error:
            print(f"close-linked-issues: #{number}: {error}", file=sys.stderr)
            return 1
        print(f"close-linked-issues: closed #{number}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

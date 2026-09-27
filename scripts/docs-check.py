#!/usr/bin/env python3
"""Docs drift check (0240), run by `scripts/check.sh full`.

The live documents — the paths listed in `scripts/docs-check.live`, one per line — must name only
things that exist:

- every repository path they quote in backticks (`crates/…`, `scripts/…`, `web/…`, `docs/…`,
  `.claude/…`), with `:line` suffixes and trailing punctuation stripped; a glob or brace pattern
  must match at least one file;
- every `review <command>` and `archive <command>` they quote must be one the tool accepts (read from
  `crates/apps/bot/src/bin/review.rs` COMMANDS and the `archive` dispatch).

The list of live documents is `scripts/docs-check.live`, one path per line; edit that file to add
one. Historical records (docs/audits, docs/superpowers, closed tickets, research notes, bugs/) are
exempt: they describe the code as it was. Inside a live document, lines between
`<!-- docs-check: off … -->` and `<!-- docs-check: on -->` are skipped (e.g. an old → new path
table). Exit 1 listing every problem.

  scripts/docs-check.py            check
  scripts/docs-check.py --list     also print the documents checked
"""
from __future__ import annotations

import glob
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
# The document list is data, kept in `docs-check.live` so a change to the checked set is a one-line
# diff of that file rather than an edit inside this script: no branch here changes to add or drop a
# document, and a reviewer reads the list itself. One path per line, `#` comments allowed.
LIVE = [
    line.split("#", 1)[0].strip()
    for line in (ROOT / "scripts/docs-check.live").read_text().splitlines()
    if line.split("#", 1)[0].strip()
]
PATH = re.compile(r"`((?:\./)?(?:crates|scripts|web|docs|\.claude)/[^`\s]*)`")
TOOL = re.compile(r"`(?:\./target/release/|\./target/dev/\S*/)?(review|archive) ([a-z][a-z0-9-]*)")


def review_commands() -> set[str]:
    src = (ROOT / "crates/apps/bot/src/bin/review.rs").read_text()
    table = src[src.index("const COMMANDS"): src.index("];", src.index("const COMMANDS"))]
    return set(re.findall(r'\(\s*"([a-z0-9-]+)",', table)) | {"all"}


def archive_commands() -> set[str]:
    src = (ROOT / "crates/apps/bot/src/bin/archive.rs").read_text()
    return set(re.findall(r'Some\("([a-z-]+)"\)', src)) | set(re.findall(r'"([a-z-]+)" \|', src)) | set(re.findall(r'\| "([a-z-]+)"\)', src))


def clean(path: str) -> str:
    path = path.removeprefix("./")
    path = re.sub(r":\d+(?:[-,]\d+)*$", "", path)
    return path.rstrip(".,;:)")


def exists(path: str) -> bool:
    if "<" in path or "…" in path or "..." in path or "$" in path:
        return True  # a template (`crates/<name>`), not a path
    if "{" in path:
        m = re.match(r"(.*)\{([^}]*)\}(.*)", path)
        if m:
            return all(exists(m.group(1) + alt + m.group(3)) for alt in m.group(2).split(","))
    if any(ch in path for ch in "*?["):
        return bool(glob.glob(str(ROOT / path), recursive=True))
    return (ROOT / path).exists() or ignored(path)


def ignored(path: str) -> bool:
    """A path git ignores is a build output (`web/dist`): a clean checkout lacks it, so it counts."""
    import subprocess
    # `dir/` patterns match only directories git can see; asking for a child covers a missing one.
    for p in (path, path.rstrip("/") + "/_"):
        if subprocess.run(["git", "-C", str(ROOT), "check-ignore", "-q", "--no-index", p], capture_output=True).returncode == 0:
            return True
    return False


def main(argv: list[str]) -> int:
    review, archive = review_commands(), archive_commands()
    problems = []
    for doc in LIVE:
        p = ROOT / doc
        if not p.exists():
            problems.append(f"{doc}: listed as a live document but missing")
            continue
        if "--list" in argv:
            print(f"checking {doc}")
        checking = True
        for n, line in enumerate(p.read_text().splitlines(), 1):
            # A region may quote old paths on purpose (the before/after layout table).
            if "<!-- docs-check: off" in line:
                checking = False
            elif "<!-- docs-check: on" in line:
                checking = True
            if not checking:
                continue
            for raw in PATH.findall(line):
                path = clean(raw)
                if path and not exists(path):
                    problems.append(f"{doc}:{n}: no such path `{path}`")
            for tool, cmd in TOOL.findall(line):
                known = review if tool == "review" else archive
                if cmd not in known:
                    problems.append(f"{doc}:{n}: `{tool} {cmd}` is not a {tool} command")
    for p in problems:
        print(p)
    print(f"docs-check: {len(LIVE)} documents, {len(problems)} problem(s)", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

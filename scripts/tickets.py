#!/usr/bin/env python3
"""Ticket tool for a local-markdown tracker: one file per ticket under `<root>/issues/`, and the
map of decisions in `<root>/MAP.md`. The root is `SV10_TICKETS_ROOT`, default `.tickets/` at the
repository root.

  tickets.py frontier                open, unblocked, unclaimed tickets, by priority then id
  tickets.py list [--status S] [--label L] [--all]
  tickets.py show ID
  tickets.py new TITLE [--type T] [--priority P] [--labels a,b] [--blocked-by 0029,0075]
  tickets.py claim ID [--who NAME]
  tickets.py close ID --gist TEXT    resolve, and append the gist to the map's Decisions so far
  tickets.py lint [--fix]            schema, references, cycles, map links (exit 1 on errors)

Frontmatter: type, status, assignee, blocks, blocked_by, plus optional priority (P0..P3,
default P2), labels and created. `closed` and `resolved` both mean done; new closes write
`resolved`. A type is `<namespace>:<kind>`, kind one of task, research, grilling or prototype; the
namespace groups types and only the kind is checked. Blocking edges live on both ends; `new` and
`lint --fix` keep them symmetric.
"""
from __future__ import annotations

import argparse
import datetime as dt
import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(os.environ.get("SV10_TICKETS_ROOT", Path(__file__).resolve().parent.parent / ".tickets"))
NAMESPACE = "sv10"
TYPES = {f"{NAMESPACE}:{kind}" for kind in ("task", "research", "grilling", "prototype")}
KINDS = {t.split(":", 1)[1] for t in TYPES}
DONE = {"closed", "resolved"}
STATUSES = DONE | {"open", "out-of-scope"}
PRIORITIES = ("P0", "P1", "P2", "P3")
ORDER = ["type", "status", "priority", "assignee", "labels", "created", "blocks", "blocked_by"]
LIST_KEYS = {"blocks", "blocked_by", "labels"}


@dataclass
class Ticket:
    path: Path
    meta: dict
    body: str
    extra_order: list = field(default_factory=list)

    @property
    def id(self) -> str:
        return self.path.name[:4]

    @property
    def title(self) -> str:
        return self.path.stem[5:]

    @property
    def status(self) -> str:
        return self.meta.get("status", "")

    @property
    def done(self) -> bool:
        return self.status in DONE or self.status == "out-of-scope"

    @property
    def priority(self) -> str:
        return self.meta.get("priority") or "P2"

    def render(self) -> str:
        keys = [k for k in ORDER if k in self.meta] + [k for k in self.meta if k not in ORDER]
        lines = ["---"]
        for k in keys:
            v = self.meta[k]
            lines.append(f"{k}: [{', '.join(v)}]" if k in LIST_KEYS else f"{k}: {v}".rstrip())
        lines.append("---")
        return "\n".join(lines) + "\n" + self.body

    def save(self) -> None:
        self.path.write_text(self.render())


def known_type(ty: str) -> bool:
    """`<namespace>:<kind>` with a known kind. The namespace groups types; it is not part of the rule."""
    namespace, sep, kind = ty.partition(":")
    return bool(sep and namespace) and kind in KINDS


def parse_list(raw: str) -> list:
    raw = raw.strip()
    if raw.startswith("[") and raw.endswith("]"):
        raw = raw[1:-1]
    return [x.strip().strip("'\"") for x in raw.split(",") if x.strip()]


def load(path: Path) -> Ticket:
    text = path.read_text()
    m = re.match(r"---\n(.*?)\n---\n?", text, re.S)
    if not m:
        return Ticket(path, {}, text)
    meta = {}
    for line in m.group(1).splitlines():
        if ":" not in line:
            continue
        k, v = line.split(":", 1)
        k = k.strip()
        meta[k] = parse_list(v) if k in LIST_KEYS else v.strip()
    body = text[m.end():]
    if not body.startswith("\n"):
        body = "\n" + body
    return Ticket(path, meta, body)


def tickets() -> dict:
    out = {}
    for p in sorted((ROOT / "issues").glob("[0-9][0-9][0-9][0-9]-*.md")):
        t = load(p)
        out[t.id] = t
    return out


def find(all_t: dict, tid: str) -> Ticket:
    tid = tid.zfill(4)[:4]
    if tid not in all_t:
        sys.exit(f"tickets.py: no ticket {tid}")
    return all_t[tid]


def is_blocked(t: Ticket, all_t: dict) -> bool:
    return any(b in all_t and not all_t[b].done for b in t.meta.get("blocked_by", []))


def frontier(all_t: dict) -> list:
    open_t = [t for t in all_t.values() if t.status == "open" and not t.meta.get("assignee") and not is_blocked(t, all_t)]
    return sorted(open_t, key=lambda t: (t.priority, t.id))


def row(t: Ticket, all_t: dict) -> str:
    flags = []
    if t.meta.get("assignee"):
        flags.append(f"@{t.meta['assignee']}")
    if t.status == "open" and is_blocked(t, all_t):
        flags.append("blocked by " + ",".join(b for b in t.meta["blocked_by"] if b in all_t and not all_t[b].done))
    labels = ",".join(t.meta.get("labels", []))
    return f"{t.id} {t.priority} {t.status:<12} {t.meta.get('type', '?'):<20} {t.title}" + (f" [{labels}]" if labels else "") + (f"  ({'; '.join(flags)})" if flags else "")


def lint(all_t: dict, fix: bool) -> list:
    errors, fixed = [], set()
    for t in all_t.values():
        where = t.path.name
        if not t.meta:
            errors.append(f"{where}: no frontmatter")
            continue
        for k in ("type", "status", "assignee", "blocks", "blocked_by"):
            if k not in t.meta:
                errors.append(f"{where}: missing {k}")
        ty = t.meta.get("type", "")
        if ty and not known_type(ty):
            if fix and ty in KINDS:
                t.meta["type"] = f"{NAMESPACE}:{ty}"
                fixed.add(t.id)
            else:
                errors.append(f"{where}: type {ty!r} not one of {sorted(TYPES)}")
        if t.status not in STATUSES:
            errors.append(f"{where}: status {t.status!r} not one of {sorted(STATUSES)}")
        if "priority" in t.meta and t.meta["priority"] not in PRIORITIES:
            errors.append(f"{where}: priority {t.meta['priority']!r} not one of {PRIORITIES}")
        if "## Question" not in t.body:
            errors.append(f"{where}: no '## Question' section")
        if t.status in DONE and not re.search(r"^## (Resolution|Resolved|Answer|Outcome|Decision)", t.body, re.M | re.I) and t.id not in map_text():
            errors.append(f"{where}: done but neither a resolution section nor a map entry")
        for key, other in (("blocked_by", "blocks"), ("blocks", "blocked_by")):
            for ref in t.meta.get(key, []):
                if ref not in all_t:
                    errors.append(f"{where}: {key} names missing ticket {ref}")
                elif t.id not in all_t[ref].meta.get(other, []):
                    if fix:
                        all_t[ref].meta.setdefault(other, []).append(t.id)
                        all_t[ref].meta[other].sort()
                        fixed.add(ref)
                    else:
                        errors.append(f"{where}: {key} {ref}, but {ref} lacks {other} {t.id}")
    errors += cycles(all_t)
    for link in re.findall(r"\]\((issues/[^)#]+)", map_text()):
        if not (ROOT / link).exists():
            errors.append(f"MAP.md: broken link {link}")
    for tid in fixed:
        all_t[tid].save()
    if fixed:
        print(f"fixed {len(fixed)} ticket(s): {', '.join(sorted(fixed))}")
    return errors


def cycles(all_t: dict) -> list:
    state, errors = {}, []

    def visit(tid, stack):
        state[tid] = 1
        for nxt in all_t[tid].meta.get("blocked_by", []):
            if nxt not in all_t:
                continue
            if state.get(nxt) == 1:
                errors.append("blocking cycle: " + " -> ".join(stack[stack.index(nxt):] + [nxt]) if nxt in stack else f"blocking cycle via {nxt}")
            elif nxt not in state:
                visit(nxt, stack + [nxt])
        state[tid] = 2

    for tid in all_t:
        if tid not in state:
            visit(tid, [tid])
    return errors


_map_cache = None


def map_text() -> str:
    global _map_cache
    if _map_cache is None:
        p = ROOT / "MAP.md"
        _map_cache = p.read_text() if p.exists() else ""
    return _map_cache


def slugify(title: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-")[:60].rstrip("-")


def cmd_new(all_t, a):
    nid = f"{max([int(i) for i in all_t] or [0]) + 1:04d}"
    blocked = [b.zfill(4) for b in parse_list(a.blocked_by or "")]
    for b in blocked:
        find(all_t, b)
    if a.priority not in PRIORITIES:
        sys.exit(f"tickets.py: priority must be one of {PRIORITIES}")
    ty = a.type if ":" in a.type else f"{NAMESPACE}:{a.type}"
    if not known_type(ty):
        sys.exit(f"tickets.py: type must be one of {sorted(TYPES)}")
    meta = {"type": ty, "status": "open", "priority": a.priority, "assignee": "",
            "labels": parse_list(a.labels or ""), "created": dt.date.today().isoformat(),
            "blocks": [], "blocked_by": blocked}
    t = Ticket(ROOT / "issues" / f"{nid}-{slugify(a.title)}.md", meta, f"\n## Question\n\n{a.question or a.title}\n")
    t.save()
    for b in blocked:
        all_t[b].meta.setdefault("blocks", []).append(nid)
        all_t[b].save()
    print(t.path.relative_to(ROOT.parent.parent) if ROOT.is_relative_to(ROOT.parent.parent) else t.path)


def cmd_close(all_t, a):
    t = find(all_t, a.id)
    t.meta["status"] = "resolved"
    t.meta["assignee"] = t.meta.get("assignee") or "agent"
    t.save()
    mp = ROOT / "MAP.md"
    text = mp.read_text()
    heading = re.search(r"(?m)^## Decisions so far[ \t]*$", text)
    if heading is None:
        sys.exit("tickets.py: MAP.md has no Decisions so far section")
    marker = "\n## Not yet specified"
    i = text.find(marker, heading.end())
    decisions = text[heading.end():i if i >= 0 else len(text)]
    indexed = re.search(rf"(?m)^- \[[^\]]+\]\(issues/{re.escape(t.path.name)}\):", decisions)
    if not indexed:
        entry = f"- [{t.path.stem}](issues/{t.path.name}): {a.gist}\n"
        text = text[:i].rstrip("\n") + "\n" + entry + text[i:] if i >= 0 else text + entry
        mp.write_text(text)
    unblocked = [o for o in all_t.values() if t.id in o.meta.get("blocked_by", []) and o.status == "open" and not is_blocked(o, all_t)]
    print(f"resolved {t.id}" + (f"; now unblocked: {', '.join(o.id for o in unblocked)}" if unblocked else ""))


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("frontier")
    ls = sub.add_parser("list")
    ls.add_argument("--status")
    ls.add_argument("--label")
    ls.add_argument("--all", action="store_true")
    sh = sub.add_parser("show")
    sh.add_argument("id")
    nw = sub.add_parser("new")
    nw.add_argument("title")
    nw.add_argument("--type", default="task")
    nw.add_argument("--priority", default="P2")
    nw.add_argument("--labels")
    nw.add_argument("--blocked-by")
    nw.add_argument("--question")
    cl = sub.add_parser("claim")
    cl.add_argument("id")
    cl.add_argument("--who", default="agent")
    cs = sub.add_parser("close")
    cs.add_argument("id")
    cs.add_argument("--gist", required=True)
    li = sub.add_parser("lint")
    li.add_argument("--fix", action="store_true")
    a = p.parse_args(argv)
    all_t = tickets()
    if a.cmd == "frontier":
        for t in frontier(all_t):
            print(row(t, all_t))
    elif a.cmd == "list":
        for t in all_t.values():
            if (a.all or not t.done or a.status) and (not a.status or t.status == a.status) and (not a.label or a.label in t.meta.get("labels", [])):
                print(row(t, all_t))
    elif a.cmd == "show":
        print(find(all_t, a.id).path.read_text(), end="")
    elif a.cmd == "new":
        cmd_new(all_t, a)
    elif a.cmd == "claim":
        t = find(all_t, a.id)
        if t.meta.get("assignee") and t.meta["assignee"] != a.who:
            sys.exit(f"tickets.py: {t.id} already claimed by {t.meta['assignee']}")
        t.meta["assignee"] = a.who
        t.save()
        print(f"claimed {t.id}")
    elif a.cmd == "close":
        cmd_close(all_t, a)
    elif a.cmd == "lint":
        errors = lint(all_t, a.fix)
        for e in errors:
            print(e)
        print(f"{len(all_t)} tickets, {len(errors)} problem(s)")
        return 1 if errors else 0
    return 0


if __name__ == "__main__":
    sys.exit(main())

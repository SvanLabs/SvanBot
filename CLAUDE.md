# SvanBot — Claude Code entry point

This file exists because Claude Code reads it first. It is deliberately short: it names the three
mistakes an agent makes here, then points at the documents that carry the rest.

**It is not a second copy of `AGENTS.md`, and it must not become one.** `AGENTS.md` is the
engineering brief and the fuller document; the rules live there once. Two documents that both
claim to state the brief drift apart, and the one an agent happens to read decides which rules it
follows — so the three below are the only overlap, and they are here because they are the ones
that are cheap to get wrong and expensive to discover.

## The three that bite

**1. Build into `target/dev`, never `target/release`.** A release build that lands in
`target/release` overwrites the binary a running fleet hot-swaps from, so an untested build reaches
live play between hands. Every script here already defaults `CARGO_TARGET_DIR` to `target/dev`; a
bare `cargo build` does not, so name the directory when you run cargo yourself:

```
CARGO_TARGET_DIR=target/dev cargo build --profile release
```

With no fleet running there is nothing to swap, and the prefix is still the right habit: the run
where it matters is the one where you forgot something was running.

**2. `scripts/check.sh full` is the gate.** It is the same command CI runs
(`.github/workflows/check.yml`), so a red run at your desk and a red run on the pull request are the
same failure with the same fix — AI provenance, placeholder markers, the secret scan, rustfmt, the
500-line file check, clippy `-D warnings`, cargo-deny, the third-party notices check, docs drift,
the workspace tests and the dashboard's `tsc`. `scripts/check.sh commit` is the fast subset the
pre-commit hook runs while you iterate. Run the full one before you push, not after.

**3. Every artifact names the system that produced it.** Commits carry a trailer in the footer;
issues and pull request bodies carry the same line in the body.

```
Generated-by: <tool>/<model>
```

`Generated-by: claude-code/claude-opus-5` is the shape, and the tool alone is acceptable when the
model is not known. `scripts/provenance.py check` is what enforces it and `scripts/check.sh` runs
that, so a commit or a pull request without the line does not merge. The project settings in
`.claude/settings.json` run the same check the moment a `git commit` lands, which is a better place
to find out than CI.

What the gate enforces is *declaration*, not authorship. A trailer is self-reported, and a person
can add one to hand-written code. The convention is mandatory and visible; only review makes it
true, and a green check is not a claim about who wrote the code.

## Where the rest of it is

- `docs/README.md` — the map of every document, and which one answers which question.
- `AGENTS.md` — the engineering brief: the layout, the two hard invariants, the style the gate
  enforces, and how a change is reviewed.
- `docs/LESSONS.md` — why the rules are what they are, as mistakes this project already paid for.
  Most of `AGENTS.md` is in there with a better explanation.
- `docs/CONTRIBUTING.md` — the standard a change is held to, with every **MUST** marked.
- `docs/ARCHITECTURE.md` — processes, crates and the decision path; `docs/OPERATIONS.md` is the
  runbook.

## Workflow commands and subagents

`.claude/commands/` wraps the parts of the process that are easy to get wrong: `/gate`, `/golden`,
`/measure`, `/new-file`.

`.claude/agents/` has two subagents worth delegating to: a reviewer that knows the two hard
invariants and the crate layering, and a boundary checker for the layering rule on its own.

`.claude/skills/` holds the procedures that are not one command: `verify-install` (which commit is
the fleet actually playing, and does it match the checkout?) and `restore-store` (put runtime data
back from an archive, without ever writing over a live `artifacts/`).

## Agent skills

Matt Pocock's engineering skills are configured for this repo. Issues live as GitHub issues,
worked with the `gh` CLI as `svanlabs[bot]`.

### Issue tracker

GitHub issues via `gh`. See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical roles map 1:1 to same-named labels. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `docs/CONTEXT.md` plus `docs/adr/`, consumed lazily. See `docs/agents/domain.md`.

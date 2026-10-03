# 🗺️ Documentation map

Every document in this repository, the question it answers, and when to read it. You do not need
to read them all: find what you are about to do, and read that path in order.

## Reading paths

### 🎮 I want to run a fleet

1. [**Quick start**](../README.md#-quick-start) in the README — from a fresh machine to seated
   bots in five steps.
2. [`docs/OPERATIONS.md`](OPERATIONS.md) — the runbook: everyday commands, updates and rollbacks,
   backups, fault drills and the host checklist.
3. [`docs/GUIDE.md`](GUIDE.md), section 10 — the control room, panel by panel. The same guide is
   served on the dashboard's `/docs` page.

### 🔍 I want to understand how it plays

1. [**How a decision is made**](../README.md#-how-a-decision-is-made) in the README — the five-step
   picture.
2. [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) — the processes, the crates and the full decision path.
3. [`docs/GUIDE.md`](GUIDE.md), sections 3 to 6 — the decision, opponent modelling, the neural
   model and the learner, explained for a reader rather than an implementer.
4. [`docs/CONTEXT.md`](CONTEXT.md) — the vocabulary, when a word means something specific here.

### 🔥 Everything has gone to hell

1. [`docs/HELL.md`](HELL.md) — triage in five minutes: copy-paste commands, then agent prompts.
   Stop at the first section that explains what you see.
2. [`docs/OPERATIONS.md`](OPERATIONS.md) — treatment, once triage names the disease.

### 🛠️ I want to change something

1. [**Your first pull request**](CONTRIBUTING.md#your-first-pull-request) in `docs/CONTRIBUTING.md`
   — pick an issue, branch, run the gate, open the pull request.
2. [`AGENTS.md`](../AGENTS.md) — the brief your agent works from.
3. [`docs/LESSONS.md`](LESSONS.md) — why the rules are what they are. Read the entries for the area
   you are about to touch; most rejected changes repeat one of them.
4. The spec for that area, from the table below.

### 🤖 I am an AI agent

[`AGENTS.md`](../AGENTS.md) first. [`CLAUDE.md`](../CLAUDE.md) if you are Claude Code: it names
the three mistakes that are cheap to make and expensive to find. Then the issue you were given.

If a user pointed you at this repository and asked for a running fleet rather than a change —
[`llms.txt`](../llms.txt) is the short path: what to clone, what to run, and the six traps that
otherwise cost the first hour.

## Every document

| Document | Answers | Read it when |
|---|---|---|
| [`README.md`](../README.md) | What is this, and how do I start? | first |
| [`llms.txt`](../llms.txt) | How do I get a fleet running, in one page? | your agent was pointed here and asked for a running fleet |
| [`AGENTS.md`](../AGENTS.md) | What must an agent know before changing anything? | before any change |
| [`CLAUDE.md`](../CLAUDE.md) | Which three mistakes bite hardest here? | you are Claude Code |
| [`docs/CONTRIBUTING.md`](CONTRIBUTING.md) | How do I get a change merged, and what standard is it held to? | before your first pull request |
| [`docs/CONTEXT.md`](CONTEXT.md) | What does *flagship bot*, *fleet* or *season record* mean here? | a word seems to mean more than usual |
| [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) | Which processes run, how the crates layer, how one decision is made | you need the whole picture |
| [`docs/GUIDE.md`](GUIDE.md) | How do I use it, and what is the dashboard showing me? | you are operating it |
| [`docs/OPERATIONS.md`](OPERATIONS.md) | Which command does this job, and what do I do when something breaks? | you are running a fleet |
| [`docs/HELL.md`](HELL.md) | What do I paste first when everything is broken? | everything has gone to hell |
| [`docs/LESSONS.md`](LESSONS.md) | Why is this rule here? | before changing decisions, the learner, the client, data or operations |
| [`docs/SPEC-protocol.md`](SPEC-protocol.md) | What does the Open Poker WebSocket protocol look like, as implemented? | you touch the client or the tracker |
| [`docs/SPEC-data.md`](SPEC-data.md) | What is stored, in which format, and how is it protected? | you touch the store, a schema or a backup |
| [`docs/SPEC-learner.md`](SPEC-learner.md) | How does the learner search, and what must a change prove before it plays live? | you touch the learner or a promotion gate |
| [`docs/SPEC-dashboard.md`](SPEC-dashboard.md) | What does each dashboard endpoint and panel promise? | you touch the API or `web/` |
| [`docs/SPEC-scoring.md`](SPEC-scoring.md) | What is the season score, and which entries count? | you read or display a rank or leaderboard |
| [`docs/SPEC-payouts.md`](SPEC-payouts.md) | How do season prizes pay, and does prize value change play? | you discuss prizes or season-end play |
| [`docs/SPEC-competitions.md`](SPEC-competitions.md) | What are private competitions, and does SvanBot play them? | you consider private-competition support |
| [`docs/SPEC-pro.md`](SPEC-pro.md) | What do Pro and Portfolio endpoints offer, and does SvanBot call them? | you touch tiers, keys or the fleet cap |
| [`docs/RELEASE.md`](RELEASE.md) | How is a version cut and verified? | you are tagging a release |
| [`docs/AI-PROVENANCE.md`](AI-PROVENANCE.md) | Which AI systems are on record for this repository? | you want the provenance rule's evidence, not its statement |
| [`docs/SUPPORT.md`](SUPPORT.md) | Where does my question go? | you are stuck |
| [`docs/SECURITY.md`](SECURITY.md) | How do I report a vulnerability privately? | you found one — never in a public issue |

## Where each area lives

| Area | Code | Spec |
|---|---|---|
| The decision | `crates/libs/policy`, `crates/libs/model`, `crates/libs/engine` | [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) |
| The protocol | `crates/apps/bot/src/client`, `crates/libs/venue` | [`docs/SPEC-protocol.md`](SPEC-protocol.md) |
| The store | `crates/libs/store` | [`docs/SPEC-data.md`](SPEC-data.md) |
| The learner | `crates/apps/bot/src/bin/learner.rs`, `crates/apps/bot/src/learner` | [`docs/SPEC-learner.md`](SPEC-learner.md) |
| The dashboard | `web/src` (contract: `web/src/types.ts`), `crates/apps/bot/src/api` | [`docs/SPEC-dashboard.md`](SPEC-dashboard.md) |
| The gate | `scripts/check.sh`, `.github/workflows/check.yml` | [`docs/CONTRIBUTING.md`](CONTRIBUTING.md) section 11 |

## Keeping this map true

Every backticked repository path in the live documents is checked by `scripts/docs-check.py`, and
this file is one of them (`scripts/docs-check.live`). When a document is added, moved or renamed,
this map is part of the change.

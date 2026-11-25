# The documentation

What is decided, why, and how the agents that work here are set up. The project's words are in
[`GLOSSARY.md`](../GLOSSARY.md), and the brief for anyone working here is [`AGENTS.md`](../AGENTS.md).

## Decisions: `adr/`

An architecture decision record says what was decided, the forces behind it, and what it costs.
A decision that changes gets a new record, not an edit.

| ADR | Decision |
|---|---|
| [0001](adr/0001-decision-path-stays-in-process.md) | The decision path stays in-process |
| [0002](adr/0002-v11-from-scratch.md) | v11 is written from scratch on its own history |
| [0003](adr/0003-crate-graph.md) | Six layers, one crate per layer, checked by machine |
| [0004](adr/0004-reads-are-per-bot.md) | Reads are per bot, and so is the store |
| [0005](adr/0005-a-bots-policy-is-its-own.md) | A bot's policy is its own |

## Research: `research/2026-11/`

The reading behind the plan, each note with its sources. [The index](research/2026-11/README.md)
says what each one settled.

| Note | Subject |
|---|---|
| [00](research/2026-11/00-fairplay-audit.md) | The fair-play audit of v10 |
| [01](research/2026-11/01-decision-system.md) | The decision system |
| [02](research/2026-11/02-range-calibration.md) | Range calibration |
| [03](research/2026-11/03-opponent-response.md) | Per-opponent response |
| [04](research/2026-11/04-learner-experiments.md) | The learner's experiments |
| [05](research/2026-11/05-sim-vector.md) | The sim and the vectorised kernel |
| [06](research/2026-11/06-build-ci-tooling.md) | Build, CI and tooling |
| [07](research/2026-11/07-dashboard-agentic-docs.md) | The dashboard and the agentic docs |
| [08](research/2026-11/08-headline-measurement.md) | How the headline is measured |
| [09](research/2026-11/09-what-a-gate-can-trust.md) | What a gate can trust |

## Designs and plans: `superpowers/`

The design of a program of work and the plans that carry it out, each dated, for work with a map on
GitHub.

| Document | Subject |
|---|---|
| [Five brains](superpowers/specs/2026-11-25-five-brains-design.md) | The design of the map V11-59: a policy of its own for each bot |
| [Phase 1, the tracer](superpowers/plans/2026-11-25-per-bot-policy-phase-1-tracer.md) | The plan for its first slice |
| [Phase 0, measurement](superpowers/plans/2026-11-25-per-bot-policy-phase-0-measurement.md) | The plan for sharper measurement |

## The agents' configuration: `agents/`

What the pinned skills read: [the issue tracker](agents/issue-tracker.md) (GitHub, worked as
`svanlabs[bot]`), [the triage labels](agents/triage-labels.md) and [the domain docs](agents/domain.md).

## Elsewhere

- The box and its units: [`deploy/README.md`](../deploy/README.md).
- What changed, release by release: [`CHANGELOG.md`](../CHANGELOG.md).
- The venue's own documentation, which comes first: <https://docs.openpoker.ai/llms-full.txt>.

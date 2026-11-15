# Triage labels

The skills speak in terms of five canonical triage roles. This file maps those roles to the actual
label strings used in this repo's issue tracker.

## The five canonical roles

| Label in mattpocock/skills | Label in our tracker | Meaning                              |
| -------------------------- | -------------------- | ------------------------------------ |
| `needs-triage`             | `needs-triage`       | Not yet assessed or routed           |
| `needs-info`               | `needs-info`         | Waiting on reporter for more detail  |
| `ready-for-agent`          | `agent-friendly`     | Fully specified, ready for an agent  |
| `ready-for-human`          | *(none — see below)* | Requires human implementation        |
| `wontfix`                  | `wontfix`            | Will not be actioned                 |

Two of these do not map one-to-one, and a skill must not assume they do:

- **`ready-for-agent` is `agent-friendly` here.** The skill's role and this repo's label are the
  same concept under different names (see `AGENTS.md`, *Which issue to take*). Apply
  `agent-friendly`; do not create a `ready-for-agent` label.
- **`ready-for-human` has no equivalent.** This repo does not route issues to a human to *implement*.
  What it has instead is `blocked-on-decision` — the diagnosis is done, but a maintainer or the
  operator has to choose before anyone can build it. That is a different thing: it means *stop and
  ask*, not *a person will write this*. A skill reaching for `ready-for-human` should stop and use
  `blocked-on-decision` if waiting on a decision is the real state, and should say so rather than
  inventing a label.

`needs-triage` and `needs-info` keep their canonical names and meanings, and both labels already
exist in the tracker. `wontfix` exists too, though few issues have needed it.

All five canonical strings, plus `ready-for-agent` and `ready-for-human`, exist as labels on this
repository. The two with no usage are the two mapped above: `ready-for-agent`, because
`agent-friendly` is the label in play, and `ready-for-human`, because this repo has no such state.

## Labels outside this vocabulary

These carry meaning in this repo's tracker but are **not** triage roles. The skills do not read or
write them; they are recorded here so a skill does not mistake one for a triage state.

| Label                 | Meaning                                                                 |
| --------------------- | ----------------------------------------------------------------------- |
| `blocked-on-decision` | Diagnosis done; a decision is needed before anyone can start             |
| `agent-claimed`       | A session has taken this issue; a second agent should not race for it    |
| `decisions`           | The issue is a record of decisions taken                                 |

Readiness is also mirrored as a **Readiness** field on the project board at
<https://github.com/orgs/SvanLabs/projects/1>.

## When the tracker changes

If a label here is renamed in the tracker, edit the middle column of the first table and this file
stays the single place that records the mapping. Do not rename canonical roles to match — the skills
speak in role names, and this file is what translates them.

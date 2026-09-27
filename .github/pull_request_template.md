## What generated this?

Generated-by: <tool>/<model>

<!-- Replace the placeholder with your own values: the tool, a slash, the model — `claude-code/claude-opus-5`
     and `aider/gpt-5` are the shape, and the tool alone is acceptable when the model is not known.
     Keep the line: `scripts/provenance.py check` reads the commits, and
     `scripts/provenance.py check --pr-body <file>` reads this description. The rule in full is
     `AGENTS.md` section 0, and it is the one convention here that a reviewer refuses a change for
     rather than fixes. -->

## Motivation

<!-- The problem, and how you know it is one: the failing test, the measurement, the log line, the
     issue this closes. A change with no motivation is a change nobody can review. -->

## Solution

<!-- What changed, in which layer it belongs (`crates/deps`, `crates/libs`, `crates/apps`, `web`,
     `docs`, `scripts`), and why there rather than somewhere else. Note anything a reviewer should be
     suspicious of, including a test that fails for a reason you do not understand — say so rather
     than adjusting the test until it goes green. -->

## Checklist

- [ ] `scripts/check.sh full` passes on this exact commit, and its output is in this description.
- [ ] This is one change. A bug fix and a rename are two pull requests.
- [ ] A behaviour change carries a paired simulation on identical deals (`sim paired`), with its
      numbers above; the golden snapshot (`crates/apps/core/tests/golden/core.json`) changed only
      where I meant it to, and each changed line is explained.
- [ ] A performance change carries a measurement taken on the same machine against the previous
      commit. A speed-only change reproduces the guard run exactly.
- [ ] Documents that name a path, command or rule I changed are updated in this pull request
      (`scripts/docs-check.py` fails the gate otherwise).
- [ ] The AI-provenance rule was followed: `Generated-by:` is in every commit footer and in the
      section above.

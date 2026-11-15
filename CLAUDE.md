@AGENTS.md

## Claude Code

- The Matt Pocock skills are pinned in `.claude/settings.json`, so use them:
  - `wayfinder` works the V11-00 map
  - `tdd` is the build loop
  - `code-review` and `pr` close a change
  - `domain-modeling` keeps `GLOSSARY.md` and `docs/adr/` current
- Run the work inline in this session, with no subagents. The operator set this rule.
- Before calling a skill, check `docs/agents/`: it holds the configuration the skills read (the
  tracker, the labels, the domain docs).

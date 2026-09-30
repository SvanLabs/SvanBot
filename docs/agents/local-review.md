# Local review with any coding agent

Run review on the user's computer from a checkout with committed changes:

```sh
python3 scripts/local-review.py origin/main
```

This pins the base and head, writes the commit history and diff to `artifacts/review/input.md`,
and runs the full gate. Give that file to your preferred coding agent. An agent CLI that reads
its prompt from stdin can also run directly:

```sh
python3 scripts/local-review.py origin/main -- your-agent-command its-arguments
```

The command is passed as arguments, without shell evaluation. It runs only after the gate passes,
and writes its stdout to `artifacts/review/result.md`; its failure status is preserved. The selected
agent may use its own subscription or API and network settings. The review runner makes no model
calls itself, installs nothing, and publishes nothing. Review inline to control cost.

Read the originating issue and repository standards, then report Standards and Spec findings
separately with evidence. Missing issue access is a limitation to record. Report hypotheses as such.
Use the repository's provenance convention in review artifacts.

GitHub runs the ordinary deterministic check workflow. Hosted Claude review, maintenance, triage
and mention workflows have been removed. Provider-specific local configuration remains optional;
AGENTS.md and this procedure are the shared entry points.

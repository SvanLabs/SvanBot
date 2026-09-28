"""Workflow shape: a job that runs the gate installs what the gate needs.

#418: `claude-maintainer.yml` ran `scripts/check.sh full` on a fresh runner with no cargo-deny,
no `web/node_modules` and no board-strength tables, so the gate died at `error: no such command:
deny` — a red run whose cause had nothing to do with the issue the run was working. The step that
installs them is `scripts/cloud-setup.sh`, and this pins the pairing so the next job that runs the
gate cannot quietly omit it.
"""
import unittest
from pathlib import Path

WORKFLOWS = Path(__file__).resolve().parents[2] / ".github" / "workflows"
SETUP = "scripts/cloud-setup.sh"
GATE = "scripts/check.sh"


class GateJobsInstallTheirTools(unittest.TestCase):
    def test_a_workflow_that_runs_the_gate_runs_cloud_setup(self):
        for path in sorted(WORKFLOWS.glob("*.yml")):
            text = path.read_text()
            if GATE not in text:
                continue
            with self.subTest(workflow=path.name):
                self.assertIn(
                    SETUP,
                    text,
                    f"{path.name} runs {GATE} without running {SETUP}: a fresh runner has no "
                    "cargo-deny, no web/node_modules and no artifacts/tables, so the gate fails "
                    "before it reaches the change (#418)",
                )

    def test_cloud_setup_is_run_and_not_only_named(self):
        # A path in prose is not an install. The gate only gets its tools from a step that runs it.
        for path in sorted(WORKFLOWS.glob("*.yml")):
            text = path.read_text()
            if SETUP not in text:
                continue
            with self.subTest(workflow=path.name):
                self.assertIn(
                    f"run: bash {SETUP}",
                    text,
                    f"{path.name} names {SETUP} but never runs it",
                )


if __name__ == "__main__":
    unittest.main()

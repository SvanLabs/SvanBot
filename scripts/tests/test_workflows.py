"""Workflow shape: a job that runs the gate installs what the gate needs.

#418: `claude-maintainer.yml` ran `scripts/check.sh full` on a fresh runner with no cargo-deny,
no `web/node_modules` and no board-strength tables, so the gate died at `error: no such command:
deny` — a red run whose cause had nothing to do with the issue the run was working. The step that
installs them is `scripts/cloud-setup.sh`, and this pins the pairing so the next job that runs the
gate cannot quietly omit it.

Each property is carried by the text of a *step that runs something*, not by a path mentioned
somewhere in the file, and a test that finds its step nowhere has checked nothing: #584's
reproduction renamed the gate entrypoint and left the suite green with no assertion run, and the
same hole stays open one level up for a marker that survives in prose while the step is deleted.
A missing step is therefore a failure here, not a skip, and the count each case returns is what
says it looked at something.
"""
import unittest
from pathlib import Path

WORKFLOWS = Path(__file__).resolve().parents[2] / ".github" / "workflows"
SETUP_RUN = "run: bash scripts/cloud-setup.sh"
GATE_RUN = "bash scripts/check.sh full"


def workflow_files() -> list[Path]:
    return sorted(WORKFLOWS.glob("*.yml"))


class GateJobsInstallTheirTools(unittest.TestCase):
    def test_a_workflow_that_runs_the_gate_runs_cloud_setup(self):
        checked = 0
        for path in workflow_files():
            text = path.read_text()
            if GATE_RUN not in text:
                continue
            checked += 1
            with self.subTest(workflow=path.name):
                self.assertIn(
                    SETUP_RUN,
                    text,
                    f"{path.name} runs the gate without running scripts/cloud-setup.sh: a fresh "
                    "runner has no cargo-deny, no web/node_modules and no artifacts/tables, so the "
                    "gate fails before it reaches the change (#418)",
                )
        self.assertGreater(
            checked,
            0,
            f"no workflow runs `{GATE_RUN}`: the gate has been moved out of the workflow text or "
            "renamed, or the workflows are not where this reads them — update this test with the "
            "new shape rather than deleting the property",
        )

    def test_cloud_setup_is_run_and_not_only_named(self):
        # The same hole on the other side: a workflow that mentions the setup script in prose
        # installs nothing, and the gate then fails on the runner it was supposed to fix.
        checked = 0
        for path in workflow_files():
            text = path.read_text()
            if "scripts/cloud-setup.sh" not in text:
                continue
            checked += 1
            with self.subTest(workflow=path.name):
                self.assertIn(
                    SETUP_RUN,
                    text,
                    f"{path.name} names scripts/cloud-setup.sh but never runs it",
                )
        self.assertGreater(checked, 0, "no workflow installs the gate's tools with scripts/cloud-setup.sh")


if __name__ == "__main__":
    unittest.main()

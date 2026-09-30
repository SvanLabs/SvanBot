"""A local review pins its inputs, runs the gate, and invokes only an explicit agent."""
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class LocalReviewTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        (self.repo / "scripts").mkdir()
        shutil.copy(ROOT / "scripts/local-review.py", self.repo / "scripts/local-review.py")
        self.git("init", "-q")
        self.git("config", "user.name", "test")
        self.git("config", "user.email", "test@example.invalid")
        gate = self.repo / "scripts/check.sh"
        gate.write_text('#!/bin/sh\n[ "$1" = full ] || exit 5\nexit "${GATE_STATUS:-0}"\n')
        self.git("add", ".")
        self.git("commit", "-qm", "base")
        self.base = self.git("rev-parse", "HEAD").strip()
        (self.repo / "change.txt").write_text("review this\n")
        self.git("add", ".")
        self.git("commit", "-qm", "change")

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.repo, text=True)

    def run_review(self, *args, env=None):
        return subprocess.run([sys.executable, "scripts/local-review.py", self.base, *args],
                              cwd=self.repo, text=True, capture_output=True, env=env)

    def test_packet_records_exact_commits_and_diff_without_an_agent(self):
        result = self.run_review()
        self.assertEqual(result.returncode, 0, result.stderr)
        packet = (self.repo / "artifacts/review/input.md").read_text()
        self.assertIn(self.base, packet)
        self.assertIn(self.git("rev-parse", "HEAD").strip(), packet)
        self.assertIn("+review this", packet)
        self.assertIn("Standards", packet)
        self.assertIn("Spec", packet)

    def test_gate_failure_prevents_agent_execution(self):
        import os
        agent = [sys.executable, "-c", "from pathlib import Path; Path('called').touch()"]
        result = self.run_review("--", *agent, env=dict(os.environ, GATE_STATUS="7"))
        self.assertEqual(result.returncode, 7, result.stderr)
        self.assertFalse((self.repo / "called").exists())

    def test_explicit_agent_receives_packet_and_failure_is_propagated(self):
        agent = [sys.executable, "-c", "import sys; print(sys.stdin.read()); sys.exit(9)"]
        result = self.run_review("--", *agent)
        self.assertEqual(result.returncode, 9, result.stderr)
        self.assertIn("+review this", (self.repo / "artifacts/review/result.md").read_text())

    def test_dirty_tracked_sources_are_rejected(self):
        (self.repo / "change.txt").write_text("uncommitted\n")
        result = self.run_review()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Commit tracked changes", result.stderr)

    def test_hosted_model_review_is_removed(self):
        for path in (ROOT / ".github/workflows").glob("*.y*ml"):
            text = path.read_text().lower()
            self.assertNotIn("anthropics/claude-code-action", text, path.name)
            self.assertNotIn("claude_code_oauth_token", text, path.name)
            self.assertFalse(path.name.startswith("claude"), path.name)


if __name__ == "__main__":
    unittest.main()

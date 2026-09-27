"""scripts/provenance.py (0367): every commit and pull request names its generating system.

The tests build throwaway repositories: the real one predates the rule, so it is the wrong fixture
for asking whether the check works.
"""
from __future__ import annotations

import importlib.util
import io
import os
import subprocess
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("provenance", HERE / "provenance.py")
pv = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pv)

GIT_ENV = {
    "GIT_AUTHOR_NAME": "Tester",
    "GIT_AUTHOR_EMAIL": "tester@example.invalid",
    "GIT_COMMITTER_NAME": "Tester",
    "GIT_COMMITTER_EMAIL": "tester@example.invalid",
    "GIT_CONFIG_GLOBAL": "/dev/null",
    "GIT_CONFIG_SYSTEM": "/dev/null",
}


def git(root: Path, *args: str, author: str | None = None) -> str:
    """Run git in a throwaway repository."""
    env = {**os.environ, **GIT_ENV}
    if author:
        env["GIT_AUTHOR_NAME"] = env["GIT_COMMITTER_NAME"] = author
    done = subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True, env=env, check=True)
    return done.stdout


class Provenance(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = Path(self._tmp.name)
        git(self.root, "init", "-q", "-b", "main")
        self.n = 0
        self._old_root = pv.ROOT
        pv.ROOT = self.root

    def tearDown(self):
        pv.ROOT = self._old_root
        self._tmp.cleanup()

    def commit(self, message: str, author: str | None = None) -> None:
        # A file per commit: two branches touching one path would make the merge test conflict.
        self.n += 1
        (self.root / f"f{self.n}.txt").write_text(f"{message}\n")
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", message, author=author)

    def run_check(self, *args: str) -> tuple[int, str]:
        """The exit code and everything written, however it was streamed."""
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            code = pv.main(["check", *args])
        return code, out.getvalue() + err.getvalue()

    def test_a_commit_naming_no_system_is_reported(self):
        self.commit("fix: clamp the equity table index")
        code, err = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("no Generated-by trailer", err)

    def test_a_commit_naming_its_system_passes(self):
        self.commit("fix: clamp the equity table index\n\nGenerated-by: claude-code/opus-5\n")
        code, err = self.run_check()
        self.assertEqual(code, 0, err)
        self.assertIn("1 commit(s)", err)

    def test_a_range_reports_only_the_commits_in_it(self):
        self.commit("first: no trailer")
        self.commit("second\n\nGenerated-by: claude-code/opus-5\n")
        self.commit("third: no trailer")
        code, out = self.run_check("HEAD~2..HEAD~1")
        self.assertEqual(code, 0, out)
        code, out = self.run_check("HEAD~1..HEAD")
        self.assertEqual(code, 1)
        self.assertIn("third: no trailer", out)
        self.assertNotIn("second\n", out)

    def test_a_bot_authored_commit_is_exempt(self):
        # dependabot[bot] names its generator in the author field; a trailer would repeat it.
        self.commit("chore(deps): bump serde", author="dependabot[bot]")
        code, err = self.run_check()
        self.assertEqual(code, 0, err)
        self.assertIn("0 commit(s)", err)

    def test_a_merge_commit_is_skipped(self):
        self.commit("first\n\nGenerated-by: claude-code/opus-5\n")
        git(self.root, "checkout", "-q", "-b", "side")
        self.commit("side\n\nGenerated-by: claude-code/opus-5\n")
        git(self.root, "checkout", "-q", "main")
        self.commit("main\n\nGenerated-by: claude-code/opus-5\n")
        git(self.root, "merge", "-q", "--no-ff", "-m", "merge side without a trailer", "side")
        code, err = self.run_check()
        self.assertEqual(code, 0, err)

    def test_an_unreadable_range_is_reported_not_ignored(self):
        self.commit("only\n\nGenerated-by: claude-code/opus-5\n")
        code, err = self.run_check("no-such-ref..HEAD")
        self.assertEqual(code, 1)
        self.assertIn("cannot read commits", err)

    def test_a_pull_request_body_must_name_its_system(self):
        self.commit("fix\n\nGenerated-by: claude-code/opus-5\n")
        body = self.root / "body.md"
        body.write_text("## Motivation\nNone.\n")
        code, err = self.run_check("--pr-body", str(body))
        self.assertEqual(code, 1)
        self.assertIn("pull request body", err)
        body.write_text("## Motivation\nNone.\n\nGenerated-by: claude-code/opus-5\n")
        code, err = self.run_check("--pr-body", str(body))
        self.assertEqual(code, 0, err)

    def test_a_missing_pull_request_body_is_reported(self):
        self.commit("fix\n\nGenerated-by: claude-code/opus-5\n")
        code, err = self.run_check("--pr-body", str(self.root / "absent.md"))
        self.assertEqual(code, 1)
        self.assertIn("not found", err)

    def quiet_stamp(self) -> tuple[int, str]:
        out = io.StringIO()
        with redirect_stdout(out), redirect_stderr(io.StringIO()):
            return pv.stamp(), out.getvalue()

    def test_stamp_names_the_system_or_refuses(self):
        old = os.environ.pop("SVANBOT_GENERATED_BY", None)
        try:
            code, out = self.quiet_stamp()
            self.assertEqual((code, out), (1, ""))
            os.environ["SVANBOT_GENERATED_BY"] = "claude-code/opus-5"
            self.assertEqual(self.quiet_stamp(), (0, "Generated-by: claude-code/opus-5\n"))
            os.environ["SVANBOT_GENERATED_BY"] = "not a system!"
            self.assertEqual(self.quiet_stamp()[0], 1)
        finally:
            os.environ.pop("SVANBOT_GENERATED_BY", None)
            if old is not None:
                os.environ["SVANBOT_GENERATED_BY"] = old

    def test_usage_without_a_command(self):
        for argv in ([], ["nonsense"]):
            with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                self.assertEqual(pv.main(argv), 2)


if __name__ == "__main__":
    unittest.main()

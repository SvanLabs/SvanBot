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
        code, err = self.run_check("--pr-body", str(body), "--pr-author", "someone")
        self.assertEqual(code, 1)
        self.assertIn("pull request body", err)
        body.write_text("## Motivation\nNone.\n\nGenerated-by: claude-code/opus-5\n")
        code, err = self.run_check("--pr-body", str(body), "--pr-author", "someone")
        self.assertEqual(code, 0, err)

    def test_a_bot_opened_pull_request_is_exempt(self):
        # The same exemption `test_a_bot_authored_commit_is_exempt` covers: Dependabot's pull request
        # is Dependabot's, and the login that opened it names the system (#367).
        self.commit("chore(deps): bump serde", author="dependabot[bot]")
        body = self.root / "body.md"
        body.write_text("Bumps the actions group with 6 updates.\n")
        code, err = self.run_check("--pr-body", str(body), "--pr-author", "dependabot[bot]")
        self.assertEqual(code, 0, err)

    def test_an_empty_author_exempts_nothing(self):
        # An unset login must not be read as a bot: no GitHub login is empty, so an empty one only
        # ever means the login did not reach the gate, and exempting it would exempt everyone.
        self.commit("fix\n\nGenerated-by: claude-code/opus-5\n")
        body = self.root / "body.md"
        body.write_text("## Motivation\nNone.\n")
        code, err = self.run_check("--pr-body", str(body), "--pr-author", "")
        self.assertEqual(code, 1)
        self.assertIn("pull request body", err)

    def test_a_missing_pull_request_body_is_reported(self):
        # Checked before the exemption: a body that never arrived means the gate checked nothing,
        # whoever opened the pull request.
        self.commit("chore(deps): bump serde", author="dependabot[bot]")
        code, err = self.run_check("--pr-body", str(self.root / "absent.md"), "--pr-author", "dependabot[bot]")
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

    def run_report(self, *args: str) -> tuple[int, str]:
        """The exit code and everything written, however it was streamed."""
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            code = pv.main(["report", *args])
        return code, out.getvalue() + err.getvalue()

    def roster(self) -> str:
        return (self.root / "AI-PROVENANCE.md").read_text()

    def test_report_lists_the_systems_on_record(self):
        self.commit("one\n\nGenerated-by: claude-code/opus-5\n")
        self.commit("two\n\nGenerated-by: claude-code/opus-5\n")
        self.commit("three\n\nGenerated-by: claude-code/sonnet-5\n")
        self.commit("four: names no system, which is not this command's business")
        code, out = self.run_report()
        self.assertEqual(code, 0, out)
        systems = pv.roster_systems(self.roster())
        self.assertEqual(systems, {"claude-code/opus-5", "claude-code/sonnet-5"})
        self.assertIn("| claude-code/opus-5 | 2 |", self.roster())
        self.assertIn("| claude-code/sonnet-5 | 1 |", self.roster())
        # The document a run just wrote is the document the check wants.
        self.assertEqual(self.run_report("--check"), (0, "provenance: AI-PROVENANCE.md lists the 2 system(s) in HEAD\n"))

    def test_a_new_system_is_a_red_check(self):
        self.commit("one\n\nGenerated-by: claude-code/opus-5\n")
        self.run_report()
        self.commit("two\n\nGenerated-by: claude-code/gpt-9\n")
        code, out = self.run_report("--check")
        self.assertEqual(code, 1)
        self.assertIn("does not list claude-code/gpt-9", out)

    def test_a_moved_count_does_not_fail_the_check(self):
        # The property the whole design rests on: the check compares the set of systems, not the counts,
        # so ordinary work on the repository does not have to regenerate the document — and does not
        # conflict in every branch over it.
        self.commit("one\n\nGenerated-by: claude-code/opus-5\n")
        self.run_report()
        before = self.roster()
        self.commit("two\n\nGenerated-by: claude-code/opus-5\n")
        code, out = self.run_report("--check")
        self.assertEqual(code, 0, out)
        self.assertEqual(self.roster(), before, "the check rewrote the document")
        # And regenerating by hand picks the count up.
        self.run_report()
        self.assertIn("| claude-code/opus-5 | 2 |", self.roster())

    def test_a_documented_system_the_history_does_not_name_is_reported(self):
        self.commit("one\n\nGenerated-by: claude-code/opus-5\n")
        self.run_report()
        (self.root / "AI-PROVENANCE.md").write_text(
            self.roster().replace("| claude-code/opus-5 | 1 |", "| claude-code/opus-5 | 1 |\n| claude-code/opus-9 | 3 |"))
        code, out = self.run_report("--check")
        self.assertEqual(code, 1)
        self.assertIn("lists claude-code/opus-9", out)

    def test_a_missing_roster_is_a_red_check_not_a_crash(self):
        self.commit("one\n\nGenerated-by: claude-code/opus-5\n")
        code, out = self.run_report("--check")
        self.assertEqual(code, 1)
        self.assertIn("does not list claude-code/opus-5", out)

    def test_usage_without_a_command(self):
        for argv in ([], ["nonsense"]):
            with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                self.assertEqual(pv.main(argv), 2)


if __name__ == "__main__":
    unittest.main()

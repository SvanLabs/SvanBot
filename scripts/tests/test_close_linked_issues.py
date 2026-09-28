"""scripts/close-linked-issues.py (#410): a merged description closes the issues it names.

The keywords and the references are GitHub's, so the tests are the forms the documentation lists plus
the ones that must *not* match — a word that merely contains a keyword, a number in another
repository, a template line left with no number on it.
"""
from __future__ import annotations

import importlib.util
import io
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("close_linked_issues", HERE / "close-linked-issues.py")
cl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cl)


class References(unittest.TestCase):
    def test_every_keyword_github_documents(self):
        keywords = ("close", "closes", "closed", "fix", "fixes", "fixed", "resolve", "resolves", "resolved")
        for keyword in keywords:
            self.assertEqual(cl.references(f"{keyword} #7"), [7], keyword)

    def test_case_is_not_significant(self):
        self.assertEqual(cl.references("Closes #7"), [7])
        self.assertEqual(cl.references("CLOSES #7"), [7])
        self.assertEqual(cl.references("Closes #7".upper()), [7])

    def test_a_colon_after_the_keyword(self):
        self.assertEqual(cl.references("Closes: #7"), [7])

    def test_each_number_once_in_the_order_it_appears(self):
        self.assertEqual(cl.references("fixes #3, closes #1, and closes #3 again"), [3, 1])

    def test_several_per_line_and_across_lines(self):
        self.assertEqual(cl.references("closes #1 fixes #2\n\nresolves #3\n"), [1, 2, 3])

    def test_punctuation_after_the_number(self):
        self.assertEqual(cl.references("Closes #370."), [370])

    def test_a_qualified_reference_to_this_repository(self):
        self.assertEqual(cl.references("closes SvanLabs/SvanBot#12", "SvanLabs/SvanBot"), [12])

    def test_another_repository_is_left_alone(self):
        self.assertEqual(cl.references("closes other/repo#12", "SvanLabs/SvanBot"), [])
        self.assertEqual(cl.references("closes #12", "SvanLabs/SvanBot"), [12])

    def test_a_word_that_merely_contains_a_keyword_is_not_one(self):
        self.assertEqual(cl.references("recloses #7"), [])
        self.assertEqual(cl.references("prefix #8"), [])
        self.assertEqual(cl.references("closes#7"), [])

    def test_a_keyword_with_no_reference(self):
        self.assertEqual(cl.references("Closes #"), [])
        self.assertEqual(cl.references("closes"), [])

    def test_the_pull_request_template_left_alone(self):
        template = (
            "## Motivation\n\nCloses #\n\n"
            "<!-- Put the issue number after `Closes #` so the issue closes when this merges; delete the\n"
            "     line if there is no issue. -->\n"
        )
        self.assertEqual(cl.references(template), [])


class Check(unittest.TestCase):
    """`--check` prints what it would close and reaches no network, so it is the one path a test runs."""

    def run_check(self, body: str) -> tuple[int, str]:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "pr-body.md"
            path.write_text(body)
            out = io.StringIO()
            with redirect_stdout(out):
                code = cl.main(["--repo", "SvanLabs/SvanBot", "--body-file", str(path), "--check"])
        return code, out.getvalue()

    def test_it_prints_each_number_and_changes_nothing(self):
        code, out = self.run_check("Closes #408\n\nAlso fixes #410\n")
        self.assertEqual(code, 0)
        self.assertIn("would close #408", out)
        self.assertIn("would close #410", out)

    def test_a_body_that_names_nothing_says_so(self):
        code, out = self.run_check("No issue reference here.\n")
        self.assertEqual(code, 0)
        self.assertIn("names no issue", out)


class Usage(unittest.TestCase):
    """The usage message goes to stderr, and the gate prints a suite's output only when it fails."""

    def usage(self, argv: list[str]) -> int:
        with redirect_stderr(io.StringIO()):
            return cl.main(argv)

    def test_a_missing_argument_is_a_usage_error(self):
        self.assertEqual(self.usage(["--check"]), 2)

    def test_an_unknown_argument_is_a_usage_error(self):
        self.assertEqual(self.usage(["--nonsense"]), 2)


if __name__ == "__main__":
    unittest.main()

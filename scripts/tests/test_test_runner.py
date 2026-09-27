"""scripts/test.py's failure report (#140): a binary that ends without printing still says how.

The block the runner prints for a failing binary is the gate's whole diagnosis — the reason a red
run is a fixable failure and not a rerun — so how it words a binary that died in silence is worth
pinning. The values are the ones `os.waitstatus_to_exitcode` returns: negative for a signal.
"""
from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("sv10_test_runner", HERE / "test.py")
tr = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tr)


class ExitNote(unittest.TestCase):
    def test_a_signal_death_is_named(self):
        self.assertEqual(tr.exit_note(-9), "killed by SIGKILL")
        self.assertEqual(tr.exit_note(-11), "killed by SIGSEGV")
        self.assertEqual(tr.exit_note(-6), "killed by SIGABRT")

    def test_an_ordinary_exit_reports_its_status(self):
        self.assertEqual(tr.exit_note(1), "exit status 1")
        self.assertEqual(tr.exit_note(101), "exit status 101")

    def test_a_signal_with_no_name_still_reports_its_number(self):
        # `signal.Signals` knows the names the kernel has; a number outside them is still a fact.
        self.assertEqual(tr.exit_note(-12345), "killed by signal 12345")


class FailureReport(unittest.TestCase):
    def test_the_status_and_the_output_are_both_in_the_block(self):
        block = tr.failure_report("sv10_bot", "test result: FAILED. 3 passed; 1 failed\n", -6)
        self.assertIn("---- sv10_bot (killed by SIGABRT) ----", block)
        self.assertIn("1 failed", block)

    def test_a_binary_that_printed_nothing_says_so(self):
        block = tr.failure_report("sv10_bot", "", 1)
        self.assertIn("---- sv10_bot (exit status 1) ----", block)
        self.assertIn("no output", block)

    def test_whitespace_is_not_output(self):
        block = tr.failure_report("sv10_bot", "\n\n", 1)
        self.assertIn("no output", block)

    def test_the_output_is_not_trimmed_of_its_last_line(self):
        block = tr.failure_report("sv10_bot", "a\nb", 1)
        self.assertTrue(block.rstrip().endswith("b"), block)


if __name__ == "__main__":
    unittest.main()

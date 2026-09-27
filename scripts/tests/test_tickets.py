"""Unit tests for scripts/tickets.py on a throwaway tracker (run: python3 -m unittest scripts/tests/test_tickets.py)."""
import contextlib
import io
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))


def ticket(status="open", blocks="[]", blocked_by="[]", assignee="", ty="sv10:task", extra=""):
    return f"---\ntype: {ty}\nstatus: {status}\nassignee: {assignee}\nblocks: {blocks}\nblocked_by: {blocked_by}\n{extra}---\n\n## Question\n\nq\n"


class TicketsTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        root = Path(self.dir.name)
        (root / "issues").mkdir()
        (root / "MAP.md").write_text("## Decisions so far\n\n- [0001-a](issues/0001-a.md): done\n\n## Not yet specified\n")
        self.root = root
        os.environ["SV10_TICKETS_ROOT"] = str(root)
        import importlib
        import tickets
        self.t = importlib.reload(tickets)

    def tearDown(self):
        self.dir.cleanup()
        os.environ.pop("SV10_TICKETS_ROOT")

    def write(self, name, text):
        (self.root / "issues" / name).write_text(text)

    def run_cli(self, *args):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = self.t.main(list(args))
        return code, out.getvalue()

    def test_frontier_skips_blocked_claimed_and_done_and_sorts_by_priority(self):
        self.write("0001-a.md", ticket(status="resolved", blocks="[0003]"))
        self.write("0002-b.md", ticket(blocks="[0004]"))
        self.write("0003-c.md", ticket(blocked_by="[0001]", extra="priority: P0\n"))
        self.write("0004-d.md", ticket(blocked_by="[0002]"))
        self.write("0005-e.md", ticket(assignee="agent"))
        _, out = self.run_cli("frontier")
        self.assertEqual([line[:4] for line in out.splitlines()], ["0003", "0002"])

    def test_lint_reports_then_fixes_one_sided_edges_and_bare_types(self):
        self.write("0001-a.md", ticket(status="resolved"))
        self.write("0002-b.md", ticket(blocked_by="[0001]", ty="task"))
        code, out = self.run_cli("lint")
        self.assertEqual(code, 1)
        self.assertIn("0001 lacks blocks 0002", out)
        self.assertIn("type 'task'", out)
        code, _ = self.run_cli("lint", "--fix")
        self.assertEqual(code, 0)
        self.assertIn("blocks: [0002]", (self.root / "issues/0001-a.md").read_text())

    def test_lint_accepts_a_known_kind_under_another_namespace(self):
        self.write("0001-a.md", ticket(status="resolved", ty="fleet:task"))
        code, out = self.run_cli("lint")
        self.assertEqual(code, 0, out)

    def test_lint_catches_cycles_missing_refs_and_broken_map_links(self):
        self.write("0001-a.md", ticket(status="resolved", blocks="[0002]", blocked_by="[0002]"))
        self.write("0002-b.md", ticket(blocks="[0001]", blocked_by="[0001, 0009]"))
        (self.root / "MAP.md").write_text("- [x](issues/0007-gone.md): y\n")
        _, out = self.run_cli("lint")
        self.assertIn("blocking cycle", out)
        self.assertIn("missing ticket 0009", out)
        self.assertIn("broken link issues/0007-gone.md", out)

    def test_new_wires_both_ends_and_close_indexes_the_map(self):
        (self.root / "MAP.md").write_text("## Decisions so far\n\n## Not yet specified\n")
        self.write("0001-a.md", ticket())
        self.run_cli("new", "Fix the thing!", "--priority", "P1", "--labels", "bug,live", "--blocked-by", "1")
        new = (self.root / "issues/0002-fix-the-thing.md").read_text()
        self.assertIn("priority: P1", new)
        self.assertIn("labels: [bug, live]", new)
        self.assertIn("blocked_by: [0001]", new)
        self.assertIn("blocks: [0002]", (self.root / "issues/0001-a.md").read_text())
        _, out = self.run_cli("close", "0001", "--gist", "shipped")
        self.assertIn("now unblocked: 0002", out)
        self.assertIn("status: resolved", (self.root / "issues/0001-a.md").read_text())
        m = (self.root / "MAP.md").read_text()
        self.assertLess(m.index("0001-a.md): shipped"), m.index("## Not yet specified"))

    def test_close_indexes_ticket_already_linked_in_notes_once(self):
        (self.root / "MAP.md").write_text(
            "## Notes\n\nSee [the ticket](issues/0001-a.md).\n\n"
            "## Decisions so far\n\n## Not yet specified\n"
        )
        self.write("0001-a.md", ticket())

        self.run_cli("close", "0001", "--gist", "shipped")
        self.run_cli("close", "0001", "--gist", "shipped")

        m = (self.root / "MAP.md").read_text()
        decisions = m.split("## Decisions so far\n", 1)[1].split("## Not yet specified", 1)[0]
        self.assertEqual(decisions.count("- [0001-a](issues/0001-a.md): shipped"), 1)

    def test_claim_refuses_someone_elses_ticket(self):
        self.write("0001-a.md", ticket(assignee="other"))
        with self.assertRaises(SystemExit):
            self.run_cli("claim", "0001")


if __name__ == "__main__":
    unittest.main()

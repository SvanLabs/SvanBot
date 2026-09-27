"""scripts/docs-check.py (0240): live docs name only paths and tool commands that exist."""
from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("docs_check", HERE / "docs-check.py")
dc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dc)


class DocsCheck(unittest.TestCase):
    def test_the_repository_docs_are_current(self):
        self.assertEqual(dc.main([]), 0)

    def test_missing_paths_and_unknown_commands_are_reported(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "crates/apps/bot/src/bin").mkdir(parents=True)
            (root / "crates/apps/bot/src/bin/review.rs").write_text('const COMMANDS: &[(&str, &str, &str)] = &[\n    ("leaks", "leaks", "x"),\n];\n')
            (root / "crates/apps/bot/src/bin/archive.rs").write_text('match x { Some("run") => {} Some(cmd @ ("compact" | "unpack")) => {} }\n')
            (root / "scripts").mkdir()
            (root / "scripts/check.sh").write_text("")
            (root / "README.md").write_text(
                "Run `scripts/check.sh`, `scripts/gone.sh:12`, `crates/<name>/src` and `crates/*/Cargo.toml`.\n"
                "Then `review leaks`, `./target/release/review nope`, `archive compact` and `archive vacuum`.\n")
            old_root, old_live = dc.ROOT, dc.LIVE
            dc.ROOT, dc.LIVE = root, ["README.md"]
            try:
                problems = []
                import contextlib, io
                out = io.StringIO()
                with contextlib.redirect_stdout(out), contextlib.redirect_stderr(io.StringIO()):
                    code = dc.main([])
                problems = out.getvalue().splitlines()
            finally:
                dc.ROOT, dc.LIVE = old_root, old_live
            self.assertEqual(code, 1)
            self.assertEqual(problems, [
                "README.md:1: no such path `scripts/gone.sh`",
                "README.md:1: no such path `crates/*/Cargo.toml`",
                "README.md:2: `review nope` is not a review command",
                "README.md:2: `archive vacuum` is not a archive command",
            ])

    def test_a_build_output_the_repo_ignores_counts_as_present(self):
        # `web/dist` is built, not committed: a clean checkout (CI) has none, yet docs may name it.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            import subprocess
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            (root / ".gitignore").write_text("web/dist/\n")
            old_root = dc.ROOT
            dc.ROOT = root
            try:
                self.assertTrue(dc.exists("web/dist"))
                self.assertTrue(dc.exists("web/dist/index.html"))
                self.assertFalse(dc.exists("web/src/gone.tsx"))
            finally:
                dc.ROOT = old_root


if __name__ == "__main__":
    unittest.main()

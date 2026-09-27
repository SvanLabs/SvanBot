"""Release progress (scripts/progress.py): what the dashboard's update card is built from."""
import importlib.util
import json
import os
import tempfile
import unittest
from pathlib import Path


def load(env):
    os.environ.update(env)
    spec = importlib.util.spec_from_file_location("progress", Path(__file__).resolve().parent.parent / "progress.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class StartingPoint(unittest.TestCase):
    def test_a_run_starts_from_the_installed_build_not_the_checkout(self):
        # 2026-09-27: a release run by hand read "078e871 -> 078e871": the checkout's HEAD is the
        # commit being installed, not the one it replaces.
        with tempfile.TemporaryDirectory() as d:
            marker = Path(d) / "installed"
            marker.write_text("aaaaaaa\n")
            mod = load({"SV10_PROGRESS_DIR": d, "SV10_INSTALLED_MARKER": str(marker)})
            self.assertEqual(mod.main(["start"]), 0)
            self.assertEqual(mod.main(["installed", "bbbbbbb"]), 0)
            st = json.loads((Path(d) / "release-progress.json").read_text())
            self.assertEqual((st["from"], st["commit"]), ("aaaaaaa", "bbbbbbb"))


if __name__ == "__main__":
    unittest.main()

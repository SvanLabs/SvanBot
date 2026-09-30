"""Release progress (scripts/progress.py): what the dashboard's update card is built from."""
import importlib.util
import json
import os
import subprocess
import sys
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
    def test_progress_writer_records_its_long_lived_parent(self):
        with tempfile.TemporaryDirectory() as d:
            script = Path(__file__).resolve().parent.parent / "progress.py"
            env = dict(os.environ, SV10_PROGRESS_DIR=d, SV10_INSTALLED_MARKER=str(Path(d) / "absent"))
            subprocess.run([sys.executable, str(script), "start"], env=env, check=True)
            state = json.loads((Path(d) / "release-progress.json").read_text())
            self.assertEqual(state["pid"], os.getpid())
            self.assertTrue(state["process_start"])
            self.assertTrue(state["boot_id"])
            subprocess.run([sys.executable, str(script), "stage", "fetch"], env=env, check=True)
            later = json.loads((Path(d) / "release-progress.json").read_text())
            for field in ["pid", "process_start", "boot_id"]:
                self.assertEqual(later[field], state[field])

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


class NothingToInstall(unittest.TestCase):
    def test_a_run_with_nothing_to_install_is_neither_installed_nor_failed(self):
        # #394: a checkout ahead of the update branch has no build to install and no fault to report.
        # `installed` would name a commit that was never built and `fail` would colour the card red,
        # so the run ends in its own state, carrying the sentence that says why.
        with tempfile.TemporaryDirectory() as d:
            mod = load({"SV10_PROGRESS_DIR": d, "SV10_INSTALLED_MARKER": str(Path(d) / "absent")})
            self.assertEqual(mod.main(["start"]), 0)
            self.assertEqual(mod.main(["stage", "fetch"]), 0)
            self.assertEqual(mod.main(["current", "ahead of main; nothing to install"]), 0)
            st = json.loads((Path(d) / "release-progress.json").read_text())
            self.assertEqual(st["state"], "current")
            self.assertIsNone(st["commit"])
            self.assertEqual(st["message"], "ahead of main; nothing to install")
            # The fetch stage closed rather than being left running for the panel to spin on.
            self.assertEqual([(s["name"], s["state"]) for s in st["stages"]], [("fetch", "done")])


if __name__ == "__main__":
    unittest.main()

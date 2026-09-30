"""Resource budgets use usable capacity rather than the reference machine's size."""
import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("host_resources", ROOT / "scripts/host_resources.py")


class ResourceBudgets(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.resources = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.resources)

    def test_nested_cgroup_memory_uses_remaining_parent_capacity(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            leaf = root / "group/child"
            leaf.mkdir(parents=True)
            (root / "group/memory.max").write_text(str(1024 << 20))
            (root / "group/memory.current").write_text(str(768 << 20))
            (leaf / "memory.max").write_text("max")
            self.assertEqual(self.resources.memory_available(8 << 30, leaf, root), 256 << 20)

    def test_unknown_cgroup_usage_does_not_assume_the_limit_is_free(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "memory.max").write_text(str(512 << 20))
            self.assertEqual(self.resources.memory_available(8 << 30, root, root), 0)

    def test_cpu_budget_respects_affinity_and_parent_quota(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            leaf = root / "child"
            leaf.mkdir()
            (root / "cpu.max").write_text("150000 100000")
            (leaf / "cpu.max").write_text("max 100000")
            self.assertEqual(self.resources.cpu_available(32, leaf, root), 1)

    def test_low_memory_uses_one_build_job_and_large_hosts_scale(self):
        self.assertEqual(self.resources.build_jobs(64, 256 << 20), 1)
        self.assertEqual(self.resources.build_jobs(8, 16 << 30), 8)

    def test_disk_preflight_uses_user_available_blocks_and_keeps_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)
            sentinel = path / "user-data"
            sentinel.write_text("keep")
            stats = type("Stat", (), {"f_bavail": 10, "f_bfree": 10000, "f_frsize": 4096})()
            with patch.object(self.resources.os, "statvfs", return_value=stats):
                with self.assertRaisesRegex(ValueError, "free"):
                    self.resources.check_disk(path / "not-created", 1 << 20)
            self.assertEqual(sentinel.read_text(), "keep")


if __name__ == "__main__":
    unittest.main()

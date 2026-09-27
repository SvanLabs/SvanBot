"""fleet-check.py: hands the export shows without us, told apart from hot-swap gaps."""
import datetime as dt
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("fleet_check", Path(__file__).resolve().parent.parent / "fleet-check.py")
fc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fc)


class SwapWindow(unittest.TestCase):
    def test_a_hand_running_at_a_hot_swap_is_a_swap_gap(self):
        # 2026-09-22: Svanism's 8393ab14 started 19:09:38 UTC; the swap exited at 19:10:41 UTC.
        swaps = [dt.datetime(2026, 9, 22, 19, 10, 41, tzinfo=dt.timezone.utc)]
        self.assertTrue(fc.at_swap("2026-09-22T19:09:38.955804+00:00", swaps))
        self.assertTrue(fc.at_swap("2026-09-22T19:10:30+00:00", swaps))
        # Long before or after the swap: something else played our seat (0151).
        self.assertFalse(fc.at_swap("2026-09-22T19:02:00+00:00", swaps))
        self.assertFalse(fc.at_swap("2026-09-22T19:13:00+00:00", swaps))
        self.assertFalse(fc.at_swap("2026-09-22T19:09:38+00:00", []))

    def test_swap_times_are_read_from_log_lines_in_local_time(self):
        lines = [
            "\x1b[2m2026-09-22 21:10:41.816+02:00\x1b[0m \x1b[32m INFO\x1b[0m hot swap: exiting to restart bot=\"fleet\"",
            "2026-09-22 21:11:00.000+02:00  INFO something else",
        ]
        self.assertEqual(fc.swap_times(lines), [dt.datetime(2026, 9, 22, 19, 10, 41, 816000, tzinfo=dt.timezone.utc)])
        # A full restart (setup save, stop/start, keepalive) leaves the same gap as a hot swap.
        restart = ["2026-09-23 00:55:19.746+02:00  INFO svanbot10 starting: 5 bots, 282 known opponents, dry_run=false"]
        self.assertEqual(fc.swap_times(restart), [dt.datetime(2026, 9, 22, 22, 55, 19, 746000, tzinfo=dt.timezone.utc)])


class RenamedBots(unittest.TestCase):
    def test_names_linked_by_key_or_alias_form_one_group(self):
        groups = fc.name_groups(['["SvanBotV10", "SvanBotV7"]'], "SvanBotV10:SvanBotV7,X:Y", ["SvanBotV10", "SvanBotV7", "Svanar"])
        self.assertEqual(groups["SvanBotV7"], groups["SvanBotV10"])
        self.assertNotEqual(groups["Svanar"], groups["SvanBotV10"])
        self.assertEqual(groups["Y"], groups["X"])


if __name__ == "__main__":
    unittest.main()

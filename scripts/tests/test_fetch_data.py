"""Restoration cannot proceed while any fleet writer or supervisor is alive."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class RestoreWriterGuard(unittest.TestCase):
    def test_live_writers_are_refused_before_fetch_even_with_force(self):
        for name in ('bot.pid', 'head.pid', 'worker-bot1.pid', 'supervisor.pid',
                     'head-supervisor.pid', 'worker-bot1-supervisor.pid', 'learner.pid',
                     'analyst.pid', 'learner-supervisor.pid', 'analyst-supervisor.pid'):
            with self.subTest(pidfile=name), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                (root / 'scripts').mkdir()
                (root / 'artifacts').mkdir()
                (root / 'tools').mkdir()
                shutil.copy(ROOT / 'scripts/fetch-data.sh', root / 'scripts')
                (root / 'artifacts' / name).write_text(str(os.getpid()))
                (root / 'tools/gh').write_text('#!/bin/sh\ntouch artifacts/unwanted-fetch\nexit 17\n')
                (root / 'tools/gh').chmod(0o755)
                env = dict(os.environ, FORCE='1', SVANBOT_DATA_REPO='fixture/data',
                           PATH=f"{root / 'tools'}:{os.environ['PATH']}")
                result = subprocess.run(['bash', 'scripts/fetch-data.sh'], cwd=root,
                                        env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, 1)
                self.assertFalse((root / 'artifacts/unwanted-fetch').exists(),
                                 f'restore ignored active {name} and attempted fetching live replacements')
                self.assertIn('stop it first', result.stderr)

    def test_stale_pids_do_not_prevent_fetch_and_derived_ignores_live_writers(self):
        for derived in (False, True):
            with self.subTest(derived=derived), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                (root / 'scripts').mkdir()
                (root / 'artifacts').mkdir()
                (root / 'tools').mkdir()
                shutil.copy(ROOT / 'scripts/fetch-data.sh', root / 'scripts')
                (root / 'artifacts/head-supervisor.pid').write_text(
                    str(os.getpid() if derived else 1 << 22))
                (root / 'tools/gh').write_text('#!/bin/sh\ntouch artifacts/allowed-fetch\nexit 17\n')
                (root / 'tools/gh').chmod(0o755)
                env = dict(os.environ, SVANBOT_DATA_REPO='fixture/data',
                           PATH=f"{root / 'tools'}:{os.environ['PATH']}")
                command = ['bash', 'scripts/fetch-data.sh'] + (['--derived'] if derived else [])
                result = subprocess.run(command, cwd=root, env=env, capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertTrue((root / 'artifacts/allowed-fetch').exists())


if __name__ == '__main__':
    unittest.main()

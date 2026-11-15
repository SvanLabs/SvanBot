"""Restoration cannot proceed while any fleet writer or supervisor is alive."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
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


FAKE_GH = """#!/bin/sh
# Fixture gh: one data-1 release whose files are copied from $FIXTURE_DIR.
case "$2" in
  list) echo data-1 ;;
  download) while [ $# -gt 0 ]; do [ "$1" = --dir ] && dir=$2; shift; done
            cp "$FIXTURE_DIR"/* "$dir"/ ;;
esac
"""


def restore_tree(temp, *, history_ok):
    """A scratch deployment holding live-looking databases, and a release to restore from."""
    root = Path(temp) / 'root'
    (root / 'scripts').mkdir(parents=True)
    (root / 'artifacts').mkdir()
    (root / 'tools').mkdir()
    fixture = Path(temp) / 'fixture'
    fixture.mkdir()
    shutil.copy(ROOT / 'scripts/fetch-data.sh', root / 'scripts')
    (root / 'tools/gh').write_text(FAKE_GH)
    (root / 'tools/gh').chmod(0o755)
    for name, ok in (('svanbot10.db', True), ('history.db', history_ok)):
        raw = Path(temp) / name
        raw.write_text('restored ' + name)
        subprocess.run(['zstd', '-q', str(raw), '-o', str(fixture / (name + '.zst'))], check=True)
        if not ok:
            (fixture / (name + '.zst')).write_bytes(b'not zstd')
    misc = Path(temp) / 'misc'
    misc.mkdir()
    (misc / 'note').write_text('x')
    subprocess.run(f'tar -C {misc} -cf - . | zstd -q -o {fixture}/misc.tar.zst', shell=True, check=True)
    subprocess.run('sha256sum * > SHA256SUMS', shell=True, cwd=fixture, check=True)
    (root / 'artifacts/svanbot10.db').write_text('live svanbot10.db')
    (root / 'artifacts/history.db').write_text('live history.db')
    env = dict(os.environ, FORCE='1', SVANBOT_DATA_REPO='fixture/data', FIXTURE_DIR=str(fixture),
               PATH=f"{root / 'tools'}:{os.environ['PATH']}")
    return root, env


class RestoreIsSafe(unittest.TestCase):
    def test_restore_waits_for_no_release_operation(self):
        """A release, snapshot or rollback holds release-operation.lock; a restore must not run beside it (#1028)."""
        with tempfile.TemporaryDirectory() as temp:
            root, env = restore_tree(temp, history_ok=True)
            holder = subprocess.Popen(['flock', str(root / 'artifacts/release-operation.lock'), 'sleep', '30'])
            try:
                time.sleep(0.5)
                result = subprocess.run(['bash', 'scripts/fetch-data.sh'], cwd=root, env=env,
                                        capture_output=True, text=True, timeout=20)
            finally:
                holder.kill()
                holder.wait()
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual((root / 'artifacts/svanbot10.db').read_text(), 'live svanbot10.db')

    def test_failed_restore_leaves_the_live_databases_alone(self):
        """A bad second database must not leave the first already overwritten (#1028)."""
        with tempfile.TemporaryDirectory() as temp:
            root, env = restore_tree(temp, history_ok=False)
            result = subprocess.run(['bash', 'scripts/fetch-data.sh'], cwd=root, env=env,
                                    capture_output=True, text=True, timeout=20)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual((root / 'artifacts/svanbot10.db').read_text(), 'live svanbot10.db')
            self.assertEqual((root / 'artifacts/history.db').read_text(), 'live history.db')
            self.assertEqual(sorted(p.name for p in (root / 'artifacts').glob('*.new')), [])

    def test_good_restore_replaces_both(self):
        with tempfile.TemporaryDirectory() as temp:
            root, env = restore_tree(temp, history_ok=True)
            result = subprocess.run(['bash', 'scripts/fetch-data.sh'], cwd=root, env=env,
                                    capture_output=True, text=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((root / 'artifacts/svanbot10.db').read_text(), 'restored svanbot10.db')
            self.assertEqual((root / 'artifacts/history.db').read_text(), 'restored history.db')
            self.assertEqual((root / 'artifacts/note').read_text(), 'x')


if __name__ == '__main__':
    unittest.main()

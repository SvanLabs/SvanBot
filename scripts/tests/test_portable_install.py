"""Portable distribution must preserve the default fleet's analyst binary."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class PortableAnalyst(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / 'scripts').mkdir()
        (self.root / 'tools').mkdir()
        self.env = dict(os.environ, LEVEL='v2', PATH=f"{self.root / 'tools'}:{os.environ['PATH']}")

    def executable(self, path, body):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text('#!/usr/bin/env bash\nset -euo pipefail\n' + body + '\n')
        path.chmod(0o755)

    def test_installer_preserves_analyst(self):
        shutil.copy(ROOT / 'scripts/install.sh', self.root / 'scripts')
        for name in ('sv10-bot', 'probe', 'analyst'):
            self.executable(self.root / 'bin/x86-64-v2' / name, 'echo fixture')
        subprocess.run(['bash', 'scripts/install.sh'], cwd=self.root, env=self.env,
                       check=True, capture_output=True, text=True)
        self.assertTrue(os.access(self.root / 'target/release/analyst', os.X_OK),
                        'default startup runs analyst, but portable installation omitted it')

    def test_bundle_contains_analyst_for_each_cpu_level(self):
        shutil.copy(ROOT / 'scripts/portable.sh', self.root / 'scripts')
        (self.root / 'Cargo.toml').write_text('version = "1.0.0"\n')
        (self.root / 'web/dist').mkdir(parents=True)
        (self.root / 'web/dist/index.html').write_text('fixture')
        (self.root / 'docs').mkdir()
        for name in ('.env.example', 'CLAUDE.md'):
            (self.root / name).write_text('fixture')
        self.executable(self.root / 'tools/git', 'echo fixture')
        self.executable(self.root / 'tools/strip', ':')
        self.executable(self.root / 'tools/npm', ':')
        self.executable(self.root / 'tools/objdump', 'echo GLIBC_2.17')
        # Intercept nice rather than cargo: portable.sh prepends the real cargo bin directory.
        self.executable(self.root / 'tools/nice', '''
while [ "$1" != --target-dir ]; do shift; done
shift
mkdir -p "$1/release"
for name in sv10-bot learner analyst sim probe tables review calibrate ingest archive; do
  printf '#!/bin/sh\\necho fixture\\n' > "$1/release/$name"
  chmod +x "$1/release/$name"
done''')
        subprocess.run(['bash', 'scripts/portable.sh'], cwd=self.root, env=self.env,
                       check=True, capture_output=True, text=True)
        bundle = self.root / 'target/dist/svanbot-1.0.0-fixture-x86_64-linux-gnu/bin'
        for level in ('v2', 'v3'):
            self.assertTrue(os.access(bundle / f'x86-64-{level}/analyst', os.X_OK),
                            f'{level} bundle omitted the analyst that default startup runs')


if __name__ == '__main__':
    unittest.main()

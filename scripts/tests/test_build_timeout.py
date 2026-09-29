"""Build-comparison timeouts must retain captured subprocess diagnostics."""
import importlib.util
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    'build_ab', Path(__file__).resolve().parents[1] / 'build-ab.py')
build_ab = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build_ab)


class BuildTimeout(unittest.TestCase):
    def test_timed_children_preserve_stdout_stderr_and_empty_output(self):
        cases = [
            ('stdout', 'print("compile progress",flush=True)', 'compile progress'),
            ('stderr', 'print("compiler error",file=sys.stderr,flush=True)', 'compiler error'),
            ('empty', 'pass', ''),
            ('invalid', 'sys.stdout.buffer.write(bytes([255]));sys.stdout.flush()', '\ufffd'),
        ]
        for name, output, expected in cases:
            with self.subTest(stream=name), tempfile.TemporaryDirectory() as temp:
                args = SimpleNamespace(target_root='target/dev', commit='', timeout=0.3,
                    job=['-c', f'import sys,time; {output}; time.sleep(5)'])
                with patch.object(build_ab, 'ROOT', temp), patch.object(
                    build_ab.shutil, 'which', return_value=sys.executable):
                    runner = build_ab.Runner(args, None)
                    with self.assertRaises(SystemExit) as failure:
                        runner.build('off256', 'false', 256, None, 'warm', False, False, 0)
                log = Path(temp) / 'target/dev/logs/off256-warm.log'
                self.assertTrue(log.exists(), 'a timeout still needs its diagnostic log')
                text = log.read_text()
                self.assertIn('# exit -9', text)
                self.assertIn(expected, text)
                self.assertIn(str(log), str(failure.exception))


if __name__ == '__main__':
    unittest.main()

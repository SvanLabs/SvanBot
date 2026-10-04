"""Benchmark intervals must not understate finite-sample Student-t uncertainty."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ConservativeCriticalValues(unittest.TestCase):
    def test_each_tool_covers_independent_nist_reference_values(self):
        # NIST/SEMATECH handbook, Student-t critical values, probability0.975 column.
        references = {1: 12.706, 2: 4.303, 3: 3.182, 5: 2.571, 7: 2.365, 9: 2.262,
                      10: 2.228, 11: 2.201, 14: 2.145, 19: 2.093, 29: 2.045, 30: 2.042}
        for name in ('build-ab',):  # `ab` has the same table, tested in sv10-stats
            tool = load(name)
            for df, critical in references.items():
                with self.subTest(tool=name, df=df):
                    self.assertGreaterEqual(tool.t975(df), critical)
            self.assertGreater(tool.t975(1000), 1.96,
                               'finite samples retain a conservative t bound rather than normal fallback')
            values = [tool.t975(df) for df in range(1, 100)]
            self.assertTrue(all(a >= b for a, b in zip(values, values[1:])))


if __name__ == '__main__':
    unittest.main()

"""Live WAL database reports must include committed pages without writing the database."""
import importlib.util
from pathlib import Path
import sqlite3
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    'resource_report', Path(__file__).resolve().parents[1] / 'resource-report.py')
report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(report)


class LiveWalTables(unittest.TestCase):
    def test_committed_wal_pages_are_visible_without_checkpoint(self):
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / 'live.db'
            writer = sqlite3.connect(path)
            self.addCleanup(writer.close)
            writer.execute('PRAGMA journal_mode=WAL')
            writer.execute('PRAGMA wal_autocheckpoint=0')
            writer.execute('CREATE TABLE live_data(value BLOB)')
            writer.execute('INSERT INTO live_data VALUES (?)', (b'x' * 100000,))
            writer.commit()
            before = {p.name: p.read_bytes() for p in (path, Path(str(path) + '-wal'))}
            sizes = dict(report.db_tables(path))
            self.assertGreater(sizes.get('live_data', 0), 100000)
            self.assertEqual(before, {p.name: p.read_bytes() for p in (path, Path(str(path) + '-wal'))})
            writer.close()


if __name__ == '__main__':
    unittest.main()

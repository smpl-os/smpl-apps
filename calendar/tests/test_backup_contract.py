from pathlib import Path
from contextlib import closing
import hashlib
import json
import os
import sqlite3
import stat
import subprocess
import sys
import tempfile
import unittest


class CalendarBackupTests(unittest.TestCase):
    def test_live_wal_snapshot_preserves_events_and_dedup_privately(self):
        script = Path(__file__).resolve().parents[2] / "scripts/backup-calendar-data.py"
        with tempfile.TemporaryDirectory(prefix="calendar-backup-test-") as home:
            database = Path(home) / ".local/share/smplos/calendar/events.db"
            database.parent.mkdir(parents=True)
            with closing(sqlite3.connect(database)) as source:
                source.execute("PRAGMA journal_mode=WAL")
                source.executescript("""
                    CREATE TABLE events(id INTEGER PRIMARY KEY, title TEXT);
                    CREATE TABLE sent_alerts(event_id INTEGER PRIMARY KEY);
                    INSERT INTO events VALUES(1, 'Synthetic fixture');
                    INSERT INTO sent_alerts VALUES(1);
                """)
                source.commit()
                self.assertTrue(Path(str(database) + "-wal").exists())
                env = dict(os.environ, HOME=home, XDG_STATE_HOME=home + "/state")
                result = subprocess.run([sys.executable, str(script)], env=env,
                                        capture_output=True, text=True, check=True, timeout=15)
                manifest = json.loads(result.stdout)
                backup = Path(manifest["database"])
                self.assertEqual(manifest["integrity_check"], "ok")
                self.assertEqual(hashlib.sha256(backup.read_bytes()).hexdigest(), manifest["sha256"])
                self.assertEqual(stat.S_IMODE(backup.stat().st_mode), 0o600)
                self.assertEqual(stat.S_IMODE(backup.parent.stat().st_mode), 0o700)
                with closing(sqlite3.connect(backup)) as restored:
                    self.assertEqual(restored.execute("SELECT COUNT(*) FROM events").fetchone(), (1,))
                    self.assertEqual(restored.execute("SELECT event_id FROM sent_alerts").fetchone(), (1,))
                    self.assertEqual(restored.execute("PRAGMA integrity_check").fetchone(), ("ok",))
                self.assertEqual(source.execute("SELECT COUNT(*) FROM events").fetchone(), (1,))


if __name__ == "__main__":
    unittest.main()

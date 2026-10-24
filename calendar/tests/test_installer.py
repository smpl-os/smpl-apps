import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest
from contextlib import closing


ROOT = Path(__file__).resolve().parents[2]
INSTALLER = ROOT / "scripts/install-calendar-rpm.sh"


class CalendarInstallerTests(unittest.TestCase):
    def setUp(self):
        self.fixture = tempfile.TemporaryDirectory(prefix="calendar-installer-")
        self.addCleanup(self.fixture.cleanup)
        self.root = Path(self.fixture.name)
        self.home = self.root / "home"
        self.home.mkdir()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        (self.root / "tmp").mkdir()
        self.rpm = self.root / "download.rpm"
        self.rpm.write_bytes(b"synthetic RPM fixture")
        self.metadata = {
            "tag_name": "v0.8.35", "draft": False, "prerelease": False,
            "assets": [{
                "name": "smpl-calendar-0.8.35-1.fc43.x86_64.rpm",
                "state": "uploaded", "size": self.rpm.stat().st_size,
                "digest": f"sha256:{hashlib.sha256(self.rpm.read_bytes()).hexdigest()}",
                "browser_download_url": "https://github.com/smpl-os/smpl-apps/releases/download/"
                                        "v0.8.35/smpl-calendar-0.8.35-1.fc43.x86_64.rpm",
            }],
        }
        self.env = dict(
            os.environ, HOME=str(self.home), PATH=f"{self.bin}:{os.environ['PATH']}",
            TMPDIR=str(self.root / "tmp"), FIXTURE=str(self.root),
            XDG_CONFIG_HOME=str(self.home / ".config"),
            XDG_STATE_HOME=str(self.home / ".local/state"),
            XDG_CURRENT_DESKTOP="",
        )
        self.helper("id", 'print(os.environ.get("FIXTURE_UID", "1000"))')
        self.helper("uname", 'print("Linux" if sys.argv[1] == "-s" else "x86_64")')
        self.helper("dnf", 'raise RuntimeError("DNF must only be called through the fixture pkexec")')
        self.helper("rpm", 'print("smpl-calendar\\t0.8.35\\t1.fc43\\tx86_64")')
        self.helper("curl", '''
import shutil
args = sys.argv[1:]
url = next(arg for arg in args if arg.startswith("https://"))
target = Path(args[args.index("-o") + 1])
if url.endswith("/releases/latest"):
    shutil.copyfile(root / "release.json", target)
elif url.endswith(".rpm"):
    shutil.copyfile(root / "download.rpm", target)
elif url.endswith("/scripts/backup-calendar-data.py"):
    if os.environ.get("FAIL_BACKUP_DOWNLOAD") == "1":
        sys.exit(22)
    shutil.copyfile(os.environ["BACKUP_HELPER"], target)
else:
    raise RuntimeError(f"Unexpected download: {url}")
''')
        self.helper("pkexec", '''
import json
backup_base = Path(os.environ["XDG_STATE_HOME"]) / "smplos/calendar/backups"
backups = sorted(str(path) for path in backup_base.glob("*/events.db"))
configs = sorted(str(path) for path in backup_base.glob("*/calendar.json"))
(root / "install.json").write_text(json.dumps({
    "args": sys.argv[1:], "backups": backups, "configs": configs,
}))
sys.exit(int(os.environ.get("INSTALL_STATUS", "0")))
''')
        self.env["BACKUP_HELPER"] = str(ROOT / "scripts/backup-calendar-data.py")
        self.helper("smpl-calendar-kde-shortcut", '''
assert (root / "install.json").exists(), "Shortcut setup must follow successful installation"
(root / "shortcut-called").touch()
sys.exit(int(os.environ.get("SHORTCUT_STATUS", "0")))
''')

    def helper(self, name, body):
        path = self.bin / name
        path.write_text(
            "#!/usr/bin/python3\nimport os\nfrom pathlib import Path\nimport sys\n"
            "root = Path(os.environ['FIXTURE'])\n" + body + "\n"
        )
        path.chmod(0o755)

    def run_installer(self, *args):
        (self.root / "release.json").write_text(json.dumps(self.metadata))
        return subprocess.run(["bash", str(INSTALLER), *args], env=self.env,
                              text=True, capture_output=True, timeout=20)

    def create_data(self):
        database = self.home / ".local/share/smplos/calendar/events.db"
        database.parent.mkdir(parents=True)
        with closing(sqlite3.connect(database)) as connection, connection:
            connection.executescript("""
                CREATE TABLE events (title TEXT);
                INSERT INTO events VALUES ('Existing recurring event');
                CREATE TABLE sent_alerts (event_id INTEGER);
                INSERT INTO sent_alerts VALUES (42);
            """)
        preferences = self.home / ".config/smplos/calendar.json"
        preferences.parent.mkdir(parents=True)
        preferences.write_text('{"cities": ["saved fixture city"]}\n')
        return database, preferences

    def test_fresh_install_uses_dnf_install_without_creating_user_data(self):
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        install = json.loads((self.root / "install.json").read_text())
        self.assertEqual(install["args"][:3], ["dnf", "install", "--assumeyes"])
        self.assertEqual(install["backups"], [])
        self.assertFalse((self.home / ".local/share/smplos/calendar/events.db").exists())
        self.assertEqual(list((self.root / "tmp").iterdir()), [])

    def test_update_verifies_backup_and_preferences_before_dnf_without_resetting_data(self):
        database, preferences = self.create_data()
        original = database.read_bytes(), preferences.read_bytes()
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        install = json.loads((self.root / "install.json").read_text())
        self.assertEqual(len(install["backups"]), 1)
        self.assertEqual(len(install["configs"]), 1)
        with closing(sqlite3.connect(install["backups"][0])) as connection:
            self.assertEqual(connection.execute("PRAGMA integrity_check").fetchone(), ("ok",))
            self.assertEqual(connection.execute("SELECT * FROM events").fetchall(),
                             [("Existing recurring event",)])
            self.assertEqual(connection.execute("SELECT * FROM sent_alerts").fetchall(), [(42,)])
        self.assertEqual(Path(install["configs"][0]).read_bytes(), preferences.read_bytes())
        self.assertEqual((database.read_bytes(), preferences.read_bytes()), original)

    def test_backup_includes_committed_wal_data_while_database_is_open(self):
        database, _ = self.create_data()
        with closing(sqlite3.connect(database)) as writer:
            writer.execute("PRAGMA journal_mode=WAL")
            writer.execute("INSERT INTO events VALUES ('Committed WAL event')")
            writer.commit()
            self.assertTrue(Path(f"{database}-wal").is_file())
            result = self.run_installer()
            self.assertEqual(result.returncode, 0, result.stderr)
            install = json.loads((self.root / "install.json").read_text())
            with closing(sqlite3.connect(install["backups"][0])) as backup:
                self.assertEqual(backup.execute("SELECT title FROM events ORDER BY rowid").fetchall(),
                                 [("Existing recurring event",), ("Committed WAL event",)])

    def test_failed_install_retains_verified_data_backup(self):
        self.create_data()
        self.env["INSTALL_STATUS"] = "1"
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        install = json.loads((self.root / "install.json").read_text())
        self.assertTrue(Path(install["backups"][0]).is_file())
        self.assertNotIn("installation/update completed", result.stdout)

    def test_bad_checksum_missing_asset_and_untrusted_url_never_invoke_dnf(self):
        original = json.loads(json.dumps(self.metadata))
        for failure in ("checksum", "missing", "url"):
            self.metadata = json.loads(json.dumps(original))
            if failure == "checksum":
                self.metadata["assets"][0]["digest"] = "sha256:" + "0" * 64
            elif failure == "missing":
                self.metadata["assets"] = []
            else:
                self.metadata["assets"][0]["browser_download_url"] = "https://example.com/app.rpm"
            with self.subTest(failure=failure):
                result = self.run_installer()
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((self.root / "install.json").exists())
                self.assertEqual(list((self.root / "tmp").iterdir()), [])

    def test_corrupt_database_and_backup_download_failure_abort_before_install(self):
        database, _ = self.create_data()
        self.env["FAIL_BACKUP_DOWNLOAD"] = "1"
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / "install.json").exists())
        del self.env["FAIL_BACKUP_DOWNLOAD"]
        database.write_bytes(b"corrupt fixture database")
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / "install.json").exists())
        self.assertEqual(database.read_bytes(), b"corrupt fixture database")

    def test_root_is_rejected_before_download_or_install(self):
        self.env["FIXTURE_UID"] = "0"
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("not root", result.stderr)
        self.assertFalse((self.root / "install.json").exists())

    def test_kde_automatically_sets_shortcut_after_install(self):
        self.env["XDG_CURRENT_DESKTOP"] = "fedora:KDE"
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.root / "shortcut-called").exists())
        self.assertIn("conflicting bindings", result.stdout)

    def test_shortcut_can_be_skipped_on_kde_or_requested_explicitly(self):
        self.env["XDG_CURRENT_DESKTOP"] = "KDE"
        result = self.run_installer("--no-kde-shortcut")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.root / "shortcut-called").exists())
        self.env.pop("XDG_CURRENT_DESKTOP")
        result = self.run_installer("--kde-shortcut")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue((self.root / "shortcut-called").exists())

    def test_non_kde_and_unset_desktop_do_not_change_shortcuts(self):
        self.env.pop("XDG_CURRENT_DESKTOP")
        result = self.run_installer()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.root / "shortcut-called").exists())
        self.assertIn("shortcut setup skipped", result.stdout)

    def test_shortcut_failure_is_explicit_without_claiming_full_success(self):
        self.env["XDG_CURRENT_DESKTOP"] = "KDE"
        self.env["SHORTCUT_STATUS"] = "1"
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Calendar is installed, but KDE shortcut setup failed", result.stderr)
        self.assertNotIn("installation/update completed", result.stdout)

    def test_failed_dnf_never_changes_shortcuts(self):
        self.env["XDG_CURRENT_DESKTOP"] = "KDE"
        self.env["INSTALL_STATUS"] = "1"
        result = self.run_installer()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / "shortcut-called").exists())


if __name__ == "__main__":
    unittest.main()

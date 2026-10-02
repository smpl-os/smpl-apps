from contextlib import ExitStack, closing, redirect_stdout
import importlib.util
import io
import json
import os
from pathlib import Path
import sqlite3
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[2] / "scripts/deploy-calendar-reminders-local.py"
SPEC = importlib.util.spec_from_file_location("calendar_installer", SCRIPT)
INSTALLER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(INSTALLER)


class ReminderDeployTests(unittest.TestCase):
    def test_history_verification_allows_new_delivery_but_detects_missing_receipts(self):
        with tempfile.TemporaryDirectory(prefix="calendar-history-test-") as directory:
            database = Path(directory) / "events.db"
            with closing(sqlite3.connect(database)) as db:
                self.assertEqual(INSTALLER.legacy_receipts(database), set())
                with db:
                    db.execute("CREATE TABLE sent_alerts (event_id INTEGER, alert_ts INTEGER)")
                    db.execute("INSERT INTO sent_alerts VALUES (1, 100)")
                before = INSTALLER.legacy_receipts(database)
                self.assertEqual(before, {(1, 100)})
                with db:
                    db.execute("INSERT INTO sent_alerts VALUES (2, 200)")
                self.assertTrue(before.issubset(INSTALLER.legacy_receipts(database)))
                with db:
                    db.execute("DELETE FROM sent_alerts WHERE event_id=1")
                self.assertFalse(before.issubset(INSTALLER.legacy_receipts(database)))

    def test_explicit_service_optouts_and_customizations_are_not_overwritten(self):
        for state in [
            {"LoadState": "masked"},
            {"LoadState": "loaded", "UnitFileState": "disabled"},
            {"LoadState": "loaded", "UnitFileState": "enabled", "FragmentPath": "/custom/service"},
        ]:
            with self.assertRaises(RuntimeError):
                INSTALLER.preflight_unit(SCRIPT, state)

    def test_only_revalidated_old_inode_is_signaled_via_pidfd(self):
        old = {"pid": 123, "start_ticks": 99, "device": 1, "inode": 2}
        new = {"pid": 456, "start_ticks": 100, "device": 1, "inode": 3}
        with patch.object(INSTALLER, "daemon_processes", side_effect=[[old, new], [new]]), \
             patch.object(INSTALLER, "process_identity", return_value=old), \
             patch.object(INSTALLER.os, "pidfd_open", return_value=42) as opened, \
             patch.object(INSTALLER.signal, "pidfd_send_signal") as signaled, \
             patch.object(INSTALLER.os, "close") as closed, \
             patch.object(INSTALLER.time, "sleep"):
            self.assertEqual(INSTALLER.stop_old_daemons(SimpleNamespace(st_dev=1, st_ino=3)), 1)
            opened.assert_called_once_with(123)
            signaled.assert_called_once_with(42, INSTALLER.signal.SIGTERM)
            closed.assert_called_once_with(42)

    def test_reused_pid_is_never_signaled(self):
        old = {"pid": 123, "start_ticks": 99, "device": 1, "inode": 2}
        with patch.object(INSTALLER, "daemon_processes", side_effect=[[old], []]), \
             patch.object(INSTALLER, "process_identity", return_value={**old, "start_ticks": 100}), \
             patch.object(INSTALLER.os, "pidfd_open", return_value=42), \
             patch.object(INSTALLER.signal, "pidfd_send_signal") as signaled, \
             patch.object(INSTALLER.os, "close"), \
             patch.object(INSTALLER.time, "sleep"):
            self.assertEqual(INSTALLER.stop_old_daemons(SimpleNamespace(st_dev=1, st_ino=3)), 0)
            signaled.assert_not_called()

    def test_package_staging_keeps_verified_old_binaries(self):
        with tempfile.TemporaryDirectory(prefix="calendar-package-test-") as directory, ExitStack() as stack:
            root = Path(directory)
            binaries = root / "installed"
            binaries.mkdir()
            source = root / "target/release"
            source.mkdir(parents=True)
            unit_source = root / "calendar/systemd" / INSTALLER.UNIT
            unit_source.parent.mkdir(parents=True)
            unit_source.write_text("[Service]\nExecStart=/fixture --foreground\n")
            for name in INSTALLER.BINARIES:
                (binaries / name).write_bytes(b"old executable")
                (source / name).write_bytes(b"new executable")
            for name, value in {
                "ROOT": root, "BINARY_DIR": binaries, "UNIT_PATH": root / "units" / INSTALLER.UNIT,
                "BACKUP_BASE": root / "backups", "INSTALL_LOCK": root / "install.lock",
            }.items():
                stack.enter_context(patch.object(INSTALLER, name, value))
            stack.enter_context(patch.object(INSTALLER.os, "geteuid", return_value=0))
            stack.enter_context(patch.object(INSTALLER.os, "fchown"))
            stack.enter_context(patch.dict(os.environ, {"PKEXEC_UID": "1000"}))
            output = io.StringIO()
            with redirect_stdout(output):
                INSTALLER.install_root()
            result = json.loads(output.getvalue())
            self.assertEqual(len(result["backups"]), 2)
            self.assertEqual(len(result["installed"]), 3)
            for saved in result["backups"]:
                self.assertEqual(Path(saved["path"]).read_bytes(), b"old executable")
                self.assertEqual(INSTALLER.digest(saved["path"]), saved["sha256"])
            for installed in result["installed"]:
                self.assertEqual(INSTALLER.digest(installed["path"]), installed["sha256"])
            for name in INSTALLER.BINARIES:
                self.assertEqual((binaries / name).read_bytes(), b"new executable")


if __name__ == "__main__":
    unittest.main()

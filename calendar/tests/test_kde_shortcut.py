import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "kde_shortcut", ROOT / "calendar/packaging/setup-kde-shortcut.py")
shortcut = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shortcut)
OTHER = ("fixture", "open", "Other application", "Existing action")
ALTERNATIVE = (0x0C00004C,)


class KdeKeySequenceWireTests(unittest.TestCase):
    def setUp(self):
        self.api = shortcut.KdeShortcuts.__new__(shortcut.KdeShortcuts)
        self.api.dbus = mock.Mock()
        self.api.dbus.Array.side_effect = lambda values, signature: list(values)
        self.api.dbus.Struct.side_effect = lambda values, signature: tuple(values)

    def test_every_sequence_has_exactly_four_int32_slots(self):
        for length in range(1, 5):
            sequence = (shortcut.KEY[0], ord("D"), ord("E"), ord("F"))[:length]
            with self.subTest(length=length):
                wire = self.api.sequence(sequence)
                self.assertEqual(wire, (list(sequence) + [0] * (4 - length),))
                self.api.dbus.Array.assert_called_with(
                    tuple(sequence) + (0,) * (4 - length), signature="i")
                self.api.dbus.Struct.assert_called_with((wire[0],), signature="ai")

    def test_target_and_alternative_sequences_are_padded_in_shortcut_list(self):
        self.assertEqual(
            self.api.sequences((shortcut.KEY, ALTERNATIVE)),
            [([shortcut.KEY[0], 0, 0, 0],), ([ALTERNATIVE[0], 0, 0, 0],)],
        )
        self.assertEqual(self.api.dbus.Array.call_args.kwargs, {"signature": "(ai)"})

    def test_clearing_shortcuts_uses_empty_list_not_empty_sequence(self):
        self.assertEqual(self.api.sequences(()), [])
        self.api.dbus.Struct.assert_not_called()

    def test_invalid_sequence_lengths_are_rejected_before_marshalling(self):
        for sequence in ((), (1, 2, 3, 4, 5)):
            with self.subTest(sequence=sequence):
                with self.assertRaisesRegex(ValueError, "one to four"):
                    self.api.sequence(sequence)
        self.api.dbus.Array.assert_not_called()
        self.api.dbus.Struct.assert_not_called()


class FakeKde:
    def __init__(self, assignments=None):
        self.assignments = dict(assignments or {})
        self.failure = None

    def conflicts(self):
        return tuple(action for action, keys in self.assignments.items()
                     if any(shortcut.KEY[0] in key for key in keys))

    def registered(self, action):
        return action in self.assignments

    def keys(self, action):
        return self.assignments.get(action, ())

    def register(self, action):
        self.assignments.setdefault(action, ())

    def set_keys(self, action, keys, present=False):
        self.assignments[action] = tuple(keys)
        if self.failure == action:
            self.failure = None
            raise RuntimeError("Synthetic KDE failure after applying keys")

    def unregister(self, action):
        del self.assignments[action]


class KdeShortcutTests(unittest.TestCase):
    def setUp(self):
        fixture = tempfile.TemporaryDirectory(prefix="calendar-kde-shortcut-")
        self.addCleanup(fixture.cleanup)
        self.home = Path(fixture.name)
        self.config = self.home / ".config"
        self.config.mkdir()
        self.settings = self.config / "kglobalshortcutsrc"
        self.settings.write_text("[fixture]\nopen=Meta+Alt+C,none,Existing action\n")
        self.env = mock.patch.dict(os.environ, {
            "XDG_CONFIG_HOME": str(self.config),
            "XDG_STATE_HOME": str(self.home / ".local/state"),
        })
        self.env.start()
        self.addCleanup(self.env.stop)
        self.saved = mock.patch.object(shortcut, "wait_for_saved_binding")
        self.saved.start()
        self.addCleanup(self.saved.stop)

    def backups(self):
        return list((self.home / ".local/state/smplos/calendar/backups").glob("kde-shortcut.*"))

    def test_assigns_new_shortcut_and_verifies_private_backup(self):
        api = FakeKde()
        shortcut.configure(api)
        self.assertEqual(api.keys(shortcut.ACTION), (shortcut.KEY,))
        backup, = self.backups()
        self.assertEqual((backup / "kglobalshortcutsrc").read_bytes(), self.settings.read_bytes())
        self.assertEqual(backup.stat().st_mode & 0o777, 0o700)
        self.assertEqual((backup / "actions.json").stat().st_mode & 0o777, 0o600)
        snapshot = json.loads((backup / "actions.json").read_text())
        self.assertFalse(snapshot[0]["registered"])

    def test_reassigns_all_conflicts_and_preserves_unrelated_keys(self):
        context = ("fixture|editor", "edit", "Other application", "Editor action")
        multikey = (shortcut.KEY[0], ord("D"))
        api = FakeKde({
            OTHER: (shortcut.KEY, ALTERNATIVE),
            context: (multikey, ALTERNATIVE),
            shortcut.ACTION: (ALTERNATIVE,),
        })
        shortcut.configure(api)
        self.assertEqual(api.keys(OTHER), (ALTERNATIVE,))
        self.assertEqual(api.keys(context), (ALTERNATIVE,))
        self.assertEqual(set(api.keys(shortcut.ACTION)), {shortcut.KEY, ALTERNATIVE})
        self.assertEqual(api.conflicts(), (shortcut.ACTION,))
        self.assertEqual(len(json.loads((self.backups()[0] / "actions.json").read_text())), 3)

    def test_repeated_install_is_idempotent(self):
        api = FakeKde()
        shortcut.configure(api)
        shortcut.configure(api)
        self.assertEqual(len(self.backups()), 1)
        self.assertEqual(api.keys(shortcut.ACTION), (shortcut.KEY,))

    def test_failure_rolls_back_partially_changed_conflicts_and_new_launcher(self):
        api = FakeKde({OTHER: (shortcut.KEY, ALTERNATIVE)})
        original = dict(api.assignments)
        api.failure = OTHER
        with self.assertRaisesRegex(RuntimeError, "prior bindings restored"):
            shortcut.configure(api)
        self.assertEqual(api.assignments, original)
        self.assertEqual(len(self.backups()), 1)

    def test_failure_restores_previous_calendar_keys(self):
        api = FakeKde({OTHER: (shortcut.KEY,), shortcut.ACTION: (ALTERNATIVE,)})
        original = dict(api.assignments)
        api.failure = shortcut.ACTION
        with self.assertRaisesRegex(RuntimeError, "prior bindings restored"):
            shortcut.configure(api)
        self.assertEqual(api.assignments, original)

    def test_rollback_failure_reports_error_and_retains_backup(self):
        api = FakeKde({OTHER: (shortcut.KEY,)})
        setter = api.set_keys

        def fail_restore(action, keys, present=False):
            if action == OTHER and shortcut.KEY in keys:
                raise RuntimeError("Synthetic restore failure")
            setter(action, keys, present)

        with mock.patch.object(api, "set_keys", side_effect=fail_restore):
            with mock.patch.object(shortcut, "wait_for_saved_binding",
                                   side_effect=RuntimeError("Synthetic save failure")):
                with self.assertRaisesRegex(RuntimeError, "rollback errors: Synthetic restore failure"):
                    shortcut.configure(api)
        self.assertTrue((self.backups()[0] / "actions.json").is_file())

    def test_backup_failure_changes_no_bindings(self):
        api = FakeKde({OTHER: (shortcut.KEY,)})
        original = dict(api.assignments)
        with mock.patch.object(shortcut, "backup_settings", side_effect=OSError("Full disk")):
            with self.assertRaisesRegex(OSError, "Full disk"):
                shortcut.configure(api)
        self.assertEqual(api.assignments, original)

    def test_persistence_failure_rolls_back_instead_of_claiming_success(self):
        api = FakeKde({OTHER: (shortcut.KEY,)})
        original = dict(api.assignments)
        with mock.patch.object(shortcut, "wait_for_saved_binding",
                               side_effect=RuntimeError("KDE did not save")):
            with self.assertRaisesRegex(RuntimeError, "KDE did not save"):
                shortcut.configure(api)
        self.assertEqual(api.assignments, original)

    def test_saved_binding_accepts_calendar_group_not_another_app(self):
        self.saved.stop()
        self.settings.write_text(
            "[fixture]\n_launch=Meta+Alt+C\n[services][smpl-calendar.desktop]\n"
            "_launch=Ctrl+Alt+L\\tMeta+Alt+C\n")
        shortcut.wait_for_saved_binding()
        self.settings.write_text("[fixture]\n_launch=Meta+Alt+C\n")
        with mock.patch.object(shortcut.time, "monotonic", side_effect=[0, 6]):
            with self.assertRaisesRegex(RuntimeError, "did not save"):
                shortcut.wait_for_saved_binding()


if __name__ == "__main__":
    unittest.main()

#!/usr/bin/python3
"""Assign the calendar's KDE shortcut using KGlobalAccel, not config rewrites."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time


ACTION = ("smpl-calendar.desktop", "_launch", "smpl Calendar", "smpl Calendar")
# Qt::MetaModifier | Qt::AltModifier | Qt::Key_C.
KEY = (0x10000000 | 0x08000000 | ord("C"),)
LABEL = "Meta+Alt+C"


class KdeShortcuts:
    def __init__(self):
        import dbus

        self.dbus = dbus
        try:
            self.bus = dbus.SessionBus()
            if not self.bus.name_has_owner("org.kde.kglobalaccel"):
                raise RuntimeError("KDE's global shortcut service is not running in this session")
            self.interface = dbus.Interface(
                self.bus.get_object("org.kde.kglobalaccel", "/kglobalaccel"),
                "org.kde.KGlobalAccel",
            )
        except dbus.exceptions.DBusException as error:
            raise RuntimeError(f"Could not connect to KDE's shortcut service: {error}") from error

    def call(self, method, *args, interface=None):
        try:
            return getattr(interface or self.interface, method)(*args, timeout=10)
        except self.dbus.exceptions.DBusException as error:
            raise RuntimeError(f"KDE shortcut operation {method} failed: {error}") from error

    def action(self, action):
        return self.dbus.Array(action, signature="s")

    def sequence(self, sequence):
        if not 1 <= len(sequence) <= 4:
            raise ValueError("KDE key sequences must contain one to four key codes")
        # Older KDE readers unconditionally read four integers, even for one key.
        padded = tuple(sequence) + (0,) * (4 - len(sequence))
        return self.dbus.Struct((self.dbus.Array(padded, signature="i"),), signature="ai")

    def sequences(self, sequences):
        return self.dbus.Array([self.sequence(key) for key in sequences], signature="(ai)")

    def registered(self, action):
        return any(tuple(map(str, item[:2])) == action[:2] for item in
                   self.call("allActionsForComponent", self.action(action)))

    def keys(self, action):
        # Qt pads a QKeySequence to four integers on D-Bus.
        return tuple(sorted(tuple(int(key) for key in item[0] if key) for item in
                            self.call("shortcutKeys", self.action(action)) if any(item[0])))

    def conflicts(self):
        actions = {}
        # globalShortcutsByKey can return only the first conflicting component.
        for path in self.call("allComponents"):
            interface = self.dbus.Interface(
                self.bus.get_object("org.kde.kglobalaccel", path, introspect=False),
                "org.kde.kglobalaccel.Component",
            )
            for context in self.call("getShortcutContexts", interface=interface):
                for info in self.call("allShortcutInfos", context, interface=interface):
                    unique, friendly, component, component_name = map(str, info[:4])
                    if context and context != "default":
                        component += f"|{context}"
                    action = (component, unique, component_name, friendly)
                    if any(KEY[0] in key for key in self.keys(action)):
                        actions[action[:2]] = action
        return tuple(actions.values())

    def register(self, action):
        self.call("doRegister", self.action(action))

    def set_keys(self, action, keys, present=False):
        if present:
            # SetPresent | NoAutoloading: activate the launcher and apply explicit keys.
            result = self.call("setShortcutKeys", self.action(action),
                               self.sequences(keys), self.dbus.UInt32(6))
            if {tuple(int(key) for key in item[0] if key) for item in result} != set(keys):
                raise RuntimeError(f"KDE rejected shortcuts for {action[0]} / {action[1]}")
        # Notify running owners too, so a displaced binding stops working immediately.
        self.call("setForeignShortcutKeys", self.action(action), self.sequences(keys))
        if set(self.keys(action)) != set(keys):
            raise RuntimeError(f"KDE shortcut readback differs for {action[0]} / {action[1]}")

    def unregister(self, action):
        if not self.call("unregister", action[0], action[1]):
            raise RuntimeError("Could not remove a partially registered calendar shortcut")


def backup_settings(actions):
    config = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config")
    state = Path(os.environ.get("XDG_STATE_HOME") or Path.home() / ".local/state")
    if not config.is_absolute() or not state.is_absolute():
        raise RuntimeError("KDE config/state directories must be absolute")
    base = state / "smplos/calendar/backups"
    base.mkdir(parents=True, exist_ok=True, mode=0o700)
    directory = Path(tempfile.mkdtemp(prefix="kde-shortcut.", dir=base))
    snapshots = {}
    for source in (config / "kglobalshortcutsrc", state / "kglobalshortcutsstaterc"):
        if source.exists() or source.is_symlink():
            if not source.is_file() or source.stat().st_uid != os.getuid():
                raise RuntimeError(f"Cannot back up unowned/nonregular KDE settings: {source}")
            snapshots[source.name] = source.read_bytes()
    snapshots["actions.json"] = json.dumps([
        {"action": action, "keys": keys, "registered": registered}
        for action, keys, registered in actions
    ], indent=2).encode()
    for name, content in snapshots.items():
        target = directory / name
        with target.open("xb") as stream:
            os.chmod(target, 0o600)
            stream.write(content)
            stream.flush()
            os.fsync(stream.fileno())
        if target.read_bytes() != content:
            raise RuntimeError(f"KDE shortcut backup verification failed: {target}")
    descriptor = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
    return directory


def wait_for_saved_binding():
    config = Path(os.environ.get("XDG_CONFIG_HOME") or Path.home() / ".config")
    path = config / "kglobalshortcutsrc"
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if path.exists():
            in_group = False
            for line in path.read_text().splitlines():
                if line.startswith("["):
                    in_group = line == "[services][smpl-calendar.desktop]"
                elif in_group and line.startswith("_launch="):
                    if LABEL in line.partition("=")[2].replace(r"\t", "\t").split("\t"):
                        return
        time.sleep(0.1)
    raise RuntimeError("KDE did not save the calendar shortcut to kglobalshortcutsrc")


def configure(api):
    conflicts = [action for action in api.conflicts() if action[:2] != ACTION[:2]]
    registered = api.registered(ACTION)
    original = api.keys(ACTION) if registered else ()
    if KEY in original and not conflicts:
        wait_for_saved_binding()
        print(f"{LABEL} already opens smpl Calendar; no changes needed")
        return
    snapshots = [(ACTION, original, registered)]
    snapshots.extend((action, api.keys(action), True) for action in conflicts)
    directory = backup_settings(snapshots)
    print(f"Verified KDE shortcut backup: {directory}", flush=True)
    changed = []
    target_changed = False
    try:
        if not registered:
            target_changed = True
            api.register(ACTION)
        for action, keys, _ in snapshots[1:]:
            retained = tuple(key for key in keys if KEY[0] not in key)
            print(f"Reassigning {LABEL} from {action[2]} / {action[3]}", flush=True)
            changed.append((action, keys))
            api.set_keys(action, retained)
        desired = tuple(key for key in original if KEY[0] not in key) + (KEY,)
        target_changed = True
        api.set_keys(ACTION, desired, present=True)
        if any(action[:2] != ACTION[:2] for action in api.conflicts()):
            raise RuntimeError("Another action still conflicts with the calendar shortcut")
        wait_for_saved_binding()
    except (RuntimeError, OSError, ValueError) as error:
        failures = []
        if target_changed:
            try:
                if registered:
                    api.set_keys(ACTION, original, present=True)
                elif api.registered(ACTION):
                    api.unregister(ACTION)
            except RuntimeError as rollback_error:
                failures.append(str(rollback_error))
        for action, keys in reversed(changed):
            try:
                api.set_keys(action, keys)
            except RuntimeError as rollback_error:
                failures.append(str(rollback_error))
        detail = "; rollback errors: " + "; ".join(failures) if failures else "; prior bindings restored"
        raise RuntimeError(f"{error}{detail}. Recovery backup: {directory}") from error
    print(f"{LABEL} now opens smpl Calendar (Windows/Super + Alt + C)")


def main():
    if len(sys.argv) != 1:
        raise RuntimeError("Usage: smpl-calendar-kde-shortcut (reassigns Meta+Alt+C)")
    if os.geteuid() == 0:
        raise RuntimeError("Configure KDE shortcuts as the desktop user, not root")
    os.umask(0o077)
    api = KdeShortcuts()
    builder = shutil.which("kbuildsycoca6")
    if builder is None:
        raise RuntimeError("kbuildsycoca6 is missing; configure the shortcut in KDE System Settings")
    subprocess.run([builder], check=True, timeout=30)
    configure(api)


if __name__ == "__main__":
    try:
        main()
    except (ImportError, RuntimeError, OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"smpl-calendar: KDE shortcut setup failed: {error}", file=sys.stderr)
        sys.exit(1)

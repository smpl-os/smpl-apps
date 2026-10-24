#!/usr/bin/env python3
"""Test the installed shortcut helper on a private KDE daemon, bus and display."""

import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "kde_shortcut", ROOT / "calendar/packaging/setup-kde-shortcut.py")
shortcut = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shortcut)
OTHER = ("shortcut-fixture.desktop", "_launch", "Shortcut fixture", "Shortcut fixture")
CONTEXT = ("calendar-shortcut-fixture|editor", "shadow", "Shortcut fixture", "Multi-key action")
ALTERNATIVE = (0x0C00004C,)


def wait_until(predicate, description):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.1)
    raise RuntimeError(f"Timed out waiting for {description}")


def main():
    import dbus

    bus = dbus.SessionBus()
    if bus.name_has_owner("org.kde.kglobalaccel"):
        raise RuntimeError("Use a private dbus-run-session; never test on a running desktop")
    if not os.environ.get("DISPLAY"):
        raise RuntimeError("Use a private xvfb-run display")
    helper = shutil.which("smpl-calendar-kde-shortcut")
    if helper is None:
        raise RuntimeError("Install the calendar RPM before running this test")
    with tempfile.TemporaryDirectory(prefix="calendar-kde-") as temporary:
        home = Path(temporary)
        os.environ.update({
            "HOME": str(home),
            "XDG_CONFIG_HOME": str(home / ".config"),
            "XDG_DATA_HOME": str(home / ".local/share"),
            "XDG_STATE_HOME": str(home / ".local/state"),
            "XDG_CACHE_HOME": str(home / ".cache"),
            "XDG_RUNTIME_DIR": str(home / "runtime"),
            "QT_QPA_PLATFORM": "xcb",
            "KGLOBALACCELD_PLATFORM": "xcb",
        })
        Path(os.environ["XDG_RUNTIME_DIR"]).mkdir(mode=0o700)
        applications = Path(os.environ["XDG_DATA_HOME"]) / "applications"
        applications.mkdir(parents=True)
        marker = home / "calendar-launched"
        launcher = home / "launch.py"
        launcher.write_text(f"from pathlib import Path\nPath({str(marker)!r}).touch()\n")
        for name in ("smpl-calendar.desktop", "shortcut-fixture.desktop"):
            (applications / name).write_text(
                "[Desktop Entry]\nType=Application\nName=Calendar shortcut fixture\n"
                f"Exec=/usr/bin/python3 {launcher}\nTerminal=false\n")
        subprocess.run(["kbuildsycoca6", "--noincremental"], check=True)
        daemon = None
        with (home / "daemon.log").open("w+") as log:
            try:
                daemon = subprocess.Popen(["/usr/libexec/kglobalacceld"], stdout=log, stderr=log)
                wait_until(lambda: bus.name_has_owner("org.kde.kglobalaccel"), "KDE shortcut daemon")
                api = shortcut.KdeShortcuts()
                api.register(OTHER)
                api.set_keys(OTHER, (shortcut.KEY, ALTERNATIVE), present=True)
                anchor = ("calendar-shortcut-fixture", "anchor", "Shortcut fixture", "Default anchor")
                api.register(anchor)
                api.set_keys(anchor, (), present=True)
                api.register(CONTEXT)
                api.set_keys(CONTEXT, (shortcut.KEY, (ord("D"), shortcut.KEY[0]), ALTERNATIVE),
                             present=True)
                subprocess.run([helper], check=True, timeout=30)
                assert api.keys(OTHER) == (ALTERNATIVE,), api.keys(OTHER)
                assert api.keys(CONTEXT) == (ALTERNATIVE,), api.keys(CONTEXT)
                assert api.keys(shortcut.ACTION) == (shortcut.KEY,)
                snapshots = list((home / ".local/state/smplos/calendar/backups").glob("kde-shortcut.*"))
                assert len(snapshots) == 1, snapshots
                subprocess.run([helper], check=True, timeout=30)
                assert len(list(snapshots[0].parent.iterdir())) == 1
                subprocess.run(["xdotool", "key", "--clearmodifiers", "super+alt+c"], check=True)
                wait_until(marker.is_file, "calendar launch from the keyboard shortcut")
                daemon.terminate()
                daemon.wait(timeout=10)
                wait_until(lambda: not bus.name_has_owner("org.kde.kglobalaccel"), "KDE daemon shutdown")
                daemon = subprocess.Popen(["/usr/libexec/kglobalacceld"], stdout=log, stderr=log)
                wait_until(lambda: bus.name_has_owner("org.kde.kglobalaccel"), "restarted KDE daemon")
                api = shortcut.KdeShortcuts()
                assert api.keys(OTHER) == (ALTERNATIVE,)
                assert api.keys(CONTEXT) == (ALTERNATIVE,)
                assert api.keys(shortcut.ACTION) == (shortcut.KEY,)
                marker.unlink()
                subprocess.run(["xdotool", "key", "--clearmodifiers", "super+alt+c"], check=True)
                wait_until(marker.is_file, "calendar shortcut after daemon restart")
            finally:
                if daemon is not None and daemon.poll() is None:
                    daemon.terminate()
                    daemon.wait(timeout=10)
                log.seek(0)
                print(log.read())
    print("Installed KDE shortcut: conflict reassigned, alternatives retained, key launch and persistence verified")


if __name__ == "__main__":
    main()

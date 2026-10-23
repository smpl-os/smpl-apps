#!/usr/bin/env python3
"""Verify the installed daemon's real libnotify/D-Bus traffic on a private bus."""

import os
import json
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import time
from contextlib import closing

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib


INTERFACE = "org.freedesktop.Notifications"


class Notifications(dbus.service.Object):
    def __init__(self, bus):
        self.name = dbus.service.BusName(INTERFACE, bus=bus, do_not_queue=True)
        super().__init__(self.name, "/org/freedesktop/Notifications")
        self.calls = []
        self.reject = True

    @dbus.service.method(INTERFACE, in_signature="", out_signature="as")
    def GetCapabilities(self):
        return ["body", "persistence"]

    @dbus.service.method(INTERFACE, in_signature="", out_signature="ssss")
    def GetServerInformation(self):
        return ("Calendar fixture", "smplOS", "1", "1.2")

    @dbus.service.method(INTERFACE, in_signature="susssasa{sv}i", out_signature="u")
    def Notify(self, app, replaces, icon, summary, body, actions, hints, timeout):
        self.calls.append((app, replaces, icon, summary, body, actions, hints, timeout))
        if self.reject:
            raise dbus.exceptions.DBusException(
                "Fixture delivery rejection", name="org.freedesktop.DBus.Error.Failed")
        return len(self.calls)


def pump_until(predicate, timeout=10):
    context = GLib.MainContext.default()
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        while context.pending():
            context.iteration(False)
        if predicate():
            return
        time.sleep(0.02)
    raise RuntimeError("Timed out waiting for the reminder notification fixture")


def stop(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def main():
    if os.geteuid() == 0:
        raise RuntimeError("Run the notification fixture as an unprivileged user")
    DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus()
    # Do not replace any running notification server, even on this fixture bus.
    if bus.name_has_owner(INTERFACE):
        raise RuntimeError("Notification fixture requires a fresh private D-Bus session")
    server = Notifications(bus)
    with tempfile.TemporaryDirectory(prefix="smpl-calendar-notifications-") as home:
        env = os.environ.copy()
        for key in ("DISPLAY", "WAYLAND_DISPLAY", "WAYLAND_SOCKET", "HYPRLAND_INSTANCE_SIGNATURE"):
            env.pop(key, None)
        runtime = Path(home) / "runtime"
        runtime.mkdir(mode=0o700)
        env.update(HOME=home, XDG_RUNTIME_DIR=str(runtime),
                   XDG_CACHE_HOME=home + "/.cache", XDG_CONFIG_HOME=home + "/.config",
                   XDG_DATA_HOME=home + "/.local/share", XDG_STATE_HOME=home + "/.local/state")
        database = Path(home) / ".local/share/smplos/calendar/events.db"
        database.parent.mkdir(parents=True)
        helper = Path(home) / "bin/systemctl"
        helper.parent.mkdir()
        helper.write_text("""#!/usr/bin/python3
import json
import os
from pathlib import Path
import sys
with Path(os.environ["SYSTEMCTL_CALLS"]).open("a") as output:
    output.write(json.dumps(sys.argv[1:]) + "\\n")
if "start" in sys.argv and os.environ.get("FIXTURE_MASKED") == "1":
    print("Fixture: service is masked", file=sys.stderr)
    sys.exit(1)
if "show" in sys.argv:
    print("LoadState=loaded\\nActiveState=active")
""")
        helper.chmod(0o755)
        calls = Path(home) / "systemctl-calls.jsonl"
        startup_env = dict(env, PATH=f"{helper.parent}:{env['PATH']}", SYSTEMCTL_CALLS=str(calls))
        subprocess.run(["/usr/bin/smpl-calendar-alertd", "--autostart"],
                       env=startup_env, check=True)
        captured = [json.loads(line) for line in calls.read_text().splitlines()]
        if (len(captured) != 2
                or captured[0] != ["--user", "start", "smpl-calendar-alertd.service"]
                or captured[1][:3] != ["--user", "--no-pager", "show"]):
            raise RuntimeError(f"Autostart did not start/check the managed service: {captured}")
        rejected = subprocess.run(
            ["/usr/bin/smpl-calendar-alertd", "--autostart"],
            env=dict(startup_env, FIXTURE_MASKED="1"),
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        if rejected.returncode == 0 or "could not start" not in rejected.stderr:
            raise RuntimeError("Autostart ignored a masked/failed service")
        if database.exists():
            raise RuntimeError("Autostart entered unmanaged database runtime")
        with closing(sqlite3.connect(database)) as connection:
            connection.executescript("""
                CREATE TABLE events (
                    id INTEGER PRIMARY KEY, title TEXT NOT NULL, description TEXT DEFAULT '',
                    start_ts INTEGER NOT NULL, end_ts INTEGER NOT NULL, all_day INTEGER DEFAULT 0,
                    recurrence TEXT DEFAULT 'none', recurrence_end INTEGER, color TEXT,
                    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                    alert_minutes INTEGER DEFAULT 0
                );
            """)
            now = int(time.time())
            for event_id, start in [(1, now + 890), (2, now + 1800)]:
                connection.execute(
                    "INSERT INTO events (id,title,start_ts,end_ts,created_at,updated_at,alert_minutes)"
                    " VALUES (?,?,?,?,?,?,15)",
                    (event_id, f"RPM reminder {event_id}", start, start + 1200, now, now),
                )
            connection.commit()

            def receipts():
                return connection.execute("SELECT COUNT(*) FROM sent_alerts").fetchone()[0]

            for attempt in ("rejected", "delivered", "restarted"):
                previous_calls = len(server.calls)
                with tempfile.TemporaryFile(mode="w+") as log:
                    process = subprocess.Popen(
                        ["/usr/bin/smpl-calendar-alertd", "--foreground", "--persistent"],
                        env=env, stdout=log, stderr=log,
                    )
                    try:
                        if attempt == "rejected":
                            pump_until(lambda: len(server.calls) > previous_calls)

                            def rejected():
                                log.seek(0)
                                return "delivery failed" in log.read()

                            pump_until(rejected)
                            if receipts() != 0:
                                raise RuntimeError("Rejected notification was marked as delivered")
                            server.reject = False
                        elif attempt == "delivered":
                            pump_until(lambda: len(server.calls) > previous_calls and receipts() == 1)
                        else:
                            # The first poll is immediate. Wait long enough for startup,
                            # then check no second Notify call or early event receipt.
                            pump_until(lambda: (runtime / "smplos/calendar-alertd.lock").exists())
                            deadline = time.monotonic() + 2
                            pump_until(lambda: time.monotonic() >= deadline)
                            if len(server.calls) != previous_calls or receipts() != 1:
                                raise RuntimeError("Restart duplicated a reminder or fired an early one")
                        if process.poll() is not None:
                            log.seek(0)
                            raise RuntimeError(f"Reminder daemon exited:\n{log.read()}")
                    finally:
                        stop(process)
            if len(server.calls) != 2:
                raise RuntimeError(f"Unexpected notification count: {len(server.calls)}")
            for app, replaces, icon, summary, body, actions, hints, timeout in server.calls:
                if (app != "smpl-calendar" or replaces != 0
                        or (icon != "x-office-calendar" and hints.get("image-path") != "x-office-calendar")
                        or summary != "RPM reminder 1" or not body.startswith("Starts at ")
                        or actions or timeout != 0 or hints.get("urgency") != 2
                        or not hints.get("resident") or hints.get("transient") != False
                        or hints.get("desktop-entry") != "smpl-calendar"):
                    raise RuntimeError(f"Persistent notification contract mismatch: {hints}, {timeout}")
    print("Reminder delivery verified: managed autostart, masks, persistent D-Bus hints, retry and deduplication; no GUI")


if __name__ == "__main__":
    main()

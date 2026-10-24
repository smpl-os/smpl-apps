#!/usr/bin/env python3
"""Exercise the installed Fedora RPM on a private Xvfb, with disposable data."""

import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time
from contextlib import closing


def main():
    version = sys.argv[1]
    if os.geteuid() == 0:
        raise RuntimeError("Run the calendar smoke test as an unprivileged user")
    with tempfile.TemporaryDirectory(prefix="smpl-calendar-rpm-") as home:
        env = os.environ.copy()
        for key in ("WAYLAND_DISPLAY", "WAYLAND_SOCKET", "HYPRLAND_INSTANCE_SIGNATURE",
                    "DBUS_SESSION_BUS_ADDRESS", "SLINT_BACKEND", "SLINT_RENDERER"):
            env.pop(key, None)
        runtime = Path(home) / "runtime"
        runtime.mkdir(mode=0o700)
        env.update(HOME=home, XDG_CONFIG_HOME=home + "/.config",
                   XDG_DATA_HOME=home + "/.local/share", XDG_CACHE_HOME=home + "/.cache",
                   XDG_STATE_HOME=home + "/.local/state", XDG_RUNTIME_DIR=str(runtime),
                   LIBGL_ALWAYS_SOFTWARE="1")
        for name in ("smpl-calendar", "smpl-calendar-alertd"):
            output = subprocess.check_output([f"/usr/bin/{name}", "--version"],
                                             env=env, text=True).strip()
            expected = f"{name} v{version}" if name == "smpl-calendar" else f"{name} {version}"
            if output != expected:
                raise RuntimeError(f"Installed version mismatch: {output}")
        subprocess.run(["/usr/bin/smpl-calendar-alertd", "--check"], env=env, check=True)
        subprocess.run(["/usr/bin/smpl-calendar", "--font-license"], env=env, check=True,
                       stdout=subprocess.DEVNULL)
        database = Path(home) / ".local/share/smplos/calendar/events.db"
        if database.exists():
            raise RuntimeError("Read-only command unexpectedly opened the events database")
        for args in (["--details"], []):
            with tempfile.TemporaryFile(mode="w+") as log:
                process = subprocess.Popen(["/usr/bin/smpl-calendar", *args], env=env,
                                           stdout=log, stderr=log)
                try:
                    deadline = time.monotonic() + 20
                    while time.monotonic() < deadline:
                        if process.poll() is not None:
                            log.seek(0)
                            raise RuntimeError(f"Calendar exited before mapping:\n{log.read()}")
                        mapped = subprocess.run(
                            ["xdotool", "search", "--onlyvisible", "--pid", str(process.pid)],
                            env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                            timeout=3,
                        )
                        if mapped.returncode == 0 and database.is_file():
                            break
                        time.sleep(0.1)
                    else:
                        log.seek(0)
                        raise RuntimeError(f"Calendar window did not map:\n{log.read()}")
                    time.sleep(1)
                    if process.poll() is not None:
                        log.seek(0)
                        raise RuntimeError(f"Calendar failed after mapping:\n{log.read()}")
                finally:
                    if process.poll() is None:
                        process.terminate()
                        try:
                            process.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            process.kill()
                            process.wait()
        with closing(sqlite3.connect(f"file:{database}?mode=ro", uri=True)) as connection:
            if connection.execute("PRAGMA integrity_check").fetchone() != ("ok",):
                raise RuntimeError("Fixture calendar database failed its integrity check")
    print("Installed RPM: both calendar windows mapped; versions and database verified")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Run the native dashboard fixture on a private authenticated Xvfb, not the desktop."""
import os
import argparse
from pathlib import Path
import secrets
import socket
import struct
import subprocess
import tempfile
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--editor", action="store_true")
    parser.add_argument("--typeahead", action="store_true")
    parser.add_argument("--rings", action="store_true")
    args = parser.parse_args()
    args.editor = args.editor or args.typeahead
    root = Path(__file__).resolve().parents[2]
    binary = root / "target/debug/examples" / ("editor_probe" if args.editor else "dashboard_probe")
    output = root / "target/calendar-probe"
    output.mkdir(parents=True, exist_ok=True, mode=0o700)
    for number in range(220, 270):
        try:
            with socket.socket() as sock:
                sock.bind(("0.0.0.0", 6000 + number))
            break
        except OSError:
            continue
    else:
        raise RuntimeError("No private display available")
    authority = output / f"authority-{os.getpid()}"
    fields = (b"", str(number).encode(), b"MIT-MAGIC-COOKIE-1", secrets.token_bytes(16))
    with authority.open("xb") as stream:
        os.chmod(authority, 0o600)
        stream.write(struct.pack(">H", 65535))
        for field in fields:
            stream.write(struct.pack(">H", len(field)) + field)
    env = os.environ.copy()
    for key in ("WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DBUS_SESSION_BUS_ADDRESS", "HYPRLAND_INSTANCE_SIGNATURE",
                "SLINT_BACKEND", "SLINT_RENDERER"):
        env.pop(key, None)
    env.update(DISPLAY=f"127.0.0.1:{number}", XAUTHORITY=str(authority),
               SMPL_CALENDAR_PRIVATE_DISPLAY="1", SMPL_CALENDAR_PROBE_OUTPUT=str(output),
               LIBGL_ALWAYS_SOFTWARE="1", SLINT_SCALE_FACTOR="1", WINIT_X11_SCALE_FACTOR="1")
    server = None
    try:
        with (output / "xvfb.log").open("w") as log:
            server = subprocess.Popen(
                ["Xvfb", f":{number}", "-screen", "0", "1200x900x24", "-nolock",
                 "-nolisten", "unix", "-nolisten", "local", "-listen", "tcp",
                 "-auth", str(authority), "-noreset"], stdout=log, stderr=log, env=env)
            for _ in range(50):
                if server.poll() is not None:
                    raise RuntimeError("Private Xvfb exited")
                try:
                    with socket.create_connection(("127.0.0.1", 6000 + number), timeout=0.1):
                        break
                except OSError:
                    time.sleep(0.1)
            else:
                raise RuntimeError("Private Xvfb not responsive")
            with tempfile.TemporaryDirectory(prefix="calendar-home-", dir=output) as home:
                env.update(HOME=home, XDG_CONFIG_HOME=home + "/.config",
                           XDG_CACHE_HOME=home + "/.cache", XDG_DATA_HOME=home + "/.local/share")
                Path(home + "/runtime").mkdir(mode=0o700)
                env["XDG_RUNTIME_DIR"] = home + "/runtime"
                subprocess.run([str(binary)] + (["--typeahead"] if args.typeahead else ["--rings"] if args.rings else []),
                               env=env, cwd=root, check=True, timeout=30)
                if args.editor or args.rings:
                    return
                subprocess.run([str(binary), "--runtime"], env=env, cwd=root, check=True, timeout=15)
                subprocess.run([str(binary), "--cache-runtime"], env=env, cwd=root, check=True, timeout=15)
                marker = Path(home) / "navigation-child.pid"
                env["SMPL_CALENDAR_NAV_MARKER"] = str(marker)
                try:
                    subprocess.run([str(binary), "--navigation-runtime"], env=env, cwd=root, check=True, timeout=15)
                    if not marker.is_file():
                        raise RuntimeError("Back did not map its compact child")
                    pid = int(marker.read_text())
                    if Path(f"/proc/{pid}/exe").resolve() != binary.resolve():
                        raise RuntimeError("Unexpected navigation child executable")
                    print("navigation: compact child is running the same binary, saved fixture event intact")
                finally:
                    if marker.is_file():
                        pid = int(marker.read_text())
                        if Path(f"/proc/{pid}/exe").resolve() == binary.resolve():
                            os.kill(pid, 15)
    finally:
        if server is not None and server.poll() is None:
            server.terminate()
            server.wait(timeout=5)
        authority.unlink()


if __name__ == "__main__":
    main()

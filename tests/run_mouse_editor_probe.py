#!/usr/bin/env python3
"""Interact with the actual shared Slint editor on authenticated private Xvfb.

Builds the Settings test binary, never launches Settings on the user's desktop,
captures live input or reads/writes the user's mappings. FemtoVG remains active.
"""
import json
import os
from pathlib import Path
import secrets
import socket
import struct
import subprocess
import tempfile
import time


def main():
    root = Path(__file__).resolve().parents[1]
    build = subprocess.run(
        ["cargo", "test", "-p", "settings", "--bin", "settings", "--no-run", "--message-format=json"],
        cwd=root, check=False, text=True, stdout=subprocess.PIPE,
    )
    if build.returncode:
        for line in build.stdout.splitlines():
            item = json.loads(line)
            if item.get("reason") == "compiler-message":
                print(item["message"].get("rendered", item["message"]["message"]), end="")
        raise SystemExit(build.returncode)
    binaries = [
        item["executable"] for line in build.stdout.splitlines()
        if (item := json.loads(line)).get("reason") == "compiler-artifact"
        and item.get("executable") and item["profile"]["test"]
    ]
    if not binaries:
        raise RuntimeError("Settings test executable was not produced")
    with tempfile.TemporaryDirectory(prefix="smpl-mouse-editor-probe-") as scratch:
        private = Path(scratch)
        for number in range(170, 220):
            try:
                with socket.socket() as sock:
                    sock.bind(("0.0.0.0", 6000 + number))
                break
            except OSError:
                continue
        else:
            raise RuntimeError("No free private Xvfb display")
        fields = (b"", str(number).encode(), b"MIT-MAGIC-COOKIE-1", secrets.token_bytes(16))
        authority = private / "authority"
        authority.write_bytes(struct.pack(">H", 65535) + b"".join(
            struct.pack(">H", len(field)) + field for field in fields))
        authority.chmod(0o600)
        env = os.environ.copy()
        for key in ("WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DBUS_SESSION_BUS_ADDRESS",
                    "SLINT_BACKEND", "SLINT_RENDERER", "SLINT_SCALE_FACTOR", "HYPRLAND_INSTANCE_SIGNATURE"):
            env.pop(key, None)
        home = private / "home"
        home.mkdir()
        (private / "tmp").mkdir()
        env.update(
            DISPLAY=f"127.0.0.1:{number}", XAUTHORITY=str(authority),
            HOME=str(home), XDG_CONFIG_HOME=str(home / ".config"),
            TMPDIR=str(private / "tmp"),
            SMPL_MOUSE_PROBE_PRIVATE_DISPLAY="1", LIBGL_ALWAYS_SOFTWARE="1",
            SLINT_SCALE_FACTOR="1", WINIT_X11_SCALE_FACTOR="1",
        )
        with (private / "xvfb.log").open("w+") as log:
            server = subprocess.Popen(
                ["Xvfb", f":{number}", "-screen", "0", "800x600x24", "-nolock",
                 "-nolisten", "unix", "-nolisten", "local", "-listen", "tcp",
                 "-auth", str(authority), "-noreset"],
                stdout=log, stderr=subprocess.STDOUT, env=env, cwd=root,
            )
            try:
                for _ in range(50):
                    if server.poll() is not None:
                        log.seek(0)
                        raise RuntimeError(log.read())
                    try:
                        with socket.create_connection(("127.0.0.1", 6000 + number), timeout=0.1):
                            break
                    except OSError:
                        time.sleep(0.1)
                else:
                    raise RuntimeError("Private Xvfb did not become responsive")
                for case in ("mouse_editor_native_dropdown_selection_survives_status_polling",
                             "mouse_identify_native_selects_diagram",
                             "mouse_setup_native_visible_workflow"):
                    subprocess.run([
                        "dbus-run-session", "--", binaries[-1],
                        f"mouse::native_tests::{case}",
                        "--exact", "--ignored", "--nocapture", "--test-threads=1",
                    ], cwd=root, env=env, check=True, timeout=90)
            finally:
                server.terminate()
                try:
                    server.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()


if __name__ == "__main__":
    main()

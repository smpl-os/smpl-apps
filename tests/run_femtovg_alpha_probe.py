#!/usr/bin/env python3
"""Run only the GUI fixture on authenticated private Xvfb, never the desktop.

Requires an already-built `cargo build -p smpl-common --example femtovg_alpha_probe`.
No compositor screenshot, application action, user config, or package install occurs.
The server disables Unix sockets and lockfiles to avoid temporary-directory writes.
"""

import os
from pathlib import Path
import secrets
import shutil
import socket
import struct
import subprocess
import time


def main():
    root = Path(__file__).resolve().parents[1]
    binary = root / "target/debug/examples/femtovg_alpha_probe"
    if not binary.is_file():
        raise SystemExit("First run: cargo build -p smpl-common --example femtovg_alpha_probe")
    if not shutil.which("Xvfb"):
        raise SystemExit("Xvfb is required for this optional native test")
    artifacts = root / "target/native-alpha-probe"
    artifacts.mkdir(parents=True, exist_ok=True)
    os.chmod(artifacts, 0o700)
    # Binding chooses an unused TCP display without touching any existing X server.
    for number in range(170, 220):
        try:
            with socket.socket() as sock:
                sock.bind(("0.0.0.0", 6000 + number))
            break
        except OSError:
            continue
    else:
        raise SystemExit("No private Xvfb TCP display available")
    authority = artifacts / f"authority-{os.getpid()}"
    fields = (b"", str(number).encode(), b"MIT-MAGIC-COOKIE-1", secrets.token_bytes(16))
    data = struct.pack(">H", 65535)  # FamilyWild, usable by server and TCP client.
    data += b"".join(struct.pack(">H", len(field)) + field for field in fields)
    with authority.open("xb") as output:
        os.chmod(authority, 0o600)
        output.write(data)
    env = os.environ.copy()
    for key in (
        "WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DBUS_SESSION_BUS_ADDRESS",
        "SLINT_BACKEND", "SLINT_RENDERER", "SLINT_SCALE_FACTOR",
    ):
        env.pop(key, None)
    env.update(
        DISPLAY=f"127.0.0.1:{number}",
        XAUTHORITY=str(authority),
        SMPL_ALPHA_PROBE_PRIVATE_DISPLAY="1",
        LIBGL_ALWAYS_SOFTWARE="1",
        SLINT_SCALE_FACTOR="1",
        WINIT_X11_SCALE_FACTOR="1",
    )
    server = None
    try:
        with (artifacts / "xvfb.log").open("w") as log:
            server = subprocess.Popen(
                ["Xvfb", f":{number}", "-screen", "0", "400x240x24", "-nolock",
                 "-nolisten", "unix", "-nolisten", "local", "-listen", "tcp",
                 "-auth", str(authority), "-noreset"],
                stdout=log, stderr=subprocess.STDOUT, env=env, cwd=root,
            )
            for _ in range(50):
                if server.poll() is not None:
                    raise RuntimeError((artifacts / "xvfb.log").read_text())
                try:
                    with socket.create_connection(("127.0.0.1", 6000 + number), timeout=0.1):
                        break
                except OSError:
                    time.sleep(0.1)
            else:
                raise RuntimeError("Private Xvfb failed to become responsive")
            result = subprocess.run(
                [str(binary)], cwd=root, env=env, text=True,
                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=20,
            )
            (artifacts / "result.log").write_text(result.stdout)
            print(result.stdout, end="")
            if result.returncode:
                raise SystemExit(result.returncode)
    finally:
        if server is not None and server.poll() is None:
            server.terminate()
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait()
        authority.unlink()


if __name__ == "__main__":
    main()

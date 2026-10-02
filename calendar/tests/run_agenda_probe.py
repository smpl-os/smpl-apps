#!/usr/bin/env python3
"""Run the agenda-only fixture on authenticated private Xvfb, never the desktop."""

import os
from pathlib import Path
import secrets
import socket
import struct
import subprocess
import time


def main():
    root = Path(__file__).resolve().parents[2]
    artifacts = root / "target/native-agenda-probe"
    artifacts.mkdir(parents=True, exist_ok=True)
    os.chmod(artifacts, 0o700)
    for number in range(240, 290):
        try:
            with socket.socket() as sock:
                sock.bind(("0.0.0.0", 6000 + number))
            break
        except OSError:
            continue
    else:
        raise RuntimeError("No private Xvfb TCP display available")
    authority = artifacts / f"authority-{os.getpid()}"
    fields = (b"", str(number).encode(), b"MIT-MAGIC-COOKIE-1", secrets.token_bytes(16))
    with authority.open("xb") as output:
        os.chmod(authority, 0o600)
        output.write(struct.pack(">H", 65535))
        output.write(b"".join(struct.pack(">H", len(value)) + value for value in fields))
    env = os.environ.copy()
    for key in ("WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DBUS_SESSION_BUS_ADDRESS",
                "SLINT_BACKEND", "SLINT_RENDERER"):
        env.pop(key, None)
    env.update(DISPLAY=f"127.0.0.1:{number}", XAUTHORITY=str(authority),
               SMPL_AGENDA_PROBE_PRIVATE_DISPLAY="1", LIBGL_ALWAYS_SOFTWARE="1",
               SLINT_SCALE_FACTOR="1", WINIT_X11_SCALE_FACTOR="1")
    server = None
    try:
        with (artifacts / "xvfb.log").open("w") as log:
            server = subprocess.Popen(
                ["Xvfb", f":{number}", "-screen", "0", "800x800x24", "-nolock",
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
                raise RuntimeError("Private Xvfb did not become responsive")
            result = subprocess.run(
                [str(root / "target/debug/examples/agenda_probe")],
                cwd=root, env=env, text=True, stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT, timeout=30,
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

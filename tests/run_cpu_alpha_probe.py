#!/usr/bin/env python3
"""Run only the GUI fixture on authenticated private Xvfb, never the desktop.

Requires an already-built `cargo build -p smpl-common --example cpu_alpha_probe`.
No compositor screenshot, application action, user config, or package install occurs.
The server disables Unix sockets and lockfiles to avoid temporary-directory writes.
"""

import os
import argparse
from pathlib import Path
import secrets
import shutil
import socket
import struct
import subprocess
import statistics
import tempfile
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release", action="store_true")
    parser.add_argument("--samples", type=int, default=5)
    args = parser.parse_args()
    if args.samples < 1:
        parser.error("--samples must be positive")
    root = Path(__file__).resolve().parents[1]
    profile = "release" if args.release else "debug"
    binary = root / f"target/{profile}/examples/cpu_alpha_probe"
    if not binary.is_file():
        raise SystemExit("First build smpl-common --example cpu_alpha_probe in the chosen profile")
    if not shutil.which("Xvfb"):
        raise SystemExit("Xvfb is required for this optional native test")
    artifacts = root / "target/native-cpu-alpha-probe"
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
        LIBGL_DRIVERS_PATH="/nonexistent/smpl-probe/no-dri-drivers",
        __EGL_VENDOR_LIBRARY_FILENAMES="/nonexistent/smpl-probe/no-egl-vendors",
        VK_DRIVER_FILES="/nonexistent/smpl-probe/no-vulkan-drivers",
        SLINT_SCALE_FACTOR="1",
        WINIT_X11_SCALE_FACTOR="1",
    )
    server = None
    try:
        with (artifacts / "xvfb.log").open("w") as log:
            server = subprocess.Popen(
                ["Xvfb", f":{number}", "-screen", "0", "400x240x24", "-nolock",
                 "-nolisten", "unix", "-nolisten", "local", "-listen", "tcp",
                 "-auth", str(authority), "-noreset", "-extension", "GLX"],
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
            with tempfile.TemporaryDirectory(prefix="smpl-cpu-probe-") as private:
                for key, name in (
                    ("HOME", "home"), ("XDG_CONFIG_HOME", "config"),
                    ("XDG_DATA_HOME", "data"), ("XDG_STATE_HOME", "state"),
                    ("XDG_CACHE_HOME", "cache"), ("XDG_RUNTIME_DIR", "runtime"),
                ):
                    path = Path(private) / name
                    path.mkdir(mode=0o700)
                    env[key] = str(path)
                result = subprocess.run(
                    [str(binary)], cwd=root, env=env, text=True,
                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=20,
                )
                (artifacts / "result.log").write_text(result.stdout)
                print(result.stdout, end="")
                if result.returncode:
                    raise SystemExit(result.returncode)
                if "PASS: native CPU ARGB" not in result.stdout:
                    raise RuntimeError("CPU probe exited before completing its native pixel checks")
                timings = []
                for _ in range(args.samples):
                    started = time.monotonic()
                    result = subprocess.run(
                        [str(binary), "--startup-only"], cwd=root, env=env, text=True,
                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=10,
                    )
                    if result.returncode or "first_present_ms=" not in result.stdout:
                        raise RuntimeError(result.stdout)
                    timings.append((time.monotonic() - started) * 1000)
                print(
                    f"CPU fixture process-launch-to-exit after first present ({profile}, "
                    f"{args.samples} warm-cache samples): median={statistics.median(timings):.1f}ms "
                    f"min={min(timings):.1f}ms max={max(timings):.1f}ms; not an all-app launch benchmark"
                )
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

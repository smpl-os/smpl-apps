#!/usr/bin/env python3
"""Verify CPU window startup and ARGB presentation on a private headless Weston.

No host compositor connection, desktop capture, real app data, or GPU driver.
Build smpl-common --example cpu_alpha_probe first. Requires Weston with Pixman.
"""

import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time


def main():
    root = Path(__file__).resolve().parents[1]
    binary = root / "target/debug/examples/cpu_alpha_probe"
    if not binary.is_file() or not shutil.which("weston"):
        raise SystemExit("Build cpu_alpha_probe and install Weston before running this fixture")
    artifacts = root / "target/native-cpu-alpha-probe"
    artifacts.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="smpl-cpu-wayland-") as private:
        env = os.environ.copy()
        for key in ("DISPLAY", "WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DBUS_SESSION_BUS_ADDRESS",
                    "SLINT_BACKEND", "SLINT_RENDERER", "LIBGL_ALWAYS_SOFTWARE"):
            env.pop(key, None)
        for key, name in (
            ("HOME", "home"), ("XDG_RUNTIME_DIR", "runtime"), ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"), ("XDG_STATE_HOME", "state"), ("XDG_CACHE_HOME", "cache"),
        ):
            path = Path(private) / name
            path.mkdir(mode=0o700)
            env[key] = str(path)
        env.update(
            LIBGL_DRIVERS_PATH="/nonexistent/smpl-probe/no-dri-drivers",
            __EGL_VENDOR_LIBRARY_FILENAMES="/nonexistent/smpl-probe/no-egl-vendors",
            VK_DRIVER_FILES="/nonexistent/smpl-probe/no-vulkan-drivers",
            SLINT_SCALE_FACTOR="1", SMPL_ALPHA_PROBE_PRIVATE_DISPLAY="1",
        )
        with (artifacts / "weston.log").open("w+") as log:
            server = subprocess.Popen(
                ["weston", "--backend=headless-backend.so", "--renderer=pixman",
                 "--shell=desktop-shell.so", "--socket=smpl-cpu-probe", "--no-config",
                 "--idle-time=0", "--width=400", "--height=240"],
                env=env, cwd=root, stdout=log, stderr=subprocess.STDOUT,
            )
            try:
                socket = Path(env["XDG_RUNTIME_DIR"]) / "smpl-cpu-probe"
                for _ in range(100):
                    if server.poll() is not None:
                        log.seek(0)
                        raise RuntimeError(log.read())
                    if socket.is_socket():
                        break
                    time.sleep(0.05)
                else:
                    raise RuntimeError("Private Weston never became ready")
                env.update(WAYLAND_DISPLAY="smpl-cpu-probe", WAYLAND_DEBUG="client")
                result = subprocess.run(
                    [str(binary)], env=env, cwd=root, text=True,
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20,
                )
                (artifacts / "wayland-result.log").write_text(result.stdout + result.stderr)
                print(result.stdout, end="")
                if result.returncode or "PASS: native CPU ARGB" not in result.stdout:
                    raise RuntimeError(result.stdout + result.stderr)
                buffers = re.findall(
                    r"wl_shm_pool[@#]\d+\.create_buffer\([^,]+,\s*0,\s*(\d+),\s*(\d+),\s*(\d+),\s*(\d+)\)",
                    result.stderr,
                )
                if not buffers:
                    raise RuntimeError("No actual wl_shm buffer presentation was observed")
                for width, height, stride, format_ in buffers:
                    assert format_ == "0", f"Wayland buffer lost alpha: {format_} (ARGB8888 is 0)"
                    assert int(stride) == int(width) * 4
                assert ("240", "160", "960", "0") in buffers, "Resized ARGB buffer was not presented"
                assert sum(width == "200" and height == "120" for width, height, _, _ in buffers) >= 4
                print(f"PASS: {len(buffers)} actual Wayland ARGB buffers, including resize and window recreation")
            finally:
                if server.poll() is None:
                    server.terminate()
                    try:
                        server.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        server.kill()
                        server.wait()


if __name__ == "__main__":
    main()

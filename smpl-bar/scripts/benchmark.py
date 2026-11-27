#!/usr/bin/env python3
"""Measure smpl-bar without touching an installed bar or the user's desktop."""

import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
MIB = 1024


def read_process(pid):
    status = {}
    for line in Path(f"/proc/{pid}/status").read_text().splitlines():
        key, _, value = line.partition(":")
        if key in {"VmRSS", "VmHWM", "Threads"}:
            status[key] = int(value.split()[0])
    sample = {"pid": pid, **status}
    rollup = Path(f"/proc/{pid}/smaps_rollup")
    if rollup.exists():
        for line in rollup.read_text().splitlines():
            if line.startswith("Pss:"):
                sample["Pss"] = int(line.split()[1])
                break
    return sample


def find_eww():
    found = []
    for directory in Path("/proc").iterdir():
        if directory.name.isdigit():
            try:
                if (directory / "comm").read_text().strip() == "eww":
                    found.append(int(directory.name))
            except OSError:
                continue
    return found


def installed_baseline(pid):
    pids = [pid] if pid else find_eww()
    if not pids:
        raise SystemExit("No EWW process found; provide --eww-pid")
    sample = read_process(pids[0])
    exe = Path(f"/proc/{pids[0]}/exe").resolve()
    sample.update(
        {
            "binary_bytes": exe.stat().st_size,
            "executable": str(exe),
            "rss_kib": sample.pop("VmRSS"),
            "hwm_kib": sample.pop("VmHWM"),
            "threads": sample.pop("Threads"),
            "pss_kib": sample.pop("Pss", 0),
        }
    )
    return sample


def telemetry(path):
    if not path.exists():
        return []
    return [
        json.loads(line)
        for line in path.read_text(errors="replace").splitlines()
        if line.startswith("{")
    ]


def terminate_exact(process):
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            os.kill(process.pid, signal.SIGKILL)
            process.wait(timeout=3)


def private_run(binary, duration, output_dir, hidden, popup, recreate):
    weston = shutil.which("weston")
    if not weston:
        raise SystemExit("weston is required for the private headless measurement")
    dbus_daemon = shutil.which("dbus-daemon")
    if not dbus_daemon:
        raise SystemExit("dbus-daemon is required for the private provider fixture")
    runtime_base = Path(os.environ.get("XDG_RUNTIME_DIR", ROOT / "target"))
    runtime = (runtime_base / f"smpl-bar-bench-{os.getpid()}").resolve()
    runtime.mkdir(parents=True, exist_ok=True, mode=0o700)
    runtime.chmod(0o700)
    telemetry_path = output_dir / "smpl-bar-telemetry.jsonl"
    telemetry_path.unlink(missing_ok=True)
    bar_log_path = output_dir / "smpl-bar.log"
    bar_log_path.unlink(missing_ok=True)
    socket = "smpl-bar-benchmark"
    weston_log = (output_dir / "weston.log").open("wb")
    dbus_log = (output_dir / "dbus.log").open("wb")
    environment = os.environ.copy()
    home = output_dir / "home"
    config_home = output_dir / "config"
    data_home = output_dir / "data"
    state_home = output_dir / "state"
    cache_home = output_dir / "cache"
    for directory in (home, config_home, data_home, state_home, cache_home):
        directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    theme_dir = config_home / "eww"
    theme_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
    (theme_dir / "theme-colors.scss").write_text(
        "$theme-bg: #1e1e2e;\n"
        "$theme-fg: #cdd6f4;\n"
        "$theme-accent: #89b4fa;\n"
        "$theme-app-background-opacity: 0.7;\n"
        "$theme-popup-opacity: 0.65;\n"
    )
    environment.update(
        {
            "XDG_RUNTIME_DIR": str(runtime),
            "WAYLAND_DISPLAY": socket,
            "HOME": str(home),
            "XDG_CONFIG_HOME": str(config_home),
            "XDG_DATA_HOME": str(data_home),
            "XDG_STATE_HOME": str(state_home),
            "XDG_CACHE_HOME": str(cache_home),
            "SLINT_SCALE_FACTOR": "1",
        }
    )
    for key in (
        "DISPLAY",
        "HYPRLAND_INSTANCE_SIGNATURE",
        "NIRI_SOCKET",
        "DBUS_SESSION_BUS_ADDRESS",
        "SLINT_BACKEND",
        "SLINT_RENDERER",
    ):
        environment.pop(key, None)
    bus_socket = runtime / "bus"
    bus_address = f"unix:path={bus_socket}"
    bus = subprocess.Popen(
        [
            dbus_daemon,
            "--session",
            "--nofork",
            "--nopidfile",
            f"--address={bus_address}",
        ],
        env=environment,
        stdout=dbus_log,
        stderr=subprocess.STDOUT,
    )
    environment["DBUS_SESSION_BUS_ADDRESS"] = bus_address
    environment["SMPL_BAR_PRIVATE"] = "1"
    headless = subprocess.Popen(
        [
            weston,
            "--backend=headless-backend.so",
            "--renderer=pixman",
            "--shell=desktop-shell.so",
            f"--socket={socket}",
            "--idle-time=0",
            "--width=1920",
            "--height=1080",
            "--no-config",
        ],
        env=environment,
        stdout=weston_log,
        stderr=subprocess.STDOUT,
    )
    app = None
    try:
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if (runtime / socket).exists() and bus_socket.exists():
                break
            if headless.poll() is not None:
                raise RuntimeError("private Weston exited before creating its socket")
            if bus.poll() is not None:
                raise RuntimeError("private D-Bus exited before creating its socket")
            time.sleep(0.025)
        else:
            raise RuntimeError("private Weston socket timeout")
        environment.update(
            {
                "SMPL_BAR_TELEMETRY": "1",
                "SMPL_BAR_TELEMETRY_FILE": str(telemetry_path),
            }
        )
        arguments = [str(binary), "--resident"]
        if hidden:
            arguments = [str(binary), "--resident", "--hidden"]
        with bar_log_path.open("wb") as bar_log:
            app = subprocess.Popen(
                arguments,
                env=environment,
                stdout=bar_log,
                stderr=subprocess.STDOUT,
            )
        deadline = time.monotonic() + duration
        popup_deadline = time.monotonic() + min(1.0, duration / 3)
        popup_sent = False
        while time.monotonic() < deadline:
            if app.poll() is not None:
                raise RuntimeError(
                    f"smpl-bar exited with {app.returncode}: "
                    + bar_log_path.read_text(errors="replace").strip()
                )
            if popup and not popup_sent and time.monotonic() >= popup_deadline:
                control_environment = environment.copy()
                control_environment.pop("SMPL_BAR_TELEMETRY", None)
                control_environment.pop("SMPL_BAR_TELEMETRY_FILE", None)
                subprocess.run(
                    [str(binary), "popup", popup],
                    env=control_environment,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    timeout=2,
                    check=False,
                )
                popup_sent = True
            time.sleep(0.05)
        control_environment = environment.copy()
        control_environment.pop("SMPL_BAR_TELEMETRY", None)
        control_environment.pop("SMPL_BAR_TELEMETRY_FILE", None)
        if recreate:
            for action, pause in (("hide", 0.15), ("show", 0.9)):
                subprocess.run(
                    [str(binary), action],
                    env=control_environment,
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                    timeout=3,
                    check=False,
                )
                time.sleep(pause)
        status_result = subprocess.run(
            [str(binary), "status"],
            env=control_environment,
            capture_output=True,
            text=True,
            timeout=3,
            check=False,
        )
        try:
            status = json.loads(status_result.stdout)
        except (json.JSONDecodeError, TypeError):
            status = {}
        sample = read_process(app.pid)
        events = telemetry(telemetry_path)
        startup = next(
            (event["elapsed_us"] for event in events if event["event"] == "first_map_requested"),
            None,
        )
        paint = next(
            (event["elapsed_us"] for event in events if event["event"] == "first_paint"),
            None,
        )
        alpha_event = next(
            (event for event in events if event["event"] == "first_paint"), None
        )
        sample.update(
            {
                "binary_bytes": Path(binary).stat().st_size,
                "rss_kib": sample.pop("VmRSS"),
                "hwm_kib": sample.pop("VmHWM"),
                "threads": sample.pop("Threads"),
                "pss_kib": sample.pop("Pss", 0),
                "map_us": startup,
                "first_paint_us": paint,
                "popup_first_paint_us": next(
                    (
                        event["duration_us"]
                        for event in events
                        if event["event"] == "popup_first_paint"
                    ),
                    None,
                ),
                "app_id": status.get("app_id"),
                "renderer_ready_count": sum(
                    event["event"] == "renderer_ready" for event in events
                ),
                "renderer_teardown_count": sum(
                    event["event"] == "renderer_teardown" for event in events
                ),
                "wayland_window_created_count": sum(
                    event["event"] == "wayland_window_created" for event in events
                ),
                "wayland_window_destroyed_count": sum(
                    event["event"] == "wayland_window_destroyed" for event in events
                ),
                "opaque_foreground_pixels": (
                    alpha_event.get("opaque_pixels") if alpha_event else None
                ),
                "opaque_content_pixels": (
                    alpha_event.get("opaque_content_pixels") if alpha_event else None
                ),
                "partial_alpha_pixels": (
                    alpha_event.get("partial_alpha_pixels") if alpha_event else None
                ),
                "background_alpha_samples": (
                    alpha_event.get("background_alpha_samples") if alpha_event else None
                ),
            }
        )
        subprocess.run(
            [str(binary), "quit"],
            env=control_environment,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=3,
            check=False,
        )
        try:
            app.wait(timeout=3)
        except subprocess.TimeoutExpired:
            terminate_exact(app)
        return sample, events
    finally:
        if app is not None and app.poll() is None:
            terminate_exact(app)
        terminate_exact(headless)
        terminate_exact(bus)
        weston_log.close()
        dbus_log.close()
        shutil.rmtree(runtime, ignore_errors=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default=str(ROOT / "target/release/smpl-bar"))
    parser.add_argument("--eww-pid", type=int)
    parser.add_argument("--duration", type=float, default=8.0)
    parser.add_argument("--hidden", action="store_true")
    parser.add_argument("--recreate", action="store_true")
    parser.add_argument(
        "--popup",
        choices=("quick-settings", "usb", "workspace-overview", "keypad-cheatsheet"),
    )
    parser.add_argument(
        "--output",
        default=str(ROOT / "target/smpl-bar-benchmark"),
    )
    parser.add_argument("--enforce", action="store_true")
    args = parser.parse_args()
    binary = Path(args.binary).resolve()
    if not binary.is_file():
        raise SystemExit(f"missing binary: {binary}")
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    eww = installed_baseline(args.eww_pid)
    bar, events = private_run(
        binary, args.duration, output, args.hidden, args.popup, args.recreate
    )
    report = {
        "eww": eww,
        "smpl_bar": bar,
        "ratios": {
            "rss_to_eww": bar["rss_kib"] / eww["rss_kib"],
            "hwm_to_eww": bar["hwm_kib"] / eww["hwm_kib"],
            "threads_to_eww": bar["threads"] / eww["threads"],
            "binary_to_eww": bar["binary_bytes"] / eww["binary_bytes"],
        },
        "targets": {
            "rss_at_most_23_mib": bar["rss_kib"] <= 23 * MIB,
            "first_paint_at_most_0_8_eww": None,
            "first_paint_recorded": bar["first_paint_us"] is not None,
            "popup_first_paint_recorded": bar["popup_first_paint_us"] is not None,
            "wayland_app_id_retained": bar["app_id"] == "smpl-bar",
            "wayland_window_recreated": (
                bar["wayland_window_created_count"] >= 2
                and bar["wayland_window_destroyed_count"] >= 1
                if args.recreate
                else None
            ),
            "opaque_foreground_and_partial_background": (
                (bar["opaque_content_pixels"] or 0) > 0
                and len(bar["background_alpha_samples"] or []) == 3
                and all(
                    isinstance(alpha, int)
                    for alpha in (bar["background_alpha_samples"] or [])
                )
                and all(
                    0 < alpha < 255
                    for alpha in (bar["background_alpha_samples"] or [])
                )
                and bool(bar["background_alpha_samples"])
            ),
        },
        "telemetry": events,
    }
    report_path = output / "measurement.json"
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
    if args.enforce and not all(
        [
            report["targets"]["rss_at_most_23_mib"],
            report["targets"]["first_paint_recorded"],
            report["targets"]["wayland_app_id_retained"],
            report["targets"]["opaque_foreground_and_partial_background"],
            report["targets"]["popup_first_paint_recorded"] if args.popup else True,
            report["targets"]["wayland_window_recreated"]
            if args.recreate
            else True,
        ]
    ):
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())

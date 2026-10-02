#!/usr/bin/env python3
"""Opt-in live Hyprland fixture: private data, fixture-only floating IDs, no screenshots."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import time

COMPACT = "smpl-calendar-nav-fixture"
DETAILS = COMPACT + "-details"


def ctl(*args):
    return subprocess.check_output(["hyprctl", *args], text=True, timeout=3)


def clients():
    return json.loads(ctl("clients", "-j"))


def fixtures():
    return [client for client in clients() if client.get("class") in (COMPACT, DETAILS)]

def details_workflow(binary, env, output, parents, owned):
    log_path = output / "details.log"
    env = dict(env, SMPL_CALENDAR_PROBE_OUTPUT=str(output / "details.json"))
    with log_path.open("w") as log:
        process = subprocess.Popen([str(binary), "--host"], env=env, stdin=subprocess.PIPE,
                                   stdout=log, stderr=subprocess.STDOUT, text=True)
        parents.append(process)
        owned.add(process.pid)

        def target():
            return next((row for row in fixtures() if row["pid"] == process.pid and row["mapped"]), None)

        def command(value):
            assert process.poll() is None, log_path.read_text()
            process.stdin.write(value + "\n")
            process.stdin.flush()

        def measure():
            before = len([line for line in log_path.read_text().splitlines() if line.startswith("DETAILS ")])
            command("measure")
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                lines = [line for line in log_path.read_text().splitlines() if line.startswith("DETAILS ")]
                if len(lines) > before:
                    result = json.loads(lines[-1].removeprefix("DETAILS "))
                    print("DETAILS", result)
                    return result
                assert process.poll() is None, log_path.read_text()
                time.sleep(0.1)
            raise AssertionError("Details measurement timed out: " + log_path.read_text())

        def resize(width, height):
            row = target()
            assert row is not None
            address = hex(int(row["address"], 16))
            response = ctl("dispatch", f'hl.dsp.window.resize({{x={width},y={height},window="address:{address}"}})')
            assert response.strip() == "ok", response
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                if target()["size"] == [width, height]:
                    time.sleep(0.5)
                    return
                time.sleep(0.1)
            raise AssertionError("Compositor did not resize the fixture")

        deadline = time.monotonic() + 5
        while target() is None and time.monotonic() < deadline:
            assert process.poll() is None, log_path.read_text()
            time.sleep(0.1)
        assert target() is not None, log_path.read_text()
        time.sleep(0.8)
        initial = measure()
        print("Initial compositor state:", {key: target().get(key) for key in ("floating", "fullscreen", "fullscreenClient", "size")})
        assert not initial["maximized"], "Initial floating fixture must not report maximized"
        resize(500, 360)
        small = measure()
        resize(364, 320)
        minimum = measure()
        assert minimum["shown"] >= 1 and minimum["overflow_action"]
        resize(1600, 1100)
        large = measure()
        assert (initial["shown"], small["shown"], large["shown"]) == (3, 1, 7)
        command("maximize")
        time.sleep(1)
        maximized = measure()
        assert maximized["maximized"] and maximized["shown"] > initial["shown"]
        assert (maximized["width"], maximized["height"]) != (large["width"], large["height"]), "Maximize must negotiate a different compositor size"
        assert target().get("fullscreen") != 0, "Compositor must confirm maximization"
        command("restore")
        time.sleep(1)
        restored = measure()
        assert not restored["maximized"]
        assert (restored["width"], restored["height"], restored["shown"]) == (1600, 1100, 7)
        command("quit")
        assert process.wait(timeout=5) == 0
        assert target() is None
        print("PASS: actual Wayland responsive event capacity, maximize and restore")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--allow-host-window-fixture", action="store_true", required=True)
    parser.add_argument("--editor", action="store_true")
    parser.add_argument("--typeahead", action="store_true")
    parser.add_argument("--details", action="store_true")
    args = parser.parse_args()
    args.editor = args.editor or args.typeahead
    assert os.environ.get("HYPRLAND_INSTANCE_SIGNATURE") and os.environ.get("WAYLAND_DISPLAY")
    assert not fixtures(), "Another fixture is running; do not touch it"
    root = Path(__file__).resolve().parents[2]
    assert not (args.details and args.editor)
    binary = root / "target/debug/examples" / ("details_probe" if args.details else
                                              "editor_probe" if args.editor else "dashboard_probe")
    output = root / "target/calendar-wayland-probe"
    output.mkdir(parents=True, exist_ok=True)
    prior = json.loads(ctl("activewindow", "-j"))
    key = "__smpl_calendar_probe_" + str(os.getpid())
    rules = (
        f'{key}={{hl.window_rule({{name="{key}-compact",match={{class="^({COMPACT})$"}},'
        'float=true,move="(monitor_w-window_w-2) (monitor_h-window_h-34)",'
        'no_shadow=true,animation="slide",stay_focused=true}),'
        f'hl.window_rule({{name="{key}-details",match={{class="^({DETAILS})$"}},'
        'float=true,center=true,size="1100 700",no_shadow=true,animation="popin"})}'
    )
    owned = set()
    parents = []
    try:
        response = ctl("eval", rules)
        assert response.strip() == "ok", response
        with tempfile.TemporaryDirectory(prefix="private-home-", dir=output) as home:
            env = os.environ.copy()
            env.pop("DBUS_SESSION_BUS_ADDRESS", None)
            env.update(HOME=home, XDG_CONFIG_HOME=home + "/.config", XDG_CACHE_HOME=home + "/.cache",
                       XDG_DATA_HOME=home + "/.local/share", SMPL_CALENDAR_WAYLAND_PROBE="1",
                       SMPL_CALENDAR_PROBE_HOME=home, SMPL_CALENDAR_PROBE_OUTPUT=str(output),
                       SMPL_CALENDAR_NAV_MARKER=home + "/child.pid")
            if args.details:
                details_workflow(binary, env, output, parents, owned)
                return
            compact_pid = None
            for case in (("editor",) if args.editor else ("launch", "focus", "focus-repeat", "failure")):
                failure_marker = Path(home) / "failure-visible"
                case_env = env.copy()
                resize_marker = Path(home) / "editor-resize"
                if case == "editor":
                    case_env["SMPL_EDITOR_RESIZE_MARKER"] = str(resize_marker)
                if case == "failure":
                    case_env.update(HYPRLAND_INSTANCE_SIGNATURE="calendar-fixture-unavailable",
                                    SMPL_CALENDAR_NAV_FAILURE_MARKER=str(failure_marker))
                with (output / f"{case}.log").open("w") as log:
                    parent = subprocess.Popen([str(binary), "--typeahead" if args.typeahead else "--navigation-runtime"], env=case_env,
                                              stdout=log, stderr=subprocess.STDOUT)
                    parents.append(parent)
                    owned.add(parent.pid)
                    deadline = time.monotonic() + (30 if args.editor else 12)
                    observations = []
                    failure_visible = False
                    resized = False
                    while parent.poll() is None and time.monotonic() < deadline:
                        rows = fixtures()
                        owned.update(row["pid"] for row in rows)
                        if case == "editor" and resize_marker.exists() and not resized:
                            target = next(row for row in rows if row["pid"] == parent.pid and row["class"] == DETAILS)
                            address = hex(int(target["address"], 16))
                            response = ctl("dispatch", f'hl.dsp.window.resize({{x=364,y=360,window="address:{address}"}})')
                            assert response.strip() == "ok", response
                            resized = True
                        observations.append({name: sum(row["class"] == name for row in rows)
                                             for name in (COMPACT, DETAILS)})
                        if case == "failure" and not failure_visible and failure_marker.exists():
                            assert "Could not return to calendar" in failure_marker.read_text()
                            assert any(row["class"] == DETAILS and row["mapped"] and
                                       row["pid"] == parent.pid for row in rows)
                            failure_visible = True
                        time.sleep(0.1)
                    code = parent.poll()
                    rows = fixtures()
                    owned.update(row["pid"] for row in rows)
                    print(case, "exit=", code, "windows=",
                          [{k: row[k] for k in ("class", "pid", "mapped")} for row in rows])
                    if code != 0:
                        print((output / f"{case}.log").read_text())
                    assert code == 0, "Details process did not finish successfully"
                    if case == "editor":
                        assert any(item[DETAILS] == 1 for item in observations), "Editor never mapped"
                        assert not rows, "Editor fixture window survived exit"
                        continue
                    if case == "failure":
                        assert failure_visible, "Failure must retain a mapped details window with an error"
                    for _ in range(20):
                        rows = fixtures()
                        if not any(row["class"] == DETAILS for row in rows):
                            break
                        time.sleep(0.1)
                    assert not any(row["class"] == DETAILS for row in rows), "Details WINDOW remained mapped"
                    compact = [row for row in rows if row["class"] == COMPACT and row["mapped"]]
                    assert len(compact) == 1, "Expected exactly one compact WINDOW"
                    if compact_pid is not None:
                        assert compact[0]["pid"] == compact_pid, "Focus branch spawned a duplicate"
                    compact_pid = compact[0]["pid"]
                    assert all(item[COMPACT] <= 1 and item[DETAILS] <= 1 for item in observations)
            print("PASS: native editor interactions" if args.editor else
                  "PASS: launch and repeated focus dismiss details; failure keeps details mapped with error")
    finally:
        # Only this fixture's exact PIDs and executable may be terminated.
        for parent in parents:
            if parent.poll() is None:
                parent.terminate()
                parent.wait(timeout=3)
        for row in fixtures():
            if row["pid"] in owned and Path(f"/proc/{row['pid']}/exe").resolve() == binary.resolve():
                os.kill(row["pid"], signal.SIGTERM)
        for _ in range(30):
            if not fixtures():
                break
            time.sleep(0.1)
        assert not fixtures(), "Fixture windows survived cleanup"
        response = ctl("eval", f'if {key} then for _,r in ipairs({key}) do r:set_enabled(false) end; {key}=nil end')
        assert response.strip() == "ok", response
        current = clients()
        if prior.get("address") and any(row.get("address") == prior["address"] and
                                      row.get("pid") == prior.get("pid") for row in current):
            response = ctl("dispatch", 'hl.dsp.focus({window="address:' + prior["address"] + '"})')
            assert response.strip() == "ok", response
        print("Fixture processes cleaned, temporary rules disabled, prior focus restored when available")


if __name__ == "__main__":
    main()

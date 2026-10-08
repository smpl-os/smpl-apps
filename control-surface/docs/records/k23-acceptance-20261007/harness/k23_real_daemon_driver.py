#!/usr/bin/python3
"""Owner driver for lease k23-keypad-sim-20261007.

Runs INSIDE the prescribed xvfb-run + dbus-run-session lifetime. It launches the
pinned editor exactly as the recipe says, bootstraps the generated project with
MAIN's helper (mode "on"), records identities, captures the ControlSurface1
traffic with dbus-monitor, and then executes owner commands from RUN/ctl/NNNN.json
as its own children (the real control-surface daemon with simulated input and a
non-emitting key sink, private-display GUI focus, read-only /Editor readback).
It ends only after the editor has exited natively.
"""
import ctypes as C
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import traceback

SOURCE = Path("/home/blin/Documents/source/kdenlive/.automation-curves")
BUILD = SOURCE / "build-private-20261004"
PREFIX = Path("/home/blin/Documents/source/kdenlive/.mlt-automation-runtime-audio-phase-20261005")
FILES = Path("/home/blin/.copilot/session-state/3eb20fbc-66e2-4394-8296-a849fb978fed/files")
BINARY = BUILD / "bin/kdenlive"
RUNTIME_SHA = "248f8738c3fc597ad86256d7d88967ec8acb146c8ee89a34556ba02ec8f6c255"
MAX_LIFETIME = 4 * 3600

RUN = Path(sys.argv[1]).resolve(strict=True)
MODE = os.environ.get("K23_DRIVER_MODE", "on")  # "on": acceptance; "off": default-off negotiation
assert MODE in ("on", "off")
assert os.environ.get("K23_LEASE_ROOT") == str(RUN) and os.environ.get("HOME") == str(RUN / "home")
assert re.fullmatch(r":\d+(\.\d+)?", os.environ.get("DISPLAY", "")) and os.environ.get("DBUS_SESSION_BUS_ADDRESS")

CTL = RUN / "ctl"
CTL.mkdir(mode=0o700)
(RUN / "shots").mkdir(mode=0o700)
events = (RUN / "driver-events.jsonl").open("x", buffering=1)


def event(kind, **fields):
    events.write(json.dumps({"at": time.time(), "kind": kind, **fields}, default=str) + "\n")


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2, default=str) + "\n")


def proc_info(pid):
    try:
        stat = Path(f"/proc/{pid}/stat").read_text()
        fields = stat[stat.rindex(")") + 2:].split()
        return {"pid": pid, "ppid": int(fields[1]), "starttime": int(fields[19]),
                "comm": stat[stat.index("(") + 1:stat.rindex(")")],
                "exe": os.path.realpath(f"/proc/{pid}/exe") if os.path.exists(f"/proc/{pid}/exe") else None}
    except (FileNotFoundError, ProcessLookupError, PermissionError):
        return None


def ancestry():
    chain, pid = [], os.getpid()
    while pid > 1 and len(chain) < 8:
        info = proc_info(pid)
        if not info:
            break
        chain.append(info)
        pid = info["ppid"]
    return chain


def children_of(pids):
    found = []
    for entry in Path("/proc").iterdir():
        if entry.name.isdigit():
            info = proc_info(int(entry.name))
            if info and info["ppid"] in pids:
                found.append(info)
    return found


spec = importlib.util.spec_from_file_location("qualified_gui", FILES / "production-gui/run_gui.py")
helper = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helper)
xin = helper.XInput(RUN)
x = xin.x


class WindowAttributes(C.Structure):
    _fields_ = [("x", C.c_int), ("y", C.c_int), ("width", C.c_int), ("height", C.c_int), ("border_width", C.c_int),
                ("depth", C.c_int), ("visual", C.c_void_p), ("root", C.c_ulong), ("class", C.c_int),
                ("bit_gravity", C.c_int), ("win_gravity", C.c_int), ("backing_store", C.c_int),
                ("backing_planes", C.c_ulong), ("backing_pixel", C.c_ulong), ("save_under", C.c_int),
                ("colormap", C.c_ulong), ("map_installed", C.c_int), ("map_state", C.c_int),
                ("all_event_masks", C.c_long), ("your_event_mask", C.c_long), ("do_not_propagate_mask", C.c_long),
                ("override_redirect", C.c_int), ("screen", C.c_void_p)]


x.XFetchName.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(C.c_void_p)]
x.XInternAtom.argtypes = [C.c_void_p, C.c_char_p, C.c_int]
x.XInternAtom.restype = C.c_ulong
x.XGetWindowProperty.argtypes = [C.c_void_p, C.c_ulong, C.c_ulong, C.c_long, C.c_long, C.c_int, C.c_ulong,
                                 C.POINTER(C.c_ulong), C.POINTER(C.c_int), C.POINTER(C.c_ulong),
                                 C.POINTER(C.c_ulong), C.POINTER(C.c_void_p)]
x.XGetGeometry.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(C.c_ulong), C.POINTER(C.c_int), C.POINTER(C.c_int),
                           C.POINTER(C.c_uint), C.POINTER(C.c_uint), C.POINTER(C.c_uint), C.POINTER(C.c_uint)]
x.XGetWindowAttributes.argtypes = [C.c_void_p, C.c_ulong, C.POINTER(WindowAttributes)]
x.XSetInputFocus.argtypes = [C.c_void_p, C.c_ulong, C.c_int, C.c_ulong]
x.XRaiseWindow.argtypes = [C.c_void_p, C.c_ulong]


def windows(app_pid):
    root, parent, count = C.c_ulong(), C.c_ulong(), C.c_uint()
    kids = C.POINTER(C.c_ulong)()
    x.XQueryTree(xin.display, x.XDefaultRootWindow(xin.display), C.byref(root), C.byref(parent), C.byref(kids), C.byref(count))
    atom = x.XInternAtom(xin.display, b"_NET_WM_PID", 1)
    found = []
    for i in range(count.value):
        window = kids[i]
        name, title = C.c_void_p(), ""
        if x.XFetchName(xin.display, window, C.byref(name)) and name.value:
            title = C.string_at(name).decode(errors="replace")
            x.XFree(name)
        px, py, w, h, b, d = C.c_int(), C.c_int(), C.c_uint(), C.c_uint(), C.c_uint(), C.c_uint()
        x.XGetGeometry(xin.display, window, C.byref(root), C.byref(px), C.byref(py), C.byref(w), C.byref(h), C.byref(b), C.byref(d))
        actual, fmt, items, remaining, data = C.c_ulong(), C.c_int(), C.c_ulong(), C.c_ulong(), C.c_void_p()
        x.XGetWindowProperty(xin.display, window, atom, 0, 1, 0, 0, C.byref(actual), C.byref(fmt), C.byref(items), C.byref(remaining), C.byref(data))
        pid = C.cast(data, C.POINTER(C.c_ulong))[0] if data.value and items.value else 0
        if data.value:
            x.XFree(data)
        attributes = WindowAttributes()
        x.XGetWindowAttributes(xin.display, window, C.byref(attributes))
        found.append({"window": window, "title": title, "x": px.value, "y": py.value, "width": w.value, "height": h.value,
                      "pid": pid, "map_state": attributes.map_state})
    x.XFree(kids)
    return found


def main_window(app_pid):
    candidates = [w for w in windows(app_pid) if w["pid"] == app_pid and w["map_state"] == 2 and w["width"] >= 600 and w["height"] >= 350]
    return max(candidates, key=lambda w: w["width"] * w["height"]) if candidates else None


def focus(app_pid):
    window = main_window(app_pid)
    if not window:
        raise RuntimeError("No mapped (IsViewable) main window owned by the editor PID")
    x.XRaiseWindow(xin.display, window["window"])
    x.XSetInputFocus(xin.display, window["window"], 2, 0)
    x.XFlush(xin.display)
    return window


def gdbus(service, path, interface_method, *args, timeout=10):
    out = subprocess.run(["/usr/bin/gdbus", "call", "--session", "--dest", service, "--object-path", path,
                          "--method", interface_method, *args], text=True, capture_output=True, timeout=timeout)
    return {"rc": out.returncode, "stdout": out.stdout.strip(), "stderr": out.stderr.strip()}


def editor(service, method, params):
    import ast
    reply = gdbus(service, "/Editor", "org.kde.kdenlive.Editor.request", method,
                  json.dumps(params, separators=(",", ":"), allow_nan=False), timeout=30)
    if reply["rc"] != 0:
        return {"transport_error": reply}
    return json.loads(ast.literal_eval(reply["stdout"])[0])


def names():
    out = subprocess.run(["/usr/bin/gdbus", "call", "--session", "--dest", "org.freedesktop.DBus", "--object-path",
                          "/org/freedesktop/DBus", "--method", "org.freedesktop.DBus.ListNames"],
                         text=True, capture_output=True, timeout=5)
    return out.stdout


result = {"lease": "k23-keypad-sim-20261007", "mode": MODE, "run": str(RUN), "started": time.time(),
          "display": os.environ["DISPLAY"], "dbus": os.environ["DBUS_SESSION_BUS_ADDRESS"],
          "environment": dict(os.environ), "ancestry": ancestry()}
spawned = {}
app = None
a11y = None
try:
    ancestors = [p["pid"] for p in result["ancestry"]]
    result["session_processes"] = children_of(ancestors[1:4])  # dbus-daemon, Xvfb and friends
    monitor_log = (RUN / "dbus-monitor.log").open("x")
    spawned["dbus-monitor"] = subprocess.Popen(
        ["/usr/bin/dbus-monitor", "--session", "interface='org.kde.kdenlive.ControlSurface1'",
         "type='method_return'", "type='error'"], stdout=monitor_log, stderr=subprocess.STDOUT)
    result["dbus_monitor_pid"] = spawned["dbus-monitor"].pid
    app_command = [
        "/usr/bin/python3", "-B", str(SOURCE / "scripts/launch-automation-backend.py"),
        "--prefix", str(PREFIX), "--manifest", str(BUILD / "qualified-backend.json"),
        "--expected-sha256", RUNTIME_SHA, "--", str(BINARY), "--config", str(RUN / "config/kdenliverc")]
    result["app_command"] = app_command
    log = (RUN / "application.log").open("x")
    app = subprocess.Popen(app_command, stdout=log, stderr=log, cwd=RUN / "work")
    service = f"org.kde.kdenlive-{app.pid}"
    result.update({"app_pid": app.pid, "service": service})
    event("launch", pid=app.pid, service=service)
    deadline = time.monotonic() + 240
    while time.monotonic() < deadline and os.path.realpath(f"/proc/{app.pid}/exe") != str(BINARY):
        assert app.poll() is None, f"editor exited early: {app.returncode}"
        time.sleep(.1)
    assert os.path.realpath(f"/proc/{app.pid}/exe") == str(BINARY), "PID is not the pinned editor"
    result["app_identity"] = proc_info(app.pid)
    while time.monotonic() < deadline and service not in names():
        assert app.poll() is None, f"editor exited early: {app.returncode}"
        time.sleep(.2)
    assert service in names(), f"{service} never appeared"
    # The welcome screen answers /Editor; MainWindow (and with it the optional
    # /ControlSurface object) is configured once a project exists.
    probe = editor(service, "project.get", {})
    while "transport_error" in probe and time.monotonic() < deadline:
        time.sleep(.5)
        probe = editor(service, "project.get", {})
    result["editor_probe"] = probe
    result["capabilities_at_welcome"] = gdbus(service, "/ControlSurface", "org.kde.kdenlive.ControlSurface1.Capabilities")
    # Both modes build the same generated project, so MainWindow (and its
    # configure() of the optional interface) exists before the probe.
    fixture = subprocess.run(["/usr/bin/python3", "-B", str(FILES / "k23_keypad_fixture.py"), str(RUN), service],
                             text=True, capture_output=True, timeout=600)
    result["fixture"] = {"rc": fixture.returncode, "stdout": fixture.stdout, "stderr": fixture.stderr[-4000:]}
    assert fixture.returncode == 0, "fixture bootstrap failed"
    if MODE == "on":
        deadline = time.monotonic() + 60
        caps = gdbus(service, "/ControlSurface", "org.kde.kdenlive.ControlSurface1.Capabilities")
        while caps["rc"] != 0 and time.monotonic() < deadline:
            time.sleep(.3)
            caps = gdbus(service, "/ControlSurface", "org.kde.kdenlive.ControlSurface1.Capabilities")
        result["capabilities_probe"] = caps
        assert caps["rc"] == 0, f"ControlSurface1 unavailable: {caps}"
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline and not main_window(app.pid):
        time.sleep(.2)
    result["windows_at_ready"] = windows(app.pid)
    result["focused_window"] = focus(app.pid)
    if MODE == "off":
        # Interface off (the default): the object stays absent with the GUI up.
        caps = gdbus(service, "/ControlSurface", "org.kde.kdenlive.ControlSurface1.Capabilities")
        result["capabilities_probe"] = caps
        assert caps["rc"] != 0 and "UnknownObject" in caps["stderr"], f"interface must be absent: {caps}"
    a11y = helper.Driver(RUN, app)
    write_json(RUN / "ready.json", {k: result[k] for k in ("app_pid", "service", "display", "dbus", "focused_window")})
    event("ready")

    seen = set()
    started = time.monotonic()
    quitting = False
    while time.monotonic() - started < MAX_LIFETIME:
        if app.poll() is not None:
            result["app_exit"] = app.returncode
            event("app-exit", code=app.returncode, requested=quitting)
            break
        pending = sorted(p for p in CTL.glob("[0-9][0-9][0-9][0-9].json") if p.name not in seen)
        if not pending:
            time.sleep(.1)
            continue
        path = pending[0]
        seen.add(path.name)
        stem = path.stem
        try:
            cmd = json.loads(path.read_text())
        except json.JSONDecodeError:
            time.sleep(.2)  # still being written
            seen.discard(path.name)
            continue
        out = {"command": cmd, "started": time.time()}
        event("command", id=stem, command=cmd)
        try:
            op = cmd["op"]
            if op == "exec":
                with (CTL / f"{stem}.out").open("x") as sink:
                    proc = subprocess.run(cmd["argv"], stdout=sink, stderr=subprocess.STDOUT, timeout=cmd.get("timeout", 300),
                                          cwd=cmd.get("cwd", str(RUN / "work")))
                out["rc"] = proc.returncode
            elif op == "spawn":
                sink = (CTL / f"{stem}.out").open("x")
                proc = subprocess.Popen(cmd["argv"], stdout=sink, stderr=subprocess.STDOUT, cwd=cmd.get("cwd", str(RUN / "work")))
                spawned[cmd["name"]] = proc
                out["pid"] = proc.pid
            elif op == "wait":
                proc = spawned[cmd["name"]]
                out["rc"] = proc.wait(timeout=cmd.get("timeout", 300))
            elif op == "term":
                proc = spawned[cmd["name"]]
                if proc.poll() is None:
                    proc.terminate()
                out["rc"] = proc.wait(timeout=20)
            elif op == "focus":
                out["window"] = focus(app.pid)
            elif op == "windows":
                out["windows"] = windows(app.pid)
            elif op == "click":
                xin.click(cmd["x"], cmd["y"], cmd.get("count", 1))
            elif op == "move":
                xin.move(cmd["x"], cmd["y"])
            elif op == "chord":
                xin.chord(cmd["keys"])
            elif op == "screenshot":
                label = cmd["label"]
                assert re.fullmatch(r"[a-zA-Z0-9_-]+", label)
                subprocess.run(["/usr/bin/import", "-window", "root", str(RUN / "shots" / f"{label}.png")], check=True, timeout=30)
                out["path"] = str(RUN / "shots" / f"{label}.png")
            elif op == "a11y":
                items = [item for item, _ in a11y.tree()]
                if cmd.get("showing", True):
                    items = [i for i in items if i.get("showing")]
                write_json(CTL / f"{stem}.a11y.json", items)
                out["count"] = len(items)
            elif op == "click-a11y":
                cx, cy = a11y.center(cmd["name"], role=cmd.get("role"), occurrence=cmd.get("occurrence", 0))
                xin.click(cx + cmd.get("dx", 0), cy + cmd.get("dy", 0), cmd.get("count", 1))
                out["clicked"] = [cx, cy]
            elif op == "editor":
                out["reply"] = editor(service, cmd["method"], cmd.get("params", {}))
            elif op == "capabilities":
                out["reply"] = gdbus(service, "/ControlSurface", "org.kde.kdenlive.ControlSurface1.Capabilities")
            elif op == "names":
                out["names"] = names()
            elif op == "quit":
                quitting = True
                out["reply"] = editor(service, "application.quit", cmd.get("params", {}))
                try:
                    out["app_exit"] = app.wait(timeout=cmd.get("timeout", 90))
                except subprocess.TimeoutExpired:
                    out["blocked"] = "editor still running after quit request; X/bus preserved"
            else:
                raise ValueError(f"unknown op {op}")
        except Exception as error:
            out["error"] = repr(error)
            out["traceback"] = traceback.format_exc()
        out["finished"] = time.time()
        write_json(CTL / f"{stem}.result.json", out)
    else:
        result["lifetime_exceeded"] = True
except Exception as error:
    result["error"] = repr(error)
    result["traceback"] = traceback.format_exc()
finally:
    if app is not None and app.poll() is None:
        # Never tear down X/bus under a live editor (UF9): wait for a native exit.
        event("waiting-for-native-exit")
        while app.poll() is None:
            if (CTL / "quit-now").exists():
                editor(result.get("service", ""), "application.quit", json.loads((CTL / "quit-now").read_text() or "{}"))
                (CTL / "quit-now").rename(CTL / "quit-now.done")
            time.sleep(.5)
        result["app_exit"] = app.returncode
    for name, proc in spawned.items():
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()
        result.setdefault("spawned_exit", {})[name] = proc.returncode
    result["finished"] = time.time()
    if app is not None:
        result["app_absent"] = not Path(f"/proc/{app.pid}").exists()
    write_json(RUN / "driver-result.json", result)
    events.close()
sys.exit(0 if result.get("app_exit") == 0 and "error" not in result else 1)

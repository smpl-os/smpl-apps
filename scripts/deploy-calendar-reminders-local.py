#!/usr/bin/env python3
"""Back up, install, and restart only the current user's calendar reminders."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
from contextlib import closing
from datetime import datetime, timezone

ROOT = Path(__file__).resolve().parents[1]
UNIT = "smpl-calendar-alertd.service"
UNIT_PATH = Path("/usr/local/lib/systemd/user") / UNIT
BINARY_DIR = Path("/usr/local/bin")
BACKUP_BASE = Path("/var/backups/smpl-calendar")
INSTALL_LOCK = Path("/var/lock/smpl-calendar-install.lock")
DAEMON = BINARY_DIR / "smpl-calendar-alertd"
BINARIES = ("smpl-calendar-alertd", "smpl-calendar")


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def sync_file(path):
    with Path(path).open("rb") as stream:
        os.fsync(stream.fileno())


def systemctl(*args, check=True):
    result = subprocess.run(["systemctl", "--user", "--no-pager", *args], capture_output=True,
                            text=True, timeout=20)
    if check and result.returncode:
        raise RuntimeError("User-service operation failed: " + result.stderr.strip())
    return result


def service_state():
    result = systemctl("show", UNIT, "--property=LoadState,UnitFileState,FragmentPath,DropInPaths,MainPID,ActiveState",
                       check=False)
    values = dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)
    if not values or values.get("LoadState") not in ("loaded", "not-found", "masked"):
        raise RuntimeError("Cannot determine calendar service state: " + result.stderr.strip())
    return values


def preflight_unit(expected, state):
    if state.get("LoadState") == "masked" or state.get("UnitFileState") == "masked":
        raise RuntimeError("Calendar reminders are masked; explicit opt-out preserved")
    if state.get("LoadState") != "not-found":
        if state.get("UnitFileState") not in ("enabled", "enabled-runtime"):
            raise RuntimeError("Existing disabled/custom calendar service preserved; review before enabling")
        if state.get("FragmentPath") != str(UNIT_PATH) or state.get("DropInPaths"):
            raise RuntimeError("Custom calendar service or drop-ins preserved; review before updating")
    if UNIT_PATH.is_symlink():
        raise RuntimeError("Refusing to replace a symlinked calendar unit")
    if UNIT_PATH.exists() and UNIT_PATH.read_bytes() != expected.read_bytes():
        raise RuntimeError("Existing calendar unit differs; refusing to overwrite possible customization")


def process_identity(pid):
    proc = Path("/proc") / str(pid)
    try:
        if proc.stat().st_uid != os.getuid():
            return None
        executable = os.readlink(proc / "exe")
        if executable.removesuffix(" (deleted)") != str(DAEMON):
            return None
        stat = (proc / "stat").read_text().rsplit(")", 1)[1].split()
        binary = (proc / "exe").stat()
        return {"pid": int(pid), "uid": os.getuid(), "start_ticks": int(stat[19]),
                "device": binary.st_dev, "inode": binary.st_ino, "executable": executable}
    except (FileNotFoundError, ProcessLookupError, PermissionError):
        return None


def daemon_processes():
    return [identity for proc in Path("/proc").iterdir() if proc.name.isdigit()
            if (identity := process_identity(proc.name)) is not None]


def stop_old_daemons(installed):
    stopped = set()
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        old = [p for p in daemon_processes()
               if (p["device"], p["inode"]) != (installed.st_dev, installed.st_ino)]
        if not old:
            return len(stopped)
        for identity in old:
            key = (identity["pid"], identity["start_ticks"])
            if key in stopped:
                continue
            try:
                pidfd = os.pidfd_open(identity["pid"])
            except ProcessLookupError:
                continue
            try:
                if process_identity(identity["pid"]) == identity:
                    signal.pidfd_send_signal(pidfd, signal.SIGTERM)
                    stopped.add(key)
            except ProcessLookupError:
                pass
            finally:
                os.close(pidfd)
        time.sleep(0.2)
    raise RuntimeError("An old calendar daemon survived SIGTERM; new service was not started")


def legacy_receipts(database):
    with closing(sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True, timeout=10)) as db:
        exists = db.execute("SELECT 1 FROM sqlite_master WHERE type='table' AND name='sent_alerts'").fetchone()
        return set(db.execute("SELECT event_id, alert_ts FROM sent_alerts")) if exists else set()


def install_root():
    if os.geteuid() != 0 or not os.environ.get("PKEXEC_UID"):
        raise RuntimeError("Root installation must be entered through pkexec")
    with INSTALL_LOCK.open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        base = BACKUP_BASE
        base.mkdir(parents=True, exist_ok=True, mode=0o755)
        backup = Path(tempfile.mkdtemp(prefix=stamp + ".", dir=base))
        sources = [(ROOT / "calendar/systemd" / UNIT, UNIT_PATH, 0o644)]
        sources += [(ROOT / "target/release" / name, BINARY_DIR / name, 0o755)
                    for name in BINARIES]
        stages = []
        installed = []
        saved_files = []
        try:
            for source, destination, mode in sources:
                if not source.is_file() or source.is_symlink():
                    raise RuntimeError("Missing regular installation source: " + str(source))
                if destination.is_symlink() or (destination.exists() and not destination.is_file()):
                    raise RuntimeError("Refusing unexpected installation target: " + str(destination))
                if destination != UNIT_PATH and not destination.exists():
                    raise RuntimeError("Expected existing binary for verified backup: " + str(destination))
                if destination.exists():
                    saved = backup / destination.name
                    shutil.copy2(destination, saved)
                    sync_file(saved)
                    if digest(saved) != digest(destination):
                        raise RuntimeError("Installed-file backup verification failed")
                    saved_files.append({"path": str(saved), "sha256": digest(saved)})
                destination.parent.mkdir(parents=True, exist_ok=True, mode=0o755)
                fd, name = tempfile.mkstemp(prefix="." + destination.name + ".", dir=destination.parent)
                stage = Path(name)
                stages.append((stage, destination))
                with os.fdopen(fd, "wb") as stream, source.open("rb") as original:
                    shutil.copyfileobj(original, stream)
                    os.fchmod(stream.fileno(), mode)
                    os.fchown(stream.fileno(), 0, 0)
                    stream.flush()
                    os.fsync(stream.fileno())
                if digest(stage) != digest(source):
                    raise RuntimeError("Staged binary verification failed")
            # Unit first: an old GUI launching the new daemon sees the service.
            for stage, destination in stages:
                os.replace(stage, destination)
                installed.append({"path": str(destination), "sha256": digest(destination)})
            result = {"package_backup": str(backup), "backups": saved_files, "installed": installed}
            (backup / "manifest.json").write_text(json.dumps(result, indent=2))
            print(json.dumps(result), flush=True)
        finally:
            for stage, _destination in stages:
                if stage.exists():
                    stage.unlink()


def main():
    if sys.argv[1:] == ["--install-root"]:
        install_root()
        return
    if len(sys.argv) != 1 or os.geteuid() == 0:
        raise RuntimeError("Run this installer without arguments as the desktop user")
    unit_source = ROOT / "calendar/systemd" / UNIT
    for name in BINARIES:
        source = ROOT / "target/release" / name
        if not source.is_file() or not os.access(source, os.X_OK):
            raise RuntimeError("Build both release binaries before deploying")
    systemctl("daemon-reload")
    before = service_state()
    preflight_unit(unit_source, before)
    backup = subprocess.run([sys.executable, str(ROOT / "scripts/backup-calendar-data.py")],
                            check=True, capture_output=True, text=True, timeout=75)
    data = json.loads(backup.stdout)
    print("Verified private database backup: " + data["database"], flush=True)
    recorded = daemon_processes()
    (Path(data["database"]).parent / "daemon-processes-before.json").write_text(json.dumps(recorded, indent=2))
    print("Requesting pkexec authentication for both binaries and the user unit", flush=True)
    subprocess.run(["pkexec", "/usr/bin/python3", str(Path(__file__).resolve()), "--install-root"], check=True)
    systemctl("daemon-reload")
    ready = service_state()
    if ready.get("FragmentPath") != str(UNIT_PATH) or ready.get("DropInPaths"):
        raise RuntimeError("Service configuration changed during installation; no process cleanup performed")
    if ready.get("LoadState") != "loaded":
        raise RuntimeError("Installed service is not ready; no process cleanup performed")
    installed = DAEMON.stat()
    if before.get("ActiveState") == "active":
        systemctl("stop", UNIT)
    removed = stop_old_daemons(installed)
    backup = subprocess.run([sys.executable, str(ROOT / "scripts/backup-calendar-data.py")],
                            check=True, capture_output=True, text=True, timeout=75)
    data = json.loads(backup.stdout)
    print("Verified pre-migration database backup: " + data["database"], flush=True)
    database = Path.home() / ".local/share/smplos/calendar/events.db"
    history_before = legacy_receipts(database)
    systemctl("enable", "--now", UNIT)
    time.sleep(2)
    deadline = time.monotonic() + 10
    while True:
        after = service_state()
        running = daemon_processes()
        main_pid = int(after.get("MainPID", "0"))
        if len(running) == 1 and running[0]["pid"] == main_pid:
            break
        if time.monotonic() >= deadline:
            raise RuntimeError("Expected exactly one service-owned reminder daemon")
        time.sleep(0.2)
    if after.get("ActiveState") != "active" or after.get("UnitFileState") != "enabled":
        raise RuntimeError("Calendar reminder service did not become enabled and active")
    if len(running) != 1 or running[0]["pid"] != main_pid:
        raise RuntimeError("Expected exactly one service-owned reminder daemon")
    if (running[0]["device"], running[0]["inode"]) != (installed.st_dev, installed.st_ino):
        raise RuntimeError("Reminder service is not running the installed binary")
    if not history_before.issubset(legacy_receipts(database)):
        raise RuntimeError("Legacy reminder history disappeared; inspect the verified backup")
    with closing(sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True, timeout=10)) as db:
        if db.execute("PRAGMA integrity_check").fetchall() != [("ok",)]:
            raise RuntimeError("Post-install database integrity check failed")
    print(json.dumps({"service": UNIT, "enabled": True, "active": True, "daemon_count": 1,
                      "pid": main_pid, "old_daemons_terminated": removed,
                      "legacy_history_preserved": True, "database_backup": data["database"]}), flush=True)


if __name__ == "__main__":
    main()

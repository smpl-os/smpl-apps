#!/usr/bin/env python3
"""Create a private, verified SQLite snapshot before calendar daemon migration."""
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import time
from contextlib import closing
from datetime import datetime, timezone


def main():
    source = Path.home() / ".local/share/smplos/calendar/events.db"
    if not source.is_file() or source.stat().st_uid != os.getuid():
        raise RuntimeError("Run as the calendar user with an existing owned events database")
    os.umask(0o077)
    state = Path(os.environ.get("XDG_STATE_HOME", str(Path.home() / ".local/state")))
    if not state.is_absolute():
        raise RuntimeError("XDG_STATE_HOME must be absolute")
    base = state / "smplos/calendar/backups"
    base.mkdir(parents=True, exist_ok=True, mode=0o700)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    directory = Path(tempfile.mkdtemp(prefix=stamp + ".", dir=base))
    target = directory / "events.db"
    deadline = time.monotonic() + 60

    def progress(_status, _remaining, _total):
        if time.monotonic() >= deadline:
            raise TimeoutError("Calendar backup timed out; no installation may proceed")

    with closing(sqlite3.connect(source.resolve().as_uri() + "?mode=ro", uri=True, timeout=10)) as src:
        with closing(sqlite3.connect(target)) as dst:
            src.backup(dst, pages=256, progress=progress, sleep=0.05)
            if dst.execute("PRAGMA integrity_check").fetchall() != [("ok",)]:
                raise RuntimeError("Calendar backup integrity check failed")
    os.chmod(target, 0o600)
    with target.open("rb") as stream:
        digest = hashlib.file_digest(stream, "sha256").hexdigest()
        os.fsync(stream.fileno())
    manifest = {
        "database": str(target),
        "sha256": digest,
        "integrity_check": "ok",
        "created_at": datetime.now(timezone.utc).isoformat(),
    }
    with (directory / "manifest.json").open("x") as stream:
        json.dump(manifest, stream, indent=2)
        stream.flush()
        os.fsync(stream.fileno())
    print(json.dumps(manifest))


if __name__ == "__main__":
    main()

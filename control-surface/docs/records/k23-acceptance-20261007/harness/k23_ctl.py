#!/usr/bin/python3
"""Owner-side: enqueue one driver command and wait for its result."""
import json, sys, time
from pathlib import Path
run = Path(sys.argv[1]); cmd = json.loads(sys.argv[2]); wait = float(sys.argv[3]) if len(sys.argv) > 3 else 600
ctl = run / "ctl"
n = 1 + max([int(p.stem) for p in ctl.glob("[0-9][0-9][0-9][0-9].json")] or [0])
tmp = ctl / f".{n:04d}.tmp"; tmp.write_text(json.dumps(cmd)); tmp.rename(ctl / f"{n:04d}.json")
res = ctl / f"{n:04d}.result.json"; t = time.monotonic()
while not res.exists() and time.monotonic() - t < wait: time.sleep(.1)
print(f"[{n:04d}]", res.read_text() if res.exists() else "TIMEOUT waiting for result")

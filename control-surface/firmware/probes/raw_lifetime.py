#!/usr/bin/env python3
# Diagnosis: one RAW_MODE(T) request, then poll GET_INFO's raw flag until it drops.
#
# Usage: python3 firmware/probes/raw_lifetime.py T [T ...]   (timeouts in ms)
# Needs the pad on control-surface firmware 2.0.2+. Stop control-surfaced
# first: it reads the same hidraw node and sends its own raw-mode heartbeats.
# Ends with raw mode off.
import sys, time
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))  # firmware/, which has padctl.py
import padctl as P
pad = P.Pad(P.find_pad())
def info():
    pad.send(P.CMD_GET_INFO)
    t0 = time.monotonic()
    while time.monotonic() - t0 < 0.2:
        r = pad.read(0.2)
        if r and r[0] == P.CFG_ID and r[1] == P.CMD_GET_INFO:
            return r
    return None
for T in [int(x) for x in sys.argv[1:]]:
    time.sleep(0.3)
    r = pad.request(P.CMD_RAW_MODE, T.to_bytes(2, "little"))
    t0 = time.monotonic()
    on = r[8]
    last_on = t0
    misses = 0
    while time.monotonic() - t0 < T / 1000 * 1.5 + 0.5:
        r = info()
        if r is None:
            misses += 1
            continue
        if r[13]:
            last_on = time.monotonic()
        else:
            break
        time.sleep(0.003)
    print(f"timeout {T:5d} ms: raw lasted {1000*(last_on - t0):7.1f} ms (+<5 ms)  ratio {(last_on - t0)*1000/T:.3f}  missed replies {misses}")
pad.send(P.CMD_RAW_MODE, b"\0\0")

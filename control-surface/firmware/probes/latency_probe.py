#!/usr/bin/env python3
# Diagnosis only: GET_INFO round trips (read-only) and, with --raw, the raw-mode
# flag under the daemon's heartbeat pattern (500 ms, timeout 1500 ms).
#
# Usage: python3 firmware/probes/latency_probe.py [--raw]   (runs 15 s)
# Needs the pad on control-surface firmware 2.0.1+ (GET_STATS); --raw needs
# 2.0.2+. Stop control-surfaced first: it reads the same hidraw node. --raw
# switches raw mode on and off again; if the probe dies, the pad leaves raw
# mode by itself about 1.5 s after the last heartbeat.
import sys, time, statistics
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))  # firmware/, which has padctl.py
import padctl as P

raw = "--raw" in sys.argv
secs = 15.0
pad = P.Pad(P.find_pad())
def stats():
    out = []
    for page in (0, 1):
        r = pad.request(P.CMD_GET_STATS, bytes([page, 0]))
        out.append(list(r[3:15]))
    return out
print("stats before", stats())
lat, flaps, raw_off_samples, samples, events = [], 0, 0, 0, 0
prev_raw = None
beat = 0.0
end = time.monotonic() + secs
worst = (0, 0)
while time.monotonic() < end:
    now = time.monotonic()
    if raw and now >= beat:
        pad.send(P.CMD_RAW_MODE, (1500).to_bytes(2, "little"))
        beat = now + 0.5
    t0 = time.monotonic()
    pad.send(P.CMD_GET_INFO)
    rep = None
    while time.monotonic() - t0 < 1.0:
        r = pad.read(1.0 - (time.monotonic() - t0))
        if r is None:
            break
        if r[0] == P.RAW_ID:
            events += 1
            continue
        if r[0] == P.CFG_ID and r[1] == P.CMD_GET_INFO:
            rep = r
            break
    dt = (time.monotonic() - t0) * 1000
    samples += 1
    if rep is None:
        print(f"{time.strftime('%H:%M:%S')} NO REPLY within 1 s")
        continue
    lat.append(dt)
    if dt > worst[0]:
        worst = (dt, samples)
    r13 = rep[13]
    if raw:
        if r13 == 0:
            raw_off_samples += 1
        if prev_raw is not None and r13 != prev_raw:
            flaps += 1
            print(f"{time.strftime('%H:%M:%S')}.{int((time.time()%1)*1000):03d} raw flag {prev_raw} -> {r13}")
        prev_raw = r13
    time.sleep(0.02)
if raw:
    pad.send(P.CMD_RAW_MODE, b"\0\0")
lat.sort()
print(f"samples {samples} replies {len(lat)} raw-events-seen {events}")
print(f"latency ms: min {lat[0]:.2f} p50 {statistics.median(lat):.2f} p90 {lat[int(len(lat)*.9)]:.2f} p99 {lat[int(len(lat)*.99)]:.2f} max {lat[-1]:.2f} (sample {worst[1]})")
if raw:
    print(f"raw flag flips {flaps}, samples with raw off {raw_off_samples}")
print("stats after", stats())

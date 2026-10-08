#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Talk to the control-surface pad firmware (protocol v3) over hidraw.

Finds the pad by VID:PID 1189:8890 and serial key153; nothing else is opened.
Read-only commands: info, get, dump, stats (--clear zeroes the counters in RAM;
nothing is stored). watch switches raw mode on with heartbeats and off again. Commands that change the pad: set, layer,
reset, bootloader; each needs --yes.

    padctl.py info
    padctl.py get [--layer N]              all 24 slots
    padctl.py dump                         raw data flash, 128 bytes
    padctl.py stats [--clear]              encoder diagnostics (2.0.1+): detents per
                                           knob and direction, missed states, drops
    padctl.py watch [--seconds S]          raw mode with heartbeats; prints events
    padctl.py rawcheck [--seconds S]       raw mode under the daemon's heartbeats (2.0.2+):
                                           it must hold, every command must be answered,
                                           and silence must end it once after ~1.5 s
    padctl.py set SLOT TYPE MOD CODE [--layer N] --yes
    padctl.py layer N [--persist] --yes
    padctl.py reset --yes
    padctl.py bootloader --yes
"""

from __future__ import annotations

import argparse
import glob
import os
import select
import sys
import time

VID, PID, SERIAL = 0x1189, 0x8890, "key153"
CFG_ID, RAW_ID = 3, 5
CMD_GET_INFO, CMD_GET_ACTION, CMD_SET_ACTION, CMD_RESET = 1, 2, 3, 4
CMD_BOOTLOADER, CMD_DUMP, CMD_RAW_MODE, CMD_SET_LAYER, CMD_GET_STATS = 5, 6, 8, 9, 0x0A
CMD_GET_KEYS = 0x0B
STATUS = {1: "ok", 2: "bad index", 3: "bad action", 4: "write failed", 5: "bad argument"}
TYPES = {0: "none", 1: "key", 2: "consumer", 3: "mouse", 4: "layer"}
EVENTS = {1: "down", 2: "up", 3: "tap"}
ROLES = ("ccw", "press", "cw")


def slot_name(slot: int) -> str:
    if slot < 15:
        return f"key{slot + 1}"
    k = slot - 15
    return f"knob{k // 3 + 1}.{ROLES[k % 3]}"


def find_pad() -> str:
    for node in sorted(glob.glob("/sys/class/hidraw/hidraw*")):
        try:
            with open(os.path.join(node, "device", "uevent")) as fh:
                ev = dict(line.rstrip("\n").split("=", 1) for line in fh if "=" in line)
        except OSError:
            continue
        hid_id = ev.get("HID_ID", "").upper()
        if hid_id.endswith(f"{VID:08X}:{PID:08X}") and ev.get("HID_UNIQ") == SERIAL:
            return "/dev/" + os.path.basename(node)
    sys.exit("pad not found (1189:8890, serial key153)")


class Pad:
    def __init__(self, path: str):
        self.fd = os.open(path, os.O_RDWR | os.O_NONBLOCK)
        self.path = path

    def send(self, cmd: int, args: bytes = b"") -> None:
        buf = bytes([CFG_ID, cmd]) + args
        os.write(self.fd, buf.ljust(16, b"\0")[:16])

    def read(self, timeout: float) -> bytes | None:
        r, _, _ = select.select([self.fd], [], [], timeout)
        if not r:
            return None
        return os.read(self.fd, 64)

    def request(self, cmd: int, args: bytes = b"", timeout: float = 1.0) -> bytes:
        self.send(cmd, args)
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            rep = self.read(end - time.monotonic())
            if rep and rep[0] == CFG_ID and len(rep) >= 2 and rep[1] == cmd:
                return rep
        sys.exit(f"no reply to command {cmd:#04x} from {self.path}")


def describe(t: int, mod: int, code: int) -> str:
    if t == 1:
        return f"key usage={code:#04x} mods={mod:#04x}"
    if t == 2:
        return f"consumer usage={code:#05x}"
    if t == 3:
        sub = ("button", "wheel", "pan", "move-x", "move-y")[code >> 8] if (code >> 8) < 5 else "?"
        val = code & 0xFF
        return f"mouse {sub} {val if sub == 'button' else (val - 256 if val > 127 else val)}"
    if t == 4:
        op = ("toggle", "momentary", "set")[code >> 8] if (code >> 8) < 3 else "?"
        return f"layer {op} {code & 0xFF}"
    return "none"


def rawcheck(pad: "Pad", seconds: float, force: bool) -> int:
    """No key presses needed. 2.0.1 fails this (raw mode ended at every
    wrap of its 8-bit millisecond clock); 2.0.2 must pass."""
    r = pad.request(CMD_GET_INFO)
    version = (r[8], r[9], r[10])
    if version < (2, 0, 2) and not force:
        print(f"rawcheck: skipped, firmware {version[0]}.{version[1]}.{version[2]} predates 2.0.2 (--force runs it anyway)")
        return 0

    def raw_counters():
        rep = pad.request(CMD_GET_STATS, bytes([2, 0]), timeout=0.5)
        if rep[7] != 1:
            return None
        return rep[3] | rep[4] << 8, rep[5] | rep[6] << 8      # entries, expiries

    def ask(cmd, args=b""):
        pad.send(cmd, args)
        end = time.monotonic() + 0.2
        while time.monotonic() < end:
            rep = pad.read(end - time.monotonic())
            if rep and rep[0] == CFG_ID and rep[1] == cmd:
                return rep
        return None

    before = raw_counters() if version >= (2, 0, 2) else None
    failures = []
    samples = off = unanswered = 0
    epochs = set()
    beat = 0.0
    last_beat = 0.0
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        now = time.monotonic()
        if now >= beat:
            # The heartbeat and the snapshot request back to back: 2.0.1
            # dropped a command that arrived while one was pending.
            pad.send(CMD_RAW_MODE, (1500).to_bytes(2, "little"))
            last_beat = now
            beat = now + 0.5
        rep = ask(CMD_GET_KEYS if version >= (2, 0, 2) else CMD_GET_INFO)
        samples += 1
        if rep is None:
            unanswered += 1
            continue
        active = rep[8] if rep[1] == CMD_GET_KEYS else rep[13]
        if rep[1] == CMD_GET_KEYS:
            epochs.add(rep[2])
        if samples > 3 and not active:
            off += 1
        time.sleep(0.01)
    if off:
        failures.append(f"raw mode was off in {off} of {samples} samples")
    if unanswered:
        failures.append(f"{unanswered} of {samples} commands got no answer")
    if len(epochs) > 1:
        failures.append(f"raw mode restarted {len(epochs) - 1} time(s)")
    # Silence: raw mode must end once, about 1.5 s after the last heartbeat.
    ended = None
    while time.monotonic() - last_beat < 3.0:
        rep = ask(CMD_GET_INFO)
        if rep and not rep[13]:
            ended = time.monotonic() - last_beat
            break
        time.sleep(0.01)
    if ended is None or not 1.4 <= ended <= 1.7:
        failures.append(f"raw mode ended {ended if ended is None else round(ended, 3)} s after the last heartbeat (want ~1.5)")
    after = raw_counters() if version >= (2, 0, 2) else None
    if before and after and (after[0] - before[0], after[1] - before[1]) != (1, 1):
        failures.append(f"the pad counted {after[0] - before[0]} raw start(s) and {after[1] - before[1]} expiry(ies), want 1 and 1")
    pad.send(CMD_RAW_MODE, b"\0\0")
    print(f"rawcheck: {samples} samples over {seconds:g} s, raw off {off}, unanswered {unanswered}, "
          f"ended {ended if ended is None else round(ended, 3)} s after the last heartbeat")
    for f in failures:
        print(f"rawcheck: FAIL {f}")
    if not failures:
        print("rawcheck: PASS")
    return 1 if failures else 0


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=["info", "get", "dump", "stats", "watch", "rawcheck", "set", "layer", "reset", "bootloader"])
    ap.add_argument("args", nargs="*")
    ap.add_argument("--layer", type=int, default=None)
    ap.add_argument("--persist", action="store_true")
    ap.add_argument("--clear", action="store_true", help="stats: zero the counters after reading")
    ap.add_argument("--seconds", type=float, default=30.0)
    ap.add_argument("--device", help="hidraw node (default: find by VID:PID and serial)")
    ap.add_argument("--yes", action="store_true", help="allow commands that change the pad")
    ap.add_argument("--force", action="store_true", help="rawcheck: also on firmware before 2.0.2")
    a = ap.parse_args()

    if a.command in ("set", "layer", "reset", "bootloader") and not a.yes:
        sys.exit(f"'{a.command}' changes the pad; add --yes")
    pad = Pad(a.device or find_pad())

    if a.command == "info":
        r = pad.request(CMD_GET_INFO)
        print(f"device {pad.path}")
        print(f"magic {chr(r[2])}{chr(r[3])} format v{r[4]} slots {r[5]} eeprom {r[6]} B status {STATUS.get(r[7], r[7])}")
        print(f"firmware {r[8]}.{r[9]}.{r[10]} layers {r[11]} active {r[12]} raw {r[13]} start {r[14]}")
    elif a.command == "get":
        layers = [a.layer] if a.layer is not None else [0, 1]
        for layer in layers:
            for slot in range(24):
                r = pad.request(CMD_GET_ACTION, bytes([slot, 0, 0, 0, 0, layer]))
                st = STATUS.get(r[7], r[7])
                print(f"L{r[8]} slot {slot:2d} {slot_name(slot):12s} {TYPES.get(r[3], r[3]):8s} "
                      f"{describe(r[3], r[4], r[5] | r[6] << 8)}  [{st}]")
    elif a.command == "dump":
        data = bytearray()
        for off in range(0, 128, 12):
            r = pad.request(CMD_DUMP, bytes([off]))
            data += r[3:15]
        data = data[:128]
        for off in range(0, 128, 16):
            print(f"{off:3d}: " + " ".join(f"{b:02x}" for b in data[off:off + 16]))
    elif a.command == "stats":
        def words(r):
            idx = [3, 4, 5, 6, 8, 9, 10, 11, 12, 13, 14, 15]  # byte 7 is the status
            return [r[idx[2 * j]] | r[idx[2 * j + 1]] << 8 for j in range(6)]
        p0 = pad.request(CMD_GET_STATS, bytes([0, 0]))
        if p0[7] != 1:
            sys.exit("this firmware has no GET_STATS (2.0.0)")
        p1 = pad.request(CMD_GET_STATS, bytes([1, 1 if a.clear else 0]))
        w0, w1 = words(p0), words(p1)
        names = ("top", "middle", "bottom")
        for k in range(3):
            print(f"knob{k + 1} ({names[k]:6s}) cw {w1[2 * k]:5d}  ccw {w1[2 * k + 1]:5d}  missed-state {w0[k]:5d}")
        print(f"accumulator overruns {w0[3]}  tap-queue drops {w0[4]}  deepest tap queue {w0[5]}")
        if a.clear:
            print("counters cleared")
    elif a.command == "watch":
        # Raw mode lasts 1.5 s past each heartbeat; heartbeats go out every 0.5 s.
        print(f"raw mode on {pad.path} for {a.seconds:.0f} s; press things. Ctrl-C stops.")
        end = time.monotonic() + a.seconds
        beat = 0.0
        last_seq = None
        try:
            while time.monotonic() < end:
                if time.monotonic() >= beat:
                    pad.send(CMD_RAW_MODE, (1500).to_bytes(2, "little"))
                    beat = time.monotonic() + 0.5
                rep = pad.read(0.05)
                if rep and rep[0] == RAW_ID and len(rep) >= 5:
                    seq, slot, ev, layer = rep[1], rep[2], rep[3], rep[4]
                    count = rep[5] if len(rep) >= 6 and ev == 3 and rep[5] else 1
                    gap = "" if last_seq is None or ((last_seq + 1) & 0xFF) == seq else f"  (seq gap after {last_seq})"
                    last_seq = seq
                    times = f" x{count}" if count > 1 else ""
                    print(f"{time.strftime('%H:%M:%S')} seq {seq:3d} {slot_name(slot):12s} {EVENTS.get(ev, ev)}{times} layer {layer}{gap}",
                          flush=True)
        except KeyboardInterrupt:
            pass
        finally:
            pad.send(CMD_RAW_MODE, b"\0\0")
            print("raw mode off")
    elif a.command == "rawcheck":
        sys.exit(rawcheck(pad, a.seconds if "--seconds" in sys.argv else 6.0, a.force))
    elif a.command == "set":
        if len(a.args) != 4:
            sys.exit("set SLOT TYPE MOD CODE")
        slot, t, mod, code = (int(x, 0) for x in a.args)
        layer = a.layer or 0
        r = pad.request(CMD_SET_ACTION, bytes([slot, t, mod, code & 0xFF, code >> 8, layer]))
        print(f"set L{layer} slot {slot}: {STATUS.get(r[7], r[7])}")
    elif a.command == "layer":
        if len(a.args) != 1:
            sys.exit("layer N")
        r = pad.request(CMD_SET_LAYER, bytes([int(a.args[0], 0), 1 if a.persist else 0]))
        print(f"layer {r[8]}: {STATUS.get(r[7], r[7])}")
    elif a.command == "reset":
        r = pad.request(CMD_RESET)
        print(f"reset: {STATUS.get(r[7], r[7])}")
    elif a.command == "bootloader":
        r = pad.request(CMD_BOOTLOADER, b"BL")
        print(f"bootloader: {STATUS.get(r[7], r[7])}")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0-or-later
"""Hardware stress test for the control-surface pad, run with a person at the pad.

Pass criteria, per input mode:
  * every press has exactly one release, in order (no stuck key or hold);
  * holds from 0.1 s to 5 s are released when the key is, not before;
  * two keys at once (key 1 is on its own pin; the TM1650 reports one matrix
    key at a time, so the second key is a matrix key on another F-key);
  * every knob detent arrives exactly once: the host's count equals the
    firmware's own decoder count (GET_STATS), slow turns and fast spins;
  * raw mode: no mode flips, no heartbeat misses, no expiries on the pad.

The daemon must be running (control-surfaced run). Inputs are followed with
`control-surfaced monitor --json --identify`: reported, not dispatched (no
launchers fire, the volume does not change) while the test runs.

    scripts/stress-test.py --mode evdev     the pad's keymap ("input": "evdev")
    scripts/stress-test.py --mode raw       raw events ("input": "raw", firmware 2.0.2+)
    scripts/stress-test.py --mode both      evdev, then raw (asks for the config change)
    --quick                                 fewer holds and spins
    --report FILE                           results as JSON (default /tmp/control-surface-stress-<time>.json)
    --bin PATH                              control-surfaced to use (default: on PATH)
    --simulate                              drive mock-control-surfaced instead of a person (self-test)

Exit status: 0 all checks passed, 1 a check failed, 2 could not run.
"""

from __future__ import annotations

import argparse
import json
import queue
import shutil
import subprocess
import sys
import threading
import time

MOCK_DEST = "org.smplos.ControlSurface"
MOCK_PATH = "/org/smplos/ControlSurface/Mock"
MOCK_IFACE = "org.smplos.ControlSurface1.Mock"


class Run:
    def __init__(self, args):
        self.args = args
        self.bin = args.bin
        self.simulate = args.simulate
        self.checks: list[dict] = []
        self.events: queue.Queue = queue.Queue()
        self.monitor = None

    # ------------------------------------------------------------------ helpers
    def cli_json(self, *argv, timeout=10):
        try:
            out = subprocess.run([self.bin, *argv], capture_output=True, text=True, timeout=timeout)
        except (OSError, subprocess.TimeoutExpired) as e:
            return None, str(e)
        try:
            return json.loads(out.stdout.strip().splitlines()[-1]), None
        except (ValueError, IndexError):
            return None, (out.stderr or out.stdout).strip()

    def status(self):
        st, _ = self.cli_json("status", "--json")
        return st or {}

    def firmware_counts(self):
        """Detents the firmware decoded per knob and direction, and raw expiries."""
        if self.simulate:
            return None
        info, err = self.cli_json("firmware-info", "--json")
        if not info or not info.get("ok") or "stats" not in info:
            return None
        st = info["stats"]
        knobs = [(k["cw"], k["ccw"]) for k in st["knobs"]]
        return {"knobs": knobs, "rawExpiries": st.get("raw", {}).get("expiries")}

    def start_monitor(self):
        self.monitor = subprocess.Popen([self.bin, "monitor", "--json", "--identify"], stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, text=True)

        def pump(stream, is_err):
            for line in stream:
                line = line.strip()
                if not line:
                    continue
                if is_err:
                    if "left the bus" in line or "again" in line:
                        print(f"   (monitor: {line})")
                    continue
                try:
                    self.events.put(json.loads(line))
                except ValueError:
                    pass

        threading.Thread(target=pump, args=(self.monitor.stdout, False), daemon=True).start()
        threading.Thread(target=pump, args=(self.monitor.stderr, True), daemon=True).start()
        time.sleep(0.5)
        if self.monitor.poll() is not None:
            sys.exit("monitor did not start: is the daemon running (control-surfaced run)?")

    def stop_monitor(self):
        if self.monitor and self.monitor.poll() is None:
            self.monitor.terminate()
            self.monitor.wait(5)

    def drain(self):
        out = []
        while True:
            try:
                out.append(self.events.get_nowait())
            except queue.Empty:
                return out

    def mock(self, method, *args):
        cmd = ["gdbus", "call", "--session", "--dest", MOCK_DEST, "--object-path", MOCK_PATH,
               "--method", f"{MOCK_IFACE}.{method}", "--", *[str(a) for a in args]]
        subprocess.run(cmd, capture_output=True, check=True)

    def step(self, prompt, simulate=None):
        """Ask the person to do something (or simulate it); return the events it produced."""
        self.drain()
        if self.simulate:
            print(f"   [sim] {prompt}")
            if simulate:
                simulate()
            time.sleep(0.4)
        else:
            input(f"\n>> {prompt}\n   Press Enter when done. ")
            time.sleep(0.3)  # the last release on its way
        return self.drain()

    def check(self, mode, name, ok, detail="", warn=False):
        status = "PASS" if ok else ("WARN" if warn else "FAIL")
        self.checks.append({"mode": mode, "check": name, "status": status, "detail": detail})
        print(f"   {status:4s}  {name}{': ' + detail if detail else ''}")

    # -------------------------------------------------------------------- steps
    def taps(self, mode, keys):
        names = [f"key{i}" for i in range(1, keys + 1)]
        ev = self.step(f"Tap every key once, key1 to key{keys}, in order (left to right, top row first).",
                       lambda: [self.mock("Press", n) for n in names])
        want = [(n, e) for n in names for e in ("press", "release")]
        got = [(e["slot"], e["event"]) for e in ev]
        self.check(mode, "taps: one press and one release per key, in order", got == want,
                   "" if got == want else f"got {got}")

    def hold(self, mode, key, seconds):
        def sim():
            self.mock("Hold", key)
            time.sleep(seconds)
            self.mock("Release", key)
        ev = self.step(f"Hold {key} for about {seconds:g} s, then release it.", sim)
        got = [(e["slot"], e["event"]) for e in ev]
        ok = got == [(key, "press"), (key, "release")]
        held = (ev[1]["ms"] - ev[0]["ms"]) / 1000 if ok else 0
        # Not released early: at least half the hold (people are not clocks).
        early = ok and held < 0.5 * seconds - 0.05
        self.check(mode, f"hold {key} {seconds:g} s: press, then release when let go",
                   ok and not early, f"held {held:.2f} s" if ok else f"got {got}")

    def simultaneous(self, mode, other):
        # key1 is on its own pin; the TM1650 reports one matrix key at a time,
        # so the second key is a matrix key. In evdev mode the two must not
        # share an F-key (layer 0: keyN, keyN+6 and keyN+12 do): key2 is F15
        # and key8 Shift+F15 against key1's F14.
        def sim():
            self.mock("Hold", "key1")
            self.mock("Press", other)
            self.mock("Press", other)
            self.mock("Release", "key1")
        ev = self.step(f"Hold key1. While holding it, tap {other} twice. Then release key1.", sim)
        got = [(e["slot"], e["event"]) for e in ev]
        want = [("key1", "press"), (other, "press"), (other, "release"), (other, "press"), (other, "release"),
                ("key1", "release")]
        self.check(mode, f"key1 held while {other} is tapped twice", got == want, "" if got == want else f"got {got}")

    def knob_presses(self, mode, knobs):
        names = [f"knob{k}" for k in range(1, knobs + 1)]
        ev = self.step(f"Press each knob three times: {', '.join(names)} (press straight down, do not turn).",
                       lambda: [self.mock("Press", n) for n in names for _ in range(3)])
        got = [(e["slot"], e["event"]) for e in ev]
        want = [(n, e) for n in names for _ in range(3) for e in ("press", "release")]
        self.check(mode, "knob presses: three press/release pairs each, no turns", got == want,
                   "" if got == want else f"got {got}")

    def turns(self, mode, knob, label, prompt, sim_steps):
        before = self.firmware_counts()
        ev = self.step(prompt, lambda: [self.mock("Turn", f"knob{knob}", d) for d in sim_steps])
        after = self.firmware_counts()
        mine = [e for e in ev if e["slot"] == f"knob{knob}"]
        cw = sum(1 for e in mine if e["event"] == "cw")
        ccw = sum(1 for e in mine if e["event"] == "ccw")
        other = [(e["slot"], e["event"]) for e in ev if e["slot"] != f"knob{knob}" or e["event"] not in ("cw", "ccw")]
        if before and after:
            fcw = (after["knobs"][knob - 1][0] - before["knobs"][knob - 1][0]) & 0xFFFF
            fccw = (after["knobs"][knob - 1][1] - before["knobs"][knob - 1][1]) & 0xFFFF
            source = "the firmware decoded"
        else:
            fcw = sum(d for d in sim_steps if d > 0)
            fccw = -sum(d for d in sim_steps if d < 0)
            source = "simulated"
        ok = (cw, ccw) == (fcw, fccw) and not other
        self.check(mode, f"knob{knob} {label}: every detent once", ok,
                   f"host cw {cw} ccw {ccw}, {source} cw {fcw} ccw {fccw}" + (f", other events {other}" if other else ""))

    # --------------------------------------------------------------------- runs
    def run_mode(self, mode):
        print(f"\n=== {mode} mode ===")
        st = self.status()
        if not st.get("daemon"):
            sys.exit("no daemon on the session bus (control-surfaced run)")
        inp = st.get("input") or {}
        keys = len((st.get("layout") or {}).get("keys", [])) or 15
        knobs = len((st.get("layout") or {}).get("knobs", [])) or 3
        if not self.simulate:
            want = "raw" if mode == "raw" else "evdev-chords"
            fw = ((st.get("device") or {}).get("firmware") or {}).get("version", "?")
            self.check(mode, "daemon input mode", inp.get("mode") == want, f"{inp.get('mode')} (firmware {fw})")
            if inp.get("mode") != want:
                return
        diag0 = inp.get("raw") or {}
        evdev0 = inp.get("evdev") or {}
        fw0 = self.firmware_counts()

        self.taps(mode, keys)
        holds = [("key1", 0.1), ("key1", 1), ("key1", 5), ("key8", 2), ("key15", 3)]
        if not self.args.quick:
            holds += [("key3", 0.3), ("key12", 4), ("key6", 0.1), ("key6", 0.1)]
        for key, sec in holds:
            self.hold(mode, key, sec)
        self.simultaneous(mode, "key2")
        self.simultaneous(mode, "key8")
        self.knob_presses(mode, knobs)
        for k in range(1, knobs + 1):
            self.turns(mode, k, "slow cw", f"Turn knob{k} slowly clockwise, one full turn (about 20 clicks).", [20])
            self.turns(mode, k, "slow ccw", f"Turn knob{k} slowly counter-clockwise, one full turn.", [-20])
            self.turns(mode, k, "fast spins", f"Spin knob{k} fast, back and forth, for about 5 seconds.", [30, -25, 17, -40])
            if not self.args.quick:
                self.turns(mode, k, "flick", f"Flick knob{k} as fast as you can, one direction, two or three times.", [45, 45])

        st = self.status()
        inp = st.get("input") or {}
        diag1 = inp.get("raw") or {}
        evdev1 = inp.get("evdev") or {}
        fw1 = self.firmware_counts()
        if self.simulate:
            return
        d = {k: diag1.get(k, 0) - diag0.get(k, 0) for k in diag1}
        if mode == "raw":
            for k in ("modeFlips", "rawDrops", "heartbeatMisses"):
                self.check(mode, f"raw mode: {k} during the run", d.get(k, 0) == 0, f"{d.get(k, 0)}")
            self.check(mode, "raw mode: keymap input while raw was on", evdev1.get("whileRaw", 0) == evdev0.get("whileRaw", 0),
                       f"{evdev1.get('whileRaw', 0) - evdev0.get('whileRaw', 0)}")
            if fw0 and fw1 and fw0.get("rawExpiries") is not None:
                exp = fw1["rawExpiries"] - fw0["rawExpiries"]
                self.check(mode, "raw mode: expiries on the pad", exp == 0, f"{exp}")
            repaired = sum(d.get(k, 0) for k in ("reconciledDowns", "reconciledUps", "reconciledDetents"))
            self.check(mode, "raw mode: nothing had to be restored", repaired == 0 and d.get("seqGaps", 0) == 0,
                       f"seq gaps {d.get('seqGaps', 0)}, lost {d.get('lostEvents', 0)}, restored {repaired}", warn=True)
        else:
            self.check(mode, "evdev mode stayed on the keymap", inp.get("mode") == "evdev-chords", f"{inp.get('mode')}")

    def main(self):
        if self.simulate and not shutil.which("gdbus"):
            sys.exit("--simulate needs gdbus")
        modes = ["evdev", "raw"] if self.args.mode == "both" else [self.args.mode]
        self.start_monitor()
        try:
            for i, mode in enumerate(modes):
                if i and not self.simulate:
                    input('\n>> Now set "input": "raw" in the "device" section of ~/.config/control-surface/config.jsonc\n'
                          "   and save; the daemon applies it at once. Press Enter when done. ")
                    for _ in range(20):
                        if (self.status().get("input") or {}).get("mode") == "raw":
                            break
                        time.sleep(0.5)
                self.run_mode(mode)
        finally:
            self.stop_monitor()
        failed = [c for c in self.checks if c["status"] == "FAIL"]
        warned = [c for c in self.checks if c["status"] == "WARN"]
        report = self.args.report or time.strftime("/tmp/control-surface-stress-%Y%m%d-%H%M%S.json")
        with open(report, "w") as fh:
            json.dump({"modes": modes, "checks": self.checks, "passed": not failed}, fh, indent=2)
        print(f"\n{len(self.checks)} checks: {len(self.checks) - len(failed) - len(warned)} passed, "
              f"{len(warned)} warnings, {len(failed)} failed. Report: {report}")
        return 1 if failed else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--mode", choices=["evdev", "raw", "both"], default="both")
    ap.add_argument("--quick", action="store_true")
    ap.add_argument("--report")
    ap.add_argument("--bin", default=shutil.which("control-surfaced") or "control-surfaced")
    ap.add_argument("--simulate", action="store_true")
    args = ap.parse_args()
    return Run(args).main()


if __name__ == "__main__":
    sys.exit(main())

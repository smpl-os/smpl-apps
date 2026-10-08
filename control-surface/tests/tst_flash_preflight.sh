#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# flash-and-verify.sh refuses to start while a control-surfaced reads the pad
# raw (or would, with "auto"). A fake gdbus answers GetStatus; WCHISP is
# /bin/false and ENTER_BOOTLOADER is off, so nothing can ever be flashed.
set -u
SCRIPT=$1
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
printf 'image' > "$T/img.bin"; SHA=$(sha256sum "$T/img.bin" | cut -d' ' -f1)
mkdir "$T/bin"
fail=0
run() {  # STATUS-JSON|"" EXPECTED-RC
    if [ -n "$1" ]; then
        printf "('%s',)\n" "$1" > "$T/reply"
        printf '#!/bin/sh\ncat "%s"\n' "$T/reply" > "$T/bin/gdbus"
    else
        printf '#!/bin/sh\nexit 1\n' > "$T/bin/gdbus"   # no daemon
    fi
    chmod +x "$T/bin/gdbus"
    PATH="$T/bin:$PATH" WCHISP=/bin/false ENTER_BOOTLOADER=0 timeout 2 \
        bash "$SCRIPT" "$T/img.bin" "$SHA" "$T/log" >"$T/out" 2>&1
    rc=$?
    if [ "$rc" != "$2" ]; then echo "FAIL: ${1:-no daemon}: rc $rc, want $2"; cat "$T/out"; fail=1; fi
}
run '{"input": {"configured": "raw", "mode": "raw"}}' 8
run '{"input": {"configured": "auto", "mode": "raw"}}' 8
run '{"input": {"configured": "auto", "mode": "evdev-chords"}}' 8   # raw once the new firmware answers
run '{"input": {"configured": "evdev", "mode": "evdev-chords"}}' 124 # passes, then waits for the bootloader
grep -q "waiting up to 300 s" "$T/out" || { echo "FAIL: evdev did not get to the bootloader wait"; fail=1; }
run '{"input": null, "mode": "mock"}' 124
run '' 124
exit $fail

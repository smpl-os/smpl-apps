#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
#
# One-step flash and verify for the control-surface pad firmware.
#
# Needs the user at the pad: hold the TOP-LEFT key (P1.5, the CH552 ROM's
# download pin) while plugging the pad in. This script waits for that fresh
# bootloader session, flashes once, waits for the new firmware to enumerate,
# and runs the read-only protocol checks. It never writes the chip's config
# registers (wchisp flash = erase, write, verify, reset of code flash only)
# and it never touches any other USB device.
#
#   flash-and-verify.sh IMAGE SHA256 LOGDIR
#
# Environment: WCHISP (default /mnt/ai/keypad-lab/tools/wchisp/bin/wchisp).
# Exit codes: 0 ok, 2 usage, 3 hash mismatch, 4 no bootloader within 300 s,
# 5 flash failed, 6 pad did not come back, 7 protocol check failed.

set -u
IMAGE=${1:-}; SHA=${2:-}; LOG=${3:-}
[ -n "$IMAGE" ] && [ -n "$SHA" ] && [ -n "$LOG" ] || { echo "usage: $0 IMAGE SHA256 LOGDIR"; exit 2; }
WCHISP=${WCHISP:-/mnt/ai/keypad-lab/tools/wchisp/bin/wchisp}
HERE=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$LOG"
exec > >(tee -a "$LOG/flash-and-verify.log") 2>&1

stamp() { date -Iseconds; }
usbdev() {  # print devnum of the first device matching VID:PID [serial]
    local d
    for d in /sys/bus/usb/devices/*; do
        [ -f "$d/idVendor" ] || continue
        [ "$(cat "$d/idVendor"):$(cat "$d/idProduct")" = "$1" ] || continue
        if [ -n "${2:-}" ]; then [ "$(cat "$d/serial" 2>/dev/null)" = "$2" ] || continue; fi
        echo "$(cat "$d/devnum") $d"; return 0
    done
    return 1
}

echo "$(stamp) image $IMAGE"
GOT=$(sha256sum "$IMAGE" | cut -d' ' -f1)
[ "$GOT" = "$SHA" ] || { echo "hash mismatch: got $GOT, want $SHA"; exit 3; }
echo "$(stamp) sha256 ok $GOT ($(stat -c %s "$IMAGE") bytes)"

# A bootloader session that already answered something can wedge; only use a
# session that appears after this script started.
OLD=$(usbdev 4348:55e0 | cut -d' ' -f1)
[ -n "$OLD" ] && echo "$(stamp) ignoring existing bootloader session devnum $OLD; replug with the top-left key held"
echo "$(stamp) waiting up to 300 s: hold TOP-LEFT key and plug the pad in"
for _ in $(seq 1 3000); do
    CUR=$(usbdev 4348:55e0 | cut -d' ' -f1)
    if [ -n "$CUR" ] && [ "$CUR" != "$OLD" ]; then break; fi
    CUR=""; sleep 0.1
done
[ -n "$CUR" ] || { echo "$(stamp) no new bootloader session"; exit 4; }
echo "$(stamp) bootloader session devnum $CUR"
sleep 0.3
"$WCHISP" flash "$IMAGE"; RC=$?
echo "$(stamp) wchisp flash rc=$RC"
[ $RC -eq 0 ] || exit 5

echo "$(stamp) waiting for 1189:8890 serial key153 (release the top-left key now)"
for _ in $(seq 1 300); do
    PAD=$(usbdev 1189:8890 key153) && break
    sleep 0.1
done
[ -n "${PAD:-}" ] || { echo "$(stamp) pad did not enumerate in 30 s; release all keys and replug"; exit 6; }
SYS=$(echo "$PAD" | cut -d' ' -f2)
echo "$(stamp) enumerated: $(cat "$SYS/manufacturer" 2>/dev/null) / $(cat "$SYS/product" 2>/dev/null)" \
     "bcdDevice $(cat "$SYS/bcdDevice") serial $(cat "$SYS/serial")"
sleep 1.5   # first boot writes the default keymap to data flash

FAIL=0
python3 "$HERE/padctl.py" info | tee "$LOG/info.txt" || FAIL=1
python3 "$HERE/padctl.py" get | tee "$LOG/keymap.txt" || FAIL=1
python3 "$HERE/padctl.py" dump | tee "$LOG/dataflash.txt" || FAIL=1
grep -q "magic CS format v3 slots 24" "$LOG/info.txt" || { echo "unexpected GET_INFO"; FAIL=1; }
grep -q "firmware 2.0.0 layers 2 active 0 raw 0 start 0" "$LOG/info.txt" || { echo "unexpected firmware state"; FAIL=1; }
[ "$(grep -c '\[ok\]' "$LOG/keymap.txt")" = 48 ] || { echo "keymap read incomplete"; FAIL=1; }
grep -q "L0 slot  0 key1         key      key usage=0x69 mods=0x00" "$LOG/keymap.txt" || { echo "layer 0 slot 0 is not F14"; FAIL=1; }
grep -q "L0 slot 23 knob3.cw     key      key usage=0x6e mods=0x04" "$LOG/keymap.txt" || { echo "layer 0 slot 23 is not alt+F19"; FAIL=1; }
[ $FAIL -eq 0 ] || exit 7
echo "$(stamp) PROTOCOL CHECKS PASSED. Next: the press capture (all 24 inputs, several rounds)."

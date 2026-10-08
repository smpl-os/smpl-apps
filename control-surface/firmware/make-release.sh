#!/bin/bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Build the pad firmware twice, check the builds are identical, and write
# release/<name>.bin plus release/<name>.json (name, version, board, licence,
# sha256 ...). smplOS installs these to /usr/share/control-surface/firmware/.
# Never flashes anything.
#
#   make-release.sh [WORKDIR]     (default: a temporary directory)
#
# Needs sdcc in PATH (e.g. PATH=/mnt/ai/keypad-lab/tools/sdcc-4.5.0/bin:$PATH).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=${1:-$(mktemp -d)}
mkdir -p "$WORK"
BOARD=sy181-15k3e
VERSION=$(sed -n 's/^#define FW_VERSION_MAJOR *\([0-9]*\).*/\1/p' "$HERE/src/padstore.h").$(sed -n 's/^#define FW_VERSION_MINOR *\([0-9]*\).*/\1/p' "$HERE/src/padstore.h").$(sed -n 's/^#define FW_VERSION_PATCH *\([0-9]*\).*/\1/p' "$HERE/src/padstore.h")
NAME=control-surface-$BOARD-$VERSION
SDCC="SDCC $(sdcc --version | head -1 | grep -o '[0-9]*\.[0-9]*\.[0-9]* #[0-9]*')"

python3 "$HERE/build.py" --out "$WORK/a" padfw.c > "$WORK/build-a.log"
python3 "$HERE/build.py" --out "$WORK/b" padfw.c > "$WORK/build-b.log"
A=$(sha256sum "$WORK/a/padfw.bin" | cut -d' ' -f1)
B=$(sha256sum "$WORK/b/padfw.bin" | cut -d' ' -f1)
[ "$A" = "$B" ] || { echo "builds differ: $A vs $B"; exit 1; }

mkdir -p "$HERE/release"
cp "$WORK/a/padfw.bin" "$HERE/release/$NAME.bin"
SIZE=$(stat -c %s "$HERE/release/$NAME.bin")
USED=$(sed -n 's/^FLASH: \([0-9]*\) .*/\1/p' "$WORK/build-a.log")
SRC=$(git -C "$HERE" log -1 --format=%h -- src padfw.c build.py 2>/dev/null || echo unknown)
cat > "$HERE/release/$NAME.json" <<JSON
{
  "name": "$NAME",
  "version": "$VERSION",
  "board": "$BOARD",
  "boardName": "CH552G + TM1650, 15 keys, 3 knobs (SY181 style)",
  "usb": "1189:8890",
  "serial": "key153",
  "protocol": 3,
  "slots": 24,
  "layers": 2,
  "license": "CC-BY-SA-3.0",
  "licenseFile": "LICENSE",
  "attribution": "Fork of EpicLPer's CH552-OpenMacroPad, on Stefan Wagner's (wagiminator) CH55x USB HID code",
  "sha256": "$A",
  "size": $SIZE,
  "codeBytes": $USED,
  "codeFlashBytes": 14336,
  "source": "firmware/ at commit $SRC",
  "toolchain": "$SDCC",
  "flash": "wchisp flash <file> (ROM bootloader 4348:55e0; config registers untouched)",
  "verifiedOnHardware": false,
  "knownIssues": [],
  "notes": "Built reproducibly and host-tested (fw_logic, fw_store); not yet flashed to a pad."
}
JSON
cp "$HERE/LICENSE.upstream" "$HERE/release/LICENSE"
echo "$NAME.bin  $A  ($SIZE bytes)"

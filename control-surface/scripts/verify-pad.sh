#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-or-later
# One-command interactive check of the programmed pad: prompts for each of the
# 24 inputs (15 keys, 3 knobs x ccw/cw/press), shows the chord each one emits,
# compares with the flashed scheme and writes ~/.config/control-surface/hardware-map.json.
# The pad is grabbed while this runs, so its keys reach nothing else.
set -euo pipefail
src="$(cd "$(dirname "$0")/.." && pwd)"
bin="$(command -v control-surfaced || true)"
for b in /mnt/ai/keypad-lab/build/control-surface/control-surfaced "${XDG_CACHE_HOME:-$HOME/.cache}/control-surface-build/control-surfaced"; do
    [ -z "$bin" ] && [ -x "$b" ] && bin="$b"
done
if [ -z "$bin" ]; then
    echo "control-surfaced not built yet: running scripts/install-user.sh first"
    "$src/scripts/install-user.sh"
    bin="$HOME/.local/bin/control-surfaced"
fi
restart=0
if systemctl --user is-active --quiet control-surface.service 2>/dev/null; then
    echo "stopping control-surface.service while verifying (it holds the pad)"
    systemctl --user stop control-surface.service
    restart=1
fi
log_dir="${XDG_STATE_HOME:-$HOME/.local/state}/control-surface"
mkdir -p "$log_dir"
log="$log_dir/verify-$(date +%Y%m%d-%H%M%S).log"
set +e
"$bin" verify "$@" 2>&1 | tee "$log"
rc=${PIPESTATUS[0]}
set -e
[ "$restart" = 1 ] && systemctl --user start control-surface.service
echo "transcript: $log"
exit "$rc"

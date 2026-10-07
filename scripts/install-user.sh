#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-2.0-or-later
# Build (out of tree, on the AI drive if present) and install into ~/.local plus
# the systemd --user unit. Never enables or starts the service.
set -euo pipefail
src="$(cd "$(dirname "$0")/.." && pwd)"
if mountpoint -q /mnt/ai 2>/dev/null; then build=/mnt/ai/keypad-lab/build/control-surface; else build="${XDG_CACHE_HOME:-$HOME/.cache}/control-surface-build"; fi
cmake -S "$src" -B "$build" -G Ninja -DCMAKE_BUILD_TYPE=RelWithDebInfo -DCMAKE_INSTALL_PREFIX="$HOME/.local" >/dev/null
cmake --build "$build" -j"$(nproc)"
ctest --test-dir "$build" --output-on-failure
cmake --install "$build" >/dev/null
install -Dm644 "$src/data/systemd/control-surface.service" "$HOME/.config/systemd/user/control-surface.service"
systemctl --user daemon-reload
if [ ! -e "$HOME/.config/control-surface/config.jsonc" ]; then
    install -Dm644 "$src/data/config.example.jsonc" "$HOME/.config/control-surface/config.jsonc"
    echo "installed example config: ~/.config/control-surface/config.jsonc"
fi
echo "installed ~/.local/bin/control-surfaced, ~/.local/bin/ch552-padprog and the (disabled) user unit."
echo "next: $src/scripts/verify-pad.sh, then: systemctl --user enable --now control-surface.service"

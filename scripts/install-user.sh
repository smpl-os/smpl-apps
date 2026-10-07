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
cfg="$HOME/.config/control-surface/config.jsonc"
if [ ! -e "$cfg" ]; then
    install -Dm644 "$src/data/config.example.jsonc" "$cfg"
    echo "installed example config: ~/.config/control-surface/config.jsonc"
elif grep -qx "$(sha256sum "$cfg" | cut -d' ' -f1)" "$src/data/config.example.previous.sha256" 2>/dev/null; then
    # An unmodified earlier example: upgrade it, keeping a backup.
    cp -p "$cfg" "$cfg.bak-$(date +%Y%m%d-%H%M%S)"
    install -m644 "$src/data/config.example.jsonc" "$cfg"
    echo "upgraded unmodified example config (backup kept next to it)"
else
    echo "kept your config; compare with $src/data/config.example.jsonc for new options"
fi
echo "installed ~/.local/bin/control-surfaced, ~/.local/bin/ch552-padprog and the (disabled) user unit."
echo "next: $src/scripts/verify-pad.sh, then: systemctl --user enable --now control-surface.service"

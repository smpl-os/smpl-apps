#!/bin/bash
# Deploy built binaries to the canonical system location.
#
# Usage: ./deploy-local.sh [--build]
#   --build   run cargo build --release first
#
# Historically this script installed each binary to BOTH ~/.local/share/smplos/bin/
# AND /usr/local/bin/. That directly caused the "I updated but the tray icon is
# still running the OLD code" trap: envs.conf prepends ~/.local/share/smplos/bin
# to PATH, so Eww's `command -v smpl-calendar` resolves there first — and every
# partial deploy left the two copies out of sync. We now install ONLY to
# /usr/local/bin (canonical), and actively DELETE any leftover shadow copies
# from ~/.local/share/smplos/bin and ~/.local/bin so PATH lookup can no longer
# resolve to something stale.

set -e

RELEASE="$(dirname "$0")/target/release"

if [[ "$1" == "--build" ]]; then
    export PATH="$HOME/.cargo/bin:$PATH"
    echo ":: Building release..."
    cargo build --release --manifest-path "$(dirname "$0")/Cargo.toml"
fi

BINS=(settings start-menu notif-center app-center webapp-center sync-center-gui smpl-calendar)

echo ":: Deploying binaries to /usr/local/bin (canonical location only)..."
for bin in "${BINS[@]}"; do
    src="$RELEASE/$bin"
    [[ -f "$src" ]] || continue

    # Install to /usr/local/bin — the ONLY canonical location. Any PATH-earlier
    # shadow (~/.local/share/smplos/bin, ~/.local/bin, ~/bin) is scrubbed below.
    if sudo install -m755 "$src" "/usr/local/bin/$bin"; then
        echo "  $bin -> /usr/local/bin/"
    else
        echo "  !! $bin -> /usr/local/bin/ FAILED"
        continue
    fi

    # Single-copy invariant: nuke shadows so PATH lookup can never resolve to
    # a stale binary. Same policy sync_apps enforces on every OS update.
    for shadow in \
        "$HOME/.local/share/smplos/bin/$bin" \
        "$HOME/.local/bin/$bin" \
        "$HOME/bin/$bin"
    do
        if [[ -e "$shadow" || -L "$shadow" ]]; then
            rm -f "$shadow" 2>/dev/null && echo "  (removed shadow: $shadow)"
        fi
    done
done

# Re-export settings search index and rebuild app cache
if [[ -f "$RELEASE/settings" ]]; then
    "$RELEASE/settings" --export-index 2>/dev/null && \
        echo ":: Settings search index exported"
fi
rebuild-app-cache 2>/dev/null && echo ":: App cache rebuilt" || echo "  (rebuild-app-cache not found, skipping)"

# Kill running instances so the new binaries take effect
for bin in "${BINS[@]}"; do
    pkill -f "^/usr/local/bin/$bin\$" 2>/dev/null || true
    pkill -x "$bin" 2>/dev/null || true
done
echo ":: Old processes killed — new versions will load on next launch"

# Show versions
echo ":: Deployed versions:"
for bin in "${BINS[@]}"; do
    [[ -f "/usr/local/bin/$bin" ]] && echo "  $(/usr/local/bin/$bin --version 2>/dev/null || /usr/local/bin/$bin -v 2>/dev/null || echo "$bin (no --version)")"
done

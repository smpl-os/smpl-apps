#!/bin/bash
# Deploy built binaries to ALL local locations.
# Usage: ./deploy-local.sh [--build]
#   --build   run cargo build --release first

set -e

RELEASE="$(dirname "$0")/target/release"

if [[ "$1" == "--build" ]]; then
    export PATH="$HOME/.cargo/bin:$PATH"
    echo ":: Building release..."
    cargo build --release --manifest-path "$(dirname "$0")/Cargo.toml"
fi

BINS=(settings start-menu notif-center app-center webapp-center sync-center-gui calendar)

echo ":: Deploying binaries..."
for bin in "${BINS[@]}"; do
    src="$RELEASE/$bin"
    [[ -f "$src" ]] || continue

    # ~/.local/share/smplos/bin/
    cp -f "$src" "$HOME/.local/share/smplos/bin/$bin" 2>/dev/null && \
        echo "  $bin -> ~/.local/share/smplos/bin/"

    # /usr/local/bin/ (needs sudo)
    if [[ -f "/usr/local/bin/$bin" ]]; then
        sudo cp -f "$src" "/usr/local/bin/$bin" && \
            echo "  $bin -> /usr/local/bin/"
    fi
done

# Re-export settings search index and rebuild app cache
if [[ -f "$RELEASE/settings" ]]; then
    "$RELEASE/settings" --export-index 2>/dev/null && \
        echo ":: Settings search index exported"
fi
rebuild-app-cache 2>/dev/null && echo ":: App cache rebuilt" || echo "  (rebuild-app-cache not found, skipping)"

# Kill running instances so the new binaries take effect
for bin in "${BINS[@]}"; do
    pkill -f "^.*/$bin\$" 2>/dev/null || true
    pkill -x "$bin" 2>/dev/null || true
done
echo ":: Old processes killed — new versions will load on next launch"

# Show versions
echo ":: Deployed versions:"
for bin in "${BINS[@]}"; do
    [[ -f "/usr/local/bin/$bin" ]] && echo "  $(/usr/local/bin/$bin -v 2>/dev/null || true)"
done

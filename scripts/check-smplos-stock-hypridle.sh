#!/usr/bin/env bash
# Guards the Settings -> Power editor against drift from the hypridle.conf that
# smplOS ships. It downloads the file smplOS ships today and runs the same
# timer-change checks that the committed copy (the test fixture) gets.
#
#   scripts/check-smplos-stock-hypridle.sh           check the current upstream file
#   scripts/check-smplos-stock-hypridle.sh --update  refresh the committed fixture
#
# The editor failing on the upstream file is an error: users would hit it. A
# committed fixture that is merely out of date only warns; refresh it with
# --update and review the diff.
set -euo pipefail

repo="smpl-os/smplos"
upstream="src/compositors/hyprland/hypr/hypridle.conf"
url="https://raw.githubusercontent.com/$repo/HEAD/$upstream"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$root/settings/src/power/fixtures/smplos-hypridle.conf"
test_name="power::tests::upstream_stock_hypridle_conf_supports_every_timer_change"

case "${1:-}" in
    "" | --update) ;;
    *)
        echo "usage: ${0##*/} [--update]" >&2
        exit 2
        ;;
esac

warn() {
    if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
        echo "::warning::$*"
    else
        echo "warning: $*" >&2
    fi
}

fetch() {
    if curl --fail --silent --show-error --location --retry 3 --output "$1" "$url"; then
        return 0
    fi
    # The API also works for a private repository when gh is logged in.
    command -v gh >/dev/null &&
        gh api "repos/$repo/contents/$upstream" --jq .content | base64 -d >"$1"
}

# Comments and blank lines may change without affecting what hypridle does.
settings_lines() {
    grep -Ev '^[[:space:]]*(#|$)' "$1" || true
}

downloaded="$(mktemp)"
trap 'rm -f "$downloaded"' EXIT

if ! fetch "$downloaded"; then
    echo "error: could not download $url" >&2
    exit 1
fi
if ! grep -Eq '^[[:space:]]*(general|listener)[[:space:]]*\{' "$downloaded"; then
    echo "error: $url did not return a hypridle.conf" >&2
    exit 1
fi

if [[ "${1:-}" == "--update" ]]; then
    install -D -m 0644 "$downloaded" "$fixture"
    echo "Updated ${fixture#"$root/"}. Review the diff, then run:"
    echo "  cargo test -p settings --bin settings power"
    exit 0
fi

if ! diff -q <(settings_lines "$fixture") <(settings_lines "$downloaded") >/dev/null; then
    warn "${fixture#"$root/"} is out of date with $repo; run ${0##*/} --update"
fi

cd "$root"
# --workspace matches CI's feature set, so its test build is reused.
log="$(mktemp)"
trap 'rm -f "$downloaded" "$log"' EXIT
if ! SMPLOS_STOCK_HYPRIDLE_CONF="$downloaded" \
    cargo test --workspace --bin settings "$test_name" -- --ignored 2>&1 | tee "$log"; then
    echo "error: Settings cannot edit the hypridle.conf that $repo ships today." >&2
    echo "       Fix the editor in settings/src/power.rs (or the OS file) before releasing." >&2
    exit 1
fi
# A renamed or removed test would otherwise pass by running nothing.
if ! grep -q 'test result: ok. 1 passed' "$log"; then
    echo "error: $test_name did not run" >&2
    exit 1
fi
echo "Settings edits the hypridle.conf that $repo ships today."

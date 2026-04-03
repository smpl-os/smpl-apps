#!/usr/bin/env bash
# ── check.sh ─────────────────────────────────────────────────────────────────
# Run the EXACT same checks that CI does, locally, before pushing.
#
# Usage:
#   ./check.sh          # quick mode: guardrails + check + clippy
#   ./check.sh --full   # full mode:  + release build + YAML lint
#   ./check.sh --release # release mode: everything that the Release workflow does
#
# Exit code 0 = safe to push/release. Non-zero = fix before pushing.
# ──────────────────────────────────────────────────────────────────────────────
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BOLD='\033[1m'
RESET='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

MODE="${1:-quick}"
FAILED=0

# ── Helpers ──────────────────────────────────────────────────────────────────

step() {
    echo -e "\n${BOLD}▶ $1${RESET}"
}

pass() {
    echo -e "  ${GREEN}✓ $1${RESET}"
}

fail() {
    echo -e "  ${RED}✗ $1${RESET}"
    FAILED=1
}

warn() {
    echo -e "  ${YELLOW}⚠ $1${RESET}"
}

# ── 1. Guardrails (transparency) ────────────────────────────────────────────

step "Guardrails: renderer-femtovg"

if grep -q 'renderer-software\|renderer-skia' Cargo.toml; then
    fail "Workspace Cargo.toml uses wrong renderer -- transparency will break!"
    echo "       The Slint feature MUST be 'renderer-femtovg', not 'renderer-software' or 'renderer-skia'."
else
    pass "Cargo.toml uses renderer-femtovg"
fi

if ! grep -q 'with_renderer_name("femtovg")' smpl-common/src/lib.rs; then
    fail "smpl-common/src/lib.rs missing .with_renderer_name(\"femtovg\")"
else
    pass "smpl-common init uses femtovg renderer"
fi

if ! grep -q 'with_decorations(false)' smpl-common/src/lib.rs; then
    fail "smpl-common/src/lib.rs missing .with_decorations(false)"
else
    pass "smpl-common disables CSD"
fi

if ! grep -q 'with_name(' smpl-common/src/lib.rs; then
    fail "smpl-common/src/lib.rs missing .with_name() for Wayland app_id"
else
    pass "smpl-common sets Wayland app_id"
fi

# ── 1b. Feature inventory guardrails ────────────────────────────────────────
# These catch accidental deletion of features during sync operations between
# the smpl-apps and smplos repos — the #1 source of regressions.

step "Feature inventory guardrails"

check_feat() {
    local label="$1" file="$2" pattern="$3"
    if ! grep -q -- "$pattern" "$file" 2>/dev/null; then
        fail "REGRESSION: $label  (expected '$pattern' in $file)"
    else
        pass "$label"
    fi
}

# webapp-center
check_feat "webapp-center: keybinding import" webapp-center/src/main.rs "use smpl_common::keybindings"
check_feat "webapp-center: hotkey callback"    webapp-center/src/main.rs "on_hotkey_start_capture"
check_feat "webapp-center: build_launch_args"  webapp-center/src/main.rs "fn build_launch_args"
check_feat "webapp-center: --clear-on-exit"    webapp-center/src/main.rs '"--clear-on-exit"'
check_feat "webapp-center: --secure flag"      webapp-center/src/main.rs '"--secure"'

# start-menu: keyboard navigation
check_feat "start-menu: FocusScope owns search focus" start-menu/ui/main.slint "search-scope.focus()"
check_feat "start-menu: search-pop-char callback"     start-menu/ui/main.slint "callback search-pop-char"
check_feat "start-menu: DownArrow calls focus-app"     start-menu/ui/main.slint "root.focus-app()"
check_feat "start-menu: type-to-search forwarding"     start-menu/ui/main.slint "root.search-text = root.search-text + event.text"
check_feat "start-menu: Rust search-pop-char"          start-menu/src/main.rs   "on_search_pop_char"

# settings: keyboard layout dropdown safety
check_feat "settings: dropdown bounds check"    settings/ui/main.slint "selected-dropdown-index < root.available-layouts.length"
check_feat "settings: index reset before model" settings/src/main.rs   "set_selected_dropdown_index(-1)"

# ── 2. YAML lint (CI workflow files) ────────────────────────────────────────

step "YAML lint: CI workflow files"

yaml_ok=true
for f in .github/workflows/*.yml; do
    if [[ ! -f "$f" ]]; then
        warn "No workflow files found"
        break
    fi

    # Check 1: python yaml.safe_load (catches structural errors)
    if command -v python3 &>/dev/null; then
        if ! python3 -c "import yaml; yaml.safe_load(open('$f'))" 2>/dev/null; then
            fail "$f has invalid YAML"
            yaml_ok=false
            continue
        fi
    fi

    # Check 2: All step items under jobs.*.steps must be indented (not at column 0)
    # This catches the exact bug that broke CI #16-#21
    if grep -nP '^- name:' "$f" | head -1 | grep -q .; then
        fail "$f has a step at column 0 (must be indented under steps:)"
        grep -nP '^- name:' "$f"
        yaml_ok=false
        continue
    fi

    pass "$f"
done

if $yaml_ok; then
    pass "All workflow YAML files are valid"
fi

# ── 3. cargo check ──────────────────────────────────────────────────────────

step "cargo check --workspace"

if cargo check --workspace 2>&1; then
    pass "All workspace members pass cargo check"
else
    fail "cargo check failed"
fi

# ── 4. cargo clippy (same flags as CI: -D warnings) ─────────────────────────

step "cargo clippy --workspace -- -D warnings"

if cargo clippy --workspace -- -D warnings 2>&1; then
    pass "No clippy warnings"
else
    fail "clippy found warnings (CI uses -D warnings, these WILL fail CI)"
fi

# ── 5. Full release build (optional) ────────────────────────────────────────

if [[ "$MODE" == "--full" || "$MODE" == "--release" ]]; then
    step "cargo build --release --workspace"

    if cargo build --release --workspace 2>&1; then
        pass "Release build succeeded"
    else
        fail "Release build failed"
    fi
fi

# ── 6. Release-specific checks ──────────────────────────────────────────────

if [[ "$MODE" == "--release" ]]; then
    step "Release checks: binary existence"

    for bin in start-menu notif-center settings app-center webapp-center \
               sync-center-daemon sync-center-gui \
               smpl-calendar smpl-calendar-alertd; do
        if [[ -f "target/release/$bin" ]]; then
            size=$(du -h "target/release/$bin" | cut -f1)
            pass "$bin ($size)"
        else
            warn "$bin not found in target/release/ (may not be a [[bin]] target)"
        fi
    done
fi

# ── Summary ──────────────────────────────────────────────────────────────────

echo ""
if [[ $FAILED -eq 0 ]]; then
    echo -e "${GREEN}${BOLD}━━━ All checks passed ━━━${RESET}"
    if [[ "$MODE" != "--full" && "$MODE" != "--release" ]]; then
        echo -e "    Run ${YELLOW}./check.sh --full${RESET} to also do a release build"
        echo -e "    Run ${YELLOW}./check.sh --release${RESET} for the full release pipeline"
    fi
    exit 0
else
    echo -e "${RED}${BOLD}━━━ Some checks FAILED ━━━${RESET}"
    echo -e "    Fix the issues above before pushing."
    exit 1
fi

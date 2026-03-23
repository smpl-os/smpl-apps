#!/usr/bin/env bash
# ── setup-hooks.sh ───────────────────────────────────────────────────────────
# Configure git to use .githooks/ for hooks (checked into the repo).
# Run once after cloning.
# ──────────────────────────────────────────────────────────────────────────────
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

git config core.hooksPath .githooks
chmod +x .githooks/*

echo "✓ Git hooks installed (.githooks/)"
echo "  Pre-push hook will run ./check.sh before every push."
echo "  Bypass with: git push --no-verify"

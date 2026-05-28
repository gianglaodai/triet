#!/usr/bin/env bash
# install-hooks.sh — Point git at .githooks/ via core.hooksPath.
#
# This makes the tracked .githooks/ directory the active hook location for
# this clone. Run once after cloning. The setting is per-clone (local config),
# so each contributor must run it themselves.
#
# Why not just symlink into .git/hooks/?
# - .git/hooks/ is not tracked. Symlinks per-developer break on Windows.
# - core.hooksPath is a cleaner mechanism (git 2.9+, ubiquitous since 2016).
#
# Verify after install:
#   git config --get core.hooksPath        # should print: .githooks

set -e
cd "$(dirname "$0")/.."

git config core.hooksPath .githooks
chmod +x .githooks/pre-commit .githooks/pre-push 2>/dev/null || true

echo "✓ Git hooks installed. Hooks directory: .githooks/"
echo ""
echo "Active hooks:"
echo "  pre-commit  — cargo fmt --check (~0.5s, every commit)"
echo "  pre-push    — full ADR-0009 gate B (~1 min, blocks dirty push)"
echo ""
echo "To bypass occasionally (NOT recommended for shared branches):"
echo "  git commit --no-verify"
echo "  git push --no-verify"

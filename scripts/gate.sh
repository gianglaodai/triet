#!/usr/bin/env bash
set -euo pipefail

echo "=== build warnings ==="
cargo build --workspace 2>&1 | grep -c "warning:" || echo "0"

echo "=== test failures ==="
cargo test --workspace 2>&1 | grep -E "test result|FAILED" | tail -20

echo "=== fixtures ==="
cargo test -p triet-driver --test integration_tests -- --nocapture 2>&1 | grep -c "PASS" || echo "0"

echo "=== clippy locations ==="
cargo clippy --workspace --all-targets 2>&1 | grep -- "-->" | sort -u | wc -l

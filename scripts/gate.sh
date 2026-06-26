#!/usr/bin/env bash
# Gate — pre-commit verdict for the Triết workspace.
#
# Honest exit code (ADR-0071 infra WO, "Kỷ Luật Gate"):
#   exit 0  ⟺  fully clean: 0 build warnings · cargo build/test OK · 0 test
#              FAILED · fixtures all-pass · 0 clippy locations.
#   exit 1  ⟺  any REAL problem above.
#
# We deliberately DROP `set -e`: the old script used `set -euo pipefail` and a
# trailing `grep -- "-->" | … | wc -l`. When clippy is CLEAN, that grep finds
# nothing and exits 1; pipefail propagates it; being the last line, the whole
# script exited 1 — a clean tree read as a RED gate. The fix is an EXPLICIT
# verdict, not silent `|| true` everywhere (which would also swallow a genuine
# cargo build/test failure). Cargo's own exit codes ARE captured and respected.
set -uo pipefail

fail=0

# ── build warnings ─────────────────────────────────────────────
# `cargo build` returns 0 even with warnings, so check BOTH the hard exit code
# (compile error) AND the warning count.
echo "=== build warnings ==="
build_out=$(cargo build --workspace 2>&1)
build_rc=$?
warn_count=$(printf '%s\n' "$build_out" | grep -c "warning:" || true)
echo "$warn_count"
[ "$build_rc" -ne 0 ] && { echo "  (cargo build exit=$build_rc)"; fail=1; }
[ "$warn_count" -ne 0 ] && fail=1

# ── test failures ──────────────────────────────────────────────
# Respect cargo test's real exit code (nonzero ⟺ a test failed) AND grep the
# FAILED markers for the human-readable display the reports paste verbatim.
echo "=== test failures ==="
test_out=$(cargo test --workspace 2>&1)
test_rc=$?
printf '%s\n' "$test_out" | grep -E "test result|FAILED" | tail -20 || true
failed_count=$(printf '%s\n' "$test_out" | grep -c "FAILED" || true)
[ "$test_rc" -ne 0 ] && fail=1
[ "$failed_count" -ne 0 ] && fail=1

# ── fixtures ───────────────────────────────────────────────────
# The integration corpus is a single test; a fixture failing makes cargo test
# exit nonzero. Display the PASS count; honour the exit code.
echo "=== fixtures ==="
fix_out=$(cargo test -p triet-driver --test integration_tests -- --nocapture 2>&1)
fix_rc=$?
pass_count=$(printf '%s\n' "$fix_out" | grep -c "PASS" || true)
echo "$pass_count"
[ "$fix_rc" -ne 0 ] && fail=1

# ── clippy locations ───────────────────────────────────────────
# Workspace lints are strict; any location ⟹ red. The grep-no-match here is the
# ORIGINAL bug — `tr -d` normalises `wc -l` whitespace so the integer test is
# robust, and the missing `set -e` means a no-match no longer aborts the run.
echo "=== clippy locations ==="
clippy_out=$(cargo clippy --workspace --all-targets 2>&1)
clippy_rc=$?
clippy_count=$(printf '%s\n' "$clippy_out" | grep -- "-->" | sort -u | wc -l | tr -d '[:space:]')
echo "$clippy_count"
[ "$clippy_rc" -ne 0 ] && fail=1
[ "$clippy_count" -ne 0 ] && fail=1

# ── verdict ────────────────────────────────────────────────────
if [ "$fail" -eq 0 ]; then
  echo "=== verdict: CLEAN (exit 0) ==="
else
  echo "=== verdict: PROBLEMS FOUND (exit 1) ==="
fi
exit "$fail"

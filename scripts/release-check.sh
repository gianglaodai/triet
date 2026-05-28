#!/usr/bin/env bash
# release-check.sh — ADR-0009 4-gate verifier + drift checks.
#
# Run BEFORE tagging any vX.Y release. Codifies the gate matrix that v0.3-v0.7
# honored manually and that v0.8 release commit `78f2402` violated.
#
# Exit codes:
#   0 — all gates pass, safe to tag.
#   1 — at least one critical gate failed; refuse to release.
#
# Usage:
#   scripts/release-check.sh
#   scripts/release-check.sh --skip-tests    # for incremental development checks
#
# See ADR-0009 Addendum (v0.8.x.cadence-fix.3) for policy rationale.

set -u
cd "$(dirname "$0")/.."

PASS="\033[32m✓\033[0m"
FAIL="\033[31m✗\033[0m"
WARN="\033[33m!\033[0m"
NC="\033[0m"

errors=0
warnings=0

check() {
    local label="$1"
    shift
    printf "  %s … " "$label"
    if "$@" > /tmp/release-check.out 2>&1; then
        printf "%b\n" "$PASS"
    else
        printf "%b\n" "$FAIL"
        echo "    --- output ---"
        sed 's/^/    /' /tmp/release-check.out | tail -20
        echo "    ---"
        errors=$((errors + 1))
    fi
}

warn_check() {
    local label="$1"
    shift
    printf "  %s … " "$label"
    if "$@" > /tmp/release-check.out 2>&1; then
        printf "%b\n" "$PASS"
    else
        printf "%b (warning, not blocker)\n" "$WARN"
        sed 's/^/    /' /tmp/release-check.out | tail -10
        warnings=$((warnings + 1))
    fi
}

skip_tests=0
for arg in "$@"; do
    case "$arg" in
        --skip-tests) skip_tests=1 ;;
    esac
done

echo "Gate A — Functional"
echo "Gate B — Hygiene (ADR-0009 §B)"
if [ "$skip_tests" -eq 0 ]; then
    check "cargo test --workspace" cargo test --workspace --quiet
else
    printf "  cargo test --workspace … %b skipped (--skip-tests)\n" "$WARN"
fi
check "cargo clippy --workspace --all-targets -- -D warnings" \
    cargo clippy --workspace --all-targets -- -D warnings
check "cargo fmt --all --check" cargo fmt --all --check

echo ""
echo "Gate C — Docs (version sync)"
cargo_version=$(grep -E '^version = "[0-9]' Cargo.toml | head -1 | sed -E 's/version = "([^"]+)".*/\1/')
spec_version=$(head -1 SPEC.md | sed -E 's/.*v([0-9]+\.[0-9]+).*/\1/')
cargo_minor=$(echo "$cargo_version" | sed -E 's/^([0-9]+\.[0-9]+).*/\1/')
printf "  Cargo.toml workspace.package.version = %s … " "$cargo_version"
if [ -n "$cargo_version" ]; then
    printf "%b\n" "$PASS"
else
    printf "%b\n" "$FAIL"; errors=$((errors + 1))
fi
printf "  SPEC.md header version = v%s … " "$spec_version"
if [ "$spec_version" = "$cargo_minor" ]; then
    printf "%b (matches Cargo minor)\n" "$PASS"
else
    printf "%b (expected v%s, got v%s)\n" "$FAIL" "$cargo_minor" "$spec_version"
    errors=$((errors + 1))
fi

echo ""
echo "Gate D — Self-consistency (drift checks)"

# Check: no stray TODO(vX.Y) markers in current-version code
todo_pattern="TODO(v${cargo_minor}"
warn_check "no stray TODO($cargo_minor) markers in src/" \
    bash -c "! grep -rn 'TODO(v${cargo_minor}' crates/*/src/ 2>/dev/null | grep -v test"

# Check: TODO.md has no unfinished sub-tasks in active phase
warn_check "TODO.md has no [ ] unchecked items in active phase" \
    bash -c "! grep -E '^- \[ \]' TODO.md"

# Check: each shipped phase in TODO archive references commit hashes
warn_check "TODO archive table has 'Final test count' column populated" \
    bash -c "grep -c '|.*|.*|.*[0-9].*|' TODO.md | xargs test 5 -lt"

echo ""
echo "ADR status sanity"
# Check: ADRs with 'Locked' status should reflect normative content
warn_check "no ADRs in 'Draft' status referenced normatively from SPEC" \
    bash -c "
        # For each ADR file linked from SPEC, check its status
        spec_adrs=\$(grep -oE 'ADR-[0-9]+' SPEC.md | sort -u)
        for adr in \$spec_adrs; do
            num=\$(echo \$adr | sed 's/ADR-0*//' | sed 's/^0*//')
            file=\$(ls docs/decisions/00\${num}-*.md 2>/dev/null | head -1)
            if [ -z \"\$file\" ]; then continue; fi
            status=\$(grep -E '^\*\*Trạng thái:' \"\$file\" | head -1)
            if echo \"\$status\" | grep -qE 'Draft[^[:alnum:]]' ; then
                echo \"\$adr at \$file is Draft but referenced from SPEC\"
                exit 1
            fi
        done
        exit 0
    "

echo ""
echo "========================================"
if [ "$errors" -gt 0 ]; then
    printf "%b Release check FAILED — %d critical gate(s) failed.\n" "$FAIL" "$errors"
    if [ "$warnings" -gt 0 ]; then
        printf "%b Plus %d warning(s).\n" "$WARN" "$warnings"
    fi
    echo "Fix above before tagging release."
    exit 1
elif [ "$warnings" -gt 0 ]; then
    printf "%b Release check PASSED with %d warning(s).\n" "$WARN" "$warnings"
    echo "Critical gates pass. Review warnings above before tagging."
    exit 0
else
    printf "%b Release check PASSED. ADR-0009 4-gate matrix clean. Safe to tag.\n" "$PASS"
    exit 0
fi

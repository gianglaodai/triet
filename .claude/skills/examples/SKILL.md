---
name: examples
description: Run all .tri example files through the compiler as smoke tests and report pass/fail.
trigger: /examples
argument-hint: "[demo-dir] — optional, e.g. /examples demos/02-module-system to test a specific demo"
---

# /examples — Smoke test .tri programs

Run every `.tri` file in `examples/` (and optionally a specific `demos/` dir) through the release binary and report pass/fail.

## Steps

1. Build release binary first (needed for speed with Long arithmetic):
   ```bash
   cargo build --release
   ```

2. Run all examples:
   ```bash
   for f in examples/*.tri; do
     ./target/release/triet run "$f" && echo "OK: $f" || echo "FAILED: $f"
   done
   ```

3. If a demo directory is provided as argument, run that too:
   ```bash
   for f in <demo-dir>/*.tri; do
     ./target/release/triet run "$f" && echo "OK: $f" || echo "FAILED: $f"
   done
   ```

## Expectations

- Every `.tri` file should parse, typecheck, and run to completion.
- Any FAILED line indicates a regression — investigate immediately.
- Known-failing files (if any) are documented in TODO.md with reason.

## Important

- Always `cargo build --release` first. The debug binary is too slow for Long arithmetic in `examples/long_arithmetic.tri`.
- The user manually reviews output; this skill only reports pass/fail, not expected output correctness.
- `examples/` files are the authoritative smoke test suite — if they break, the branch is not mergable.

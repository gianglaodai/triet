---
name: check
description: Run cargo test --workspace + cargo clippy --workspace --all-targets, the pre-commit validation ritual.
---

# /check — Pre-commit validation

Run the full Triết validation suite. This MUST pass before any commit.

## Steps

1. Run all tests across the workspace:
   ```bash
   cargo test --workspace
   ```

2. Run clippy with strict workspace lints:
   ```bash
   cargo clippy --workspace --all-targets
   ```

3. If clippy finds warnings, auto-fix what can be auto-fixed:
   ```bash
   cargo clippy --workspace --all-targets --fix --allow-dirty --allow-staged
   ```

4. Re-run step 2 to confirm zero warnings.

## Expected output

- All tests green.
- Clippy zero warnings (workspace config is `pedantic` + `nursery` at `warn`).

## If failures

- **Test failures**: fix the code. Never skip or `#[ignore]` tests without explicit user request.
- **Clippy warnings**: fix every new warning. No `#[allow]` bandaids.
- **Format issues**: `cargo fmt --all` then re-run.

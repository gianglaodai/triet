//! Self-hosting bootstrap test harness for Triết (v0.7+).
//!
//! This crate is the home of the test suite that proves
//! [ADR-0019](../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md)
//! invariants:
//!
//! - **`bootstrap_determinism`** — every commit must produce
//!   byte-identical `.triv` and `.khi` output for the same input
//!   across N rebuilds. Catches HashMap-iteration leaks, env-dep
//!   state, timestamp injections, and sort-at-boundary regressions.
//! - **Per-component differential tests** (v0.7.4 → v0.7.8) — Triết-in-Triết
//!   `lexer/parser/modules/typecheck/lowerer` outputs ≡ Rust impl.
//! - **`bootstrap_loop`** (v0.7.11 → v0.7.12) — full 3-stage chain:
//!   Stage 1 (Rust) → Stage 2 (Triết-built-by-Stage-1) → Stage 3
//!   (Triết-built-by-Stage-2); gate `cmp stage2.khi stage3.khi`
//!   exit 0.
//!
//! The library surface is intentionally empty at v0.7.2 — tests live
//! under `tests/`. Public helpers will land as `lexer_differential`,
//! `parser_differential`, etc. crates need them.

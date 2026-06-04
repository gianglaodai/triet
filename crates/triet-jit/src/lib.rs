//! Triết JIT — Cranelift-backed native codegen (Track B).
//!
//! MIR-based JIT compiler. Takes `triet_mir::Body`, emits Cranelift IR,
//! produces native x86-64 code. Bậc A: all values are i64 — scalars
//! unboxed, aggregates as opaque VM heap pointers.

#![warn(missing_docs)]

pub mod mir_lower;

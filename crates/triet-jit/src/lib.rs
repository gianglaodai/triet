//! Triết JIT — Cranelift-backed Tier 2 backend per [ADR-0030].
//!
//! This crate is the **scaffold** layer for the v0.9 JIT subsystem.
//! Sub-task `v0.9.x.jit.1` ships only the public API skeleton + crate
//! wiring; actual opcode translation, codegen, and runtime integration
//! land in subsequent sub-tasks per [ADR-0030 §11]:
//!
//! - `.2` — opcode-by-opcode translation (arithmetic / comparisons /
//!   control flow `BrIf` + `BrTrilean` per ADR-0010 backend).
//! - `.3` — call dispatch (`CallLocal` / `CallCrossModule` /
//!   `WitnessCall` per ADR-0012).
//! - `.4` — builtin shim integration (opcodes 4-26 + 27-39 Atomic per
//!   ADR-0028).
//! - `.5` — VM dispatcher integration (call-count trigger + JIT
//!   compile path + native call thunk per [ADR-0030 §2]).
//! - `.6` — AOT cache filesystem layout + invalidation per
//!   [ADR-0030 §5].
//! - `.7` — Stage 2 ≡ Stage 3 byte-identical gate lift per
//!   [ADR-0019 §7 Addendum].
//! - `.8` — perf bench: ≥10× v0.3 baseline + bootstrap < 10 min.
//!
//! # Public API surface
//!
//! [`JitCompiler`] is the primary entry point — compile a
//! [`triet_ir::Function`] into native code, return a pointer suitable
//! for thunking from the VM dispatcher. [`JitError`] enumerates
//! compile failures; on error the VM falls back to bytecode dispatch
//! permanently for that `FuncId` (no retry — per ADR-0030 §2
//! "Tier-down on failure").
//!
//! # Capability gate (per [ADR-0030 Addendum Gap 1])
//!
//! JIT codegen requires `dev.jit_codegen` capability. Default ambient
//! for `usr.*` programs (free JIT). Kernel/embedded programs
//! explicitly `deny` — runtime detects + falls back to VM-only mode.
//! Capability enforcement lands in `.5` VM-dispatcher integration; this
//! scaffold layer doesn't gate.
//!
//! # `unsafe_code` policy
//!
//! Cranelift's `JITModule::finalize_definitions` returns raw function
//! pointers that the dispatcher casts to `extern "C"` callables — this
//! requires `unsafe`. The actual `unsafe` blocks land in `.5`
//! integration; scaffold layer only declares types, no machine code is
//! produced yet. Workspace-wide `unsafe_code = "forbid"` is overridden
//! to `deny` at this crate's `lib.rs` once `.5` lands; for now we keep
//! the default and revisit when codegen runs.
//!
//! [ADR-0030]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 §2]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 §5]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 §11]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 Addendum Gap 1]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0019 §7 Addendum]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md

#![warn(missing_docs)]

use std::collections::HashMap;

use thiserror::Error;
use triet_ir::FuncId;

/// JIT compiler instance per Triết runtime.
///
/// Owns the Cranelift JIT module + a cache of compiled function
/// pointers indexed by [`FuncId`]. One instance per
/// [`triet_ir::Vm`] (created lazily on first JIT trigger per
/// [ADR-0030 §5] dispatcher integration).
///
/// **Scaffold note:** v0.9.x.jit.1 ships the type with placeholder
/// internals (empty cache); `compile`/`lookup` return
/// [`JitError::Unimplemented`] until `.2` lands the opcode
/// translation layer.
///
/// [ADR-0030 §5]: ../../../docs/decisions/0030-jit-cranelift-integration.md
pub struct JitCompiler {
    /// Cache of native-code pointers keyed by `FuncId`. Populated on
    /// successful `compile()`; consulted by `lookup()` on dispatch.
    /// The pointer is opaque at this layer — `.5` integration casts
    /// it to the appropriate `extern "C"` calling convention.
    function_cache: HashMap<FuncId, NativeCodePtr>,
}

/// Opaque pointer to native machine code.
///
/// Wraps a `*const u8` to keep the public API trait-bound clean
/// (no raw pointer leakage at the type-system level). Dereferenced
/// into a calling-convention-matching `fn` pointer by the VM
/// dispatcher in `.5`.
#[derive(Clone, Copy, Debug)]
pub struct NativeCodePtr {
    /// Underlying machine-code address. `usize` rather than `*const u8`
    /// to avoid `Send`/`Sync` autoderivation concerns at scaffold layer.
    pub addr: usize,
}

/// JIT compilation errors per [ADR-0030 §2] tier-down policy.
///
/// On error, the VM marks the `FuncId` as JIT-failed and continues
/// dispatching via bytecode. No retry — failure is permanent for the
/// session.
///
/// [ADR-0030 §2]: ../../../docs/decisions/0030-jit-cranelift-integration.md
#[derive(Debug, Error)]
pub enum JitError {
    /// The opcode translation, call dispatch, builtin shim, or VM
    /// integration layer is not yet implemented. Lands in
    /// `v0.9.x.jit.2`–`.5` per ADR-0030 §11.
    #[error("JIT compilation not yet implemented (scaffold layer; see ADR-0030 §11)")]
    Unimplemented,

    /// The function uses an IR opcode that the current backend doesn't
    /// handle. Triggers permanent VM-only dispatch for this `FuncId`.
    #[error("unsupported IR opcode for JIT backend: {opcode}")]
    UnsupportedOpcode {
        /// Human-readable opcode name (per `Display for Instruction`).
        opcode: String,
    },

    /// Cranelift-internal error (verification failure, type mismatch
    /// in generated IR, target unsupported, etc.). Treated as
    /// JIT-failed for this session.
    #[error("Cranelift backend error: {message}")]
    Cranelift {
        /// Source error message from Cranelift, opaquely.
        message: String,
    },

    /// Capability gate `dev.jit_codegen` denied — kernel/embedded
    /// program declared `deny` per ADR-0030 Addendum Gap 1. Runtime
    /// falls back to VM-only mode entirely (not just this function).
    #[error("dev.jit_codegen capability denied — running in VM-only mode")]
    CapabilityDenied,
}

impl JitCompiler {
    /// Construct an empty JIT compiler. Cranelift JIT module is
    /// initialized lazily on first `compile()` call (currently a
    /// scaffold no-op).
    #[must_use]
    pub fn new() -> Self {
        Self {
            function_cache: HashMap::new(),
        }
    }

    /// Attempt to JIT-compile `func` and return the native code
    /// pointer on success. Scaffold layer returns
    /// `JitError::Unimplemented` unconditionally — actual codegen
    /// lands in `v0.9.x.jit.2`.
    ///
    /// # Errors
    ///
    /// Always `JitError::Unimplemented` at this layer.
    // Scaffold method: future `.2+` impl will not be `const fn`
    // (does I/O via Cranelift codegen). Allow lint to keep the
    // signature stable across sub-phases.
    #[allow(clippy::missing_const_for_fn)]
    pub fn compile(&mut self, func: &triet_ir::Function) -> Result<NativeCodePtr, JitError> {
        // Acknowledge the argument to silence unused warnings.
        let _ = func;
        Err(JitError::Unimplemented)
    }

    /// Return a previously-compiled native code pointer for `id`, or
    /// `None` if not yet JIT'd. Always `None` at scaffold layer.
    #[must_use]
    pub fn lookup(&self, id: FuncId) -> Option<NativeCodePtr> {
        self.function_cache.get(&id).copied()
    }

    /// Return the number of functions currently cached. Used by
    /// internal diagnostics + smoke tests.
    #[must_use]
    pub fn cached_function_count(&self) -> usize {
        self.function_cache.len()
    }
}

impl Default for JitCompiler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffold_new_returns_empty_compiler() {
        let jit = JitCompiler::new();
        assert_eq!(jit.cached_function_count(), 0);
    }

    #[test]
    fn scaffold_default_matches_new() {
        let a = JitCompiler::default();
        let b = JitCompiler::new();
        assert_eq!(a.cached_function_count(), b.cached_function_count());
    }

    #[test]
    fn scaffold_lookup_on_empty_cache_returns_none() {
        let jit = JitCompiler::new();
        assert!(jit.lookup(FuncId(0)).is_none());
    }

    #[test]
    fn scaffold_compile_returns_unimplemented() {
        // v0.9.x.jit.1 scaffold: compile always errs with
        // Unimplemented. Real codegen lands in .2.
        let mut jit = JitCompiler::new();
        // Construct a minimal Function shell via the public
        // constructor — content doesn't matter because compile()
        // ignores its argument at scaffold layer.
        let func = triet_ir::Function::new(
            FuncId(0),
            Some("smoke".to_string()),
            Vec::new(),
            triet_ir::TypeTag::Unit,
        );
        match jit.compile(&func) {
            Err(JitError::Unimplemented) => {} // expected
            other => panic!("expected JitError::Unimplemented, got {other:?}"),
        }
    }
}

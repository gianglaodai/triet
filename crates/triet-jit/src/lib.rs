//! Triết JIT — Cranelift-backed Tier 2 backend per [ADR-0030].
//!
//! v0.9 JIT subsystem. Sub-task progression per [ADR-0030 §11]:
//!
//! - `.2` — opcode-by-opcode translation (arithmetic / comparisons /
//!   control flow `BrIf` + `BrTrilean` per ADR-0010 backend).
//! - `.3` — call dispatch (`CallLocal` / `CallCrossModule` /
//!   `WitnessCall` per ADR-0012).
//! - `.4` — builtin shim integration → **deferred v0.10** per
//!   [ADR-0030 §12] (`RuntimeValue` ABI marshaling complexity).
//!   Ships only structured diagnostic for tier-down.
//! - `.5` — VM dispatcher integration (call-count trigger + JIT
//!   compile path + native call thunk per [ADR-0030 §2]).
//! - `.6` — AOT cache filesystem layout → **deferred v0.10** per
//!   [ADR-0030 §13] (cranelift-jit → cranelift-object backend
//!   swap). Ships ADR backlog only.
//! - `.7` — bootstrap gate lift → **deferred v0.10** per
//!   [ADR-0030 §14] (chained from §13.5: no AOT cache → 3000-fn
//!   self-host × cold JIT cost prohibitive).
//! - `.8` — perf bench → **deferred v0.10** per [ADR-0030 §14]
//!   (chained from §12: most builtins tier-down → benchmark
//!   understates architectural value; defer to alongside `.4`
//!   completion for honest measurement).
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
//! requires `unsafe`. As of `.5`, `dispatch_integer` localizes the
//! transmute + call in a single `#[allow(unsafe_code)]` block with
//! the safety contract documented at the function level. Workspace-
//! wide `unsafe_code = "forbid"` is overridden to `deny` only at this
//! crate's `Cargo.toml` `[lints.rust]` table (not propagated to other
//! crates).
//!
//! [ADR-0030]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 §2]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 §5]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 §11]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 §12]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 §13]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 §14]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0030 Addendum Gap 1]: ../../../../docs/decisions/0030-jit-cranelift-integration.md
//! [ADR-0019 §7 Addendum]: ../../../../docs/decisions/0019-self-hosting-compiler-bootstrap.md

#![warn(missing_docs)]

mod codegen;
mod shims;

use std::collections::HashMap;

use thiserror::Error;
use triet_ir::{BuiltinName, FuncId, VmError};

use crate::codegen::JitBackend;

/// JIT compiler instance per Triết runtime.
///
/// Owns the Cranelift JIT module + a cache of compiled function
/// pointers indexed by [`FuncId`]. One instance per
/// [`triet_ir::Vm`] (created lazily on first JIT trigger per
/// [ADR-0030 §5] dispatcher integration).
///
/// **v0.9.x.jit.4 status:** `compile` (single-fn) covers arithmetic,
/// comparison, and control-flow opcodes. `compile_program` (multi-fn)
/// additionally resolves `CallLocal`, `CallCrossModule`, and
/// `WitnessCall` (the latter dispatches identically to
/// `CallCrossModule` per ADR-0012 v0.4 semantics), and materializes
/// inline `Operand::Const` against the program's constant pool.
/// `CallBuiltin` raises a name-bearing tier-down diagnostic per
/// ADR-0030 §12 (full shim layer defers v0.10 — `RuntimeValue` ABI
/// marshaling complexity). Closures, aggregates, nullable / outcome
/// wrappers, and the `Long` type also raise
/// [`JitError::UnsupportedOpcode`] so the caller tiers down to
/// VM-only dispatch per ADR-0030 §2.
///
/// [ADR-0030 §5]: ../../../docs/decisions/0030-jit-cranelift-integration.md
pub struct JitCompiler {
    /// Cache of native-code pointers keyed by `FuncId`. Populated on
    /// successful `compile()`; consulted by `lookup()` on dispatch.
    /// The pointer is opaque at this layer — `.5` integration casts
    /// it to the appropriate `extern "C"` calling convention.
    function_cache: HashMap<FuncId, NativeCodePtr>,
    /// Lazily-initialized Cranelift JIT backend. `None` until the
    /// first `compile()` call so failed ISA detection doesn't break
    /// callers that never JIT.
    backend: Option<JitBackend>,
}

/// Opaque pointer to native machine code.
///
/// Wraps a `*const u8` to keep the public API trait-bound clean
/// (no raw pointer leakage at the type-system level). Dereferenced
/// into a calling-convention-matching `fn` pointer by the VM
/// dispatcher in `.5`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    /// Reserved for future sub-phases that wire entire feature areas
    /// in one shot (e.g. `.4` builtin shim integration, `.5` VM
    /// dispatcher with capability gate). Currently no codegen path
    /// returns this variant — per-opcode failures use
    /// [`JitError::UnsupportedOpcode`] instead. Kept for forward
    /// compatibility with `.4`/`.5` integration.
    #[error("JIT compilation not yet implemented (see ADR-0030 §11)")]
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

    /// v0.10.x.jit.1 — a `CallBuiltin` opcode names a builtin whose
    /// capability namespace is denied by the program's
    /// capability set. Per [ADR-0032 §3] this is a **compile-time
    /// defense-in-depth** check (the authoritative gate runs at
    /// program-load time per ADR-0016 §5). On this error the function
    /// tiers down to VM dispatch, where the same denial surfaces.
    ///
    /// [ADR-0032 §3]: ../../../docs/decisions/0032-builtin-shim-abi.md
    #[error("builtin `{builtin}` requires capability namespace `{namespace}` (denied)")]
    BuiltinCapabilityDenied {
        /// The builtin whose namespace was denied (Display form).
        builtin: String,
        /// The capability namespace required + denied.
        namespace: String,
    },
}

/// v0.10.x.jit.1 — Look up a [`BuiltinName`]'s capability namespace +
/// test it against a denied-namespace set per [ADR-0032 §3].
///
/// Returns `Err(JitError::BuiltinCapabilityDenied { .. })` when the
/// builtin's namespace appears in `denied`. The JIT codegen consults
/// this before emitting a builtin-shim call (defense-in-depth; the
/// authoritative gate is upstream at program-load time per
/// ADR-0016 §5). Empty `denied` (the production default — capabilities
/// already resolved at load) always returns `Ok`.
///
/// [ADR-0032 §3]: ../../../docs/decisions/0032-builtin-shim-abi.md
pub(crate) fn check_builtin_capability(
    builtin: BuiltinName,
    denied: &[&str],
) -> Result<(), JitError> {
    let namespace = shims::builtin_namespace(builtin);
    if denied.contains(&namespace) {
        return Err(JitError::BuiltinCapabilityDenied {
            builtin: format!("{builtin}"),
            namespace: namespace.to_owned(),
        });
    }
    Ok(())
}

impl JitCompiler {
    /// Construct an empty JIT compiler. Cranelift JIT module is
    /// initialized lazily on first `compile()` call.
    #[must_use]
    pub fn new() -> Self {
        Self {
            function_cache: HashMap::new(),
            backend: None,
        }
    }

    /// Attempt to JIT-compile `func` standalone and return the native
    /// code pointer on success.
    ///
    /// **Use [`Self::compile_program`] instead** when the function
    /// has cross-function calls, witness-table generics, or inline
    /// constants — those need program-level wiring. The single-fn
    /// path uses an empty pool + empty func map and will reject any
    /// such opcode.
    ///
    /// On success, the pointer is also stored in the cache so
    /// [`Self::lookup`] returns the same address.
    ///
    /// # Errors
    ///
    /// - [`JitError::UnsupportedOpcode`] for any IR opcode outside
    ///   the .2 supported set.
    /// - [`JitError::Cranelift`] if Cranelift backend rejects the
    ///   emitted IR (verifier failure, target unsupported, etc.).
    ///
    /// # Panics
    ///
    /// Panics only if backend initialization reports success but the
    /// field is `None` — defensively unreachable via the immediately-
    /// preceding `Some(...)` assignment.
    pub fn compile(&mut self, func: &triet_ir::Function) -> Result<NativeCodePtr, JitError> {
        if self.backend.is_none() {
            self.backend = Some(JitBackend::new()?);
        }
        let backend = self.backend.as_mut().expect("backend just initialized");
        let addr = backend.compile_function(func)?;
        let ptr = NativeCodePtr { addr };
        self.function_cache.insert(func.id, ptr);
        Ok(ptr)
    }

    /// JIT-compile every function in `program` with full
    /// cross-function dispatch wiring per ADR-0030 §3. Functions
    /// that fail (per-function `JitError::UnsupportedOpcode`) are
    /// silently skipped (tier-down per ADR-0030 §2); the cache
    /// only contains successfully-compiled entries.
    ///
    /// Use this entry point (instead of [`Self::compile`]) whenever
    /// the program has cross-function calls, witness-table generics,
    /// or inline constant operands — the single-function path lacks
    /// the program context to resolve any of those.
    ///
    /// # Errors
    ///
    /// - [`JitError::Cranelift`] if the pre-pass (function signature
    ///   declarations) or final `finalize_definitions` fails.
    ///
    /// # Panics
    ///
    /// Panics only if backend initialization reports success but the
    /// field is `None` — defensively unreachable.
    pub fn compile_program(&mut self, program: &triet_ir::IrProgram) -> Result<(), JitError> {
        if self.backend.is_none() {
            self.backend = Some(JitBackend::new()?);
        }
        let backend = self.backend.as_mut().expect("backend just initialized");
        // Production path: empty denied-set. Capability gating is
        // authoritative at program-load time (ADR-0016 §5); the JIT's
        // §3 check is defense-in-depth and only fires when a non-empty
        // denied set is threaded through (see the test-only
        // `compile_program_denied`).
        backend.compile_program(program, &mut self.function_cache, &[])
    }

    /// v0.10.x.jit.1 (test-support) — compile with an explicit
    /// denied-namespace set, exercising the ADR-0032 §3
    /// `BuiltinCapabilityDenied` defense-in-depth path. Production
    /// callers use [`Self::compile_program`] (empty denied-set).
    #[cfg(test)]
    pub(crate) fn compile_program_denied(
        &mut self,
        program: &triet_ir::IrProgram,
        denied: &[&str],
    ) -> Result<(), JitError> {
        if self.backend.is_none() {
            self.backend = Some(JitBackend::new()?);
        }
        let backend = self.backend.as_mut().expect("backend just initialized");
        backend.compile_program(program, &mut self.function_cache, denied)
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

    /// v0.10.x.jit.1 (test-support) — build a backend with `extra_shims`
    /// registered, compile a synthetic caller forwarding to
    /// `shim_symbol`, and cache it under `func_id` so the framework
    /// tests can drive it through [`dispatch_integer_caught`].
    #[cfg(test)]
    fn cache_shim_caller(
        &mut self,
        func_id: FuncId,
        extra_shims: &[shims::ShimEntry],
        caller_sig: &shims::ShimSignature,
        shim_symbol: &str,
        shim_sig: &shims::ShimSignature,
    ) -> Result<(), JitError> {
        let mut backend = JitBackend::new_with_extra_shims(extra_shims)?;
        let addr = backend.compile_shim_caller(caller_sig, shim_symbol, shim_sig)?;
        self.backend = Some(backend);
        self.function_cache.insert(func_id, NativeCodePtr { addr });
        Ok(())
    }
}

impl Default for JitCompiler {
    fn default() -> Self {
        Self::new()
    }
}

/// v0.9.x.jit.5 — Per-process call count threshold for JIT graduation.
///
/// Per [ADR-0030 §2]. Functions hit 100 invocations → dispatcher
/// triggers Cranelift compilation of the entire program. Hotspot
/// JVM convention. Runtime-override via `TRIET_JIT_THRESHOLD` env
/// var (deferred to a follow-up commit; constant for now).
///
/// [ADR-0030 §2]: ../../../docs/decisions/0030-jit-cranelift-integration.md
pub const JIT_THRESHOLD: u32 = 100;

/// v0.9.x.jit.5 — Runtime-side JIT integration façade.
///
/// Implements [`triet_ir::JitDispatch`] by wrapping a [`JitCompiler`]
/// plus per-`FuncId` call counters. The Vm installs this via
/// `Vm::set_jit_dispatcher` after construction. The CLI does this
/// when `--no-jit` is absent and `TRIET_JIT` env var doesn't request
/// disable, per ADR-0030 Addendum Gap 3.
///
/// Compilation is **whole-program once** semantics: the first
/// function to cross [`JIT_THRESHOLD`] triggers a single
/// `compile_program` pass that JIT-compiles every eligible function
/// in the program (per ADR-0030 §3 + §11.3 batched-compile model).
/// Subsequent threshold-crossings are no-ops; the cache is
/// populated once and consulted on every subsequent call.
pub struct JitDispatcher {
    /// Underlying Cranelift compiler holding the native code cache.
    compiler: JitCompiler,
    /// Per-`FuncId` call-count counters. Incremented by
    /// [`Self::record_call`]; the first counter to reach
    /// [`JIT_THRESHOLD`] triggers the whole-program compile.
    counters: HashMap<FuncId, u32>,
    /// One-shot guard. `false` until the first threshold-crossing
    /// fires `compile_program`; `true` after (subsequent
    /// `record_call`s skip the compile path).
    compiled: bool,
}

impl JitDispatcher {
    /// Construct a fresh dispatcher with no compiled functions and
    /// zeroed counters.
    #[must_use]
    pub fn new() -> Self {
        Self {
            compiler: JitCompiler::new(),
            counters: HashMap::new(),
            compiled: false,
        }
    }

    /// Read access to the underlying compiler (for diagnostics, test
    /// inspection, or future capability gate hooks).
    #[must_use]
    pub const fn compiler(&self) -> &JitCompiler {
        &self.compiler
    }
}

impl Default for JitDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl triet_ir::JitDispatch for JitDispatcher {
    fn record_call(&mut self, func_id: FuncId, program: &triet_ir::IrProgram) {
        if self.compiled {
            // Counter still increments — useful for diagnostics — but
            // compile path is closed. No-op compile-wise.
            *self.counters.entry(func_id).or_insert(0) += 1;
            return;
        }
        let counter = self.counters.entry(func_id).or_insert(0);
        *counter += 1;
        if *counter >= JIT_THRESHOLD {
            // Best-effort whole-program compile. Per-function
            // tier-down (UnsupportedOpcode) is silently absorbed —
            // those functions stay VM-only.
            let _ = self.compiler.compile_program(program);
            self.compiled = true;
        }
    }

    fn try_dispatch(&self, func_id: FuncId, args: &[i64]) -> Option<i64> {
        dispatch_integer(&self.compiler, func_id, args)
    }
}

/// v0.9.x.jit.5 — Dispatch a JIT-compiled function whose signature is
/// **all-`Integer` params + `Integer` return** (arity 0–4) from the
/// VM. Returns `None` when:
///
/// - The function isn't in the JIT cache (not yet compiled, or
///   compilation failed for this `FuncId`).
/// - The argument count exceeds the supported arity range (0–4 inclusive).
///
/// This is the **single safe-API gateway** for the VM's JIT trigger
/// path. Cranelift returns raw `*const u8` for finalized code and any
/// transmute to an `extern "C" fn` pointer is fundamentally unsafe;
/// this function localizes that unsafe to one auditable site so the
/// VM crate stays under `unsafe_code = "forbid"`.
///
/// # Safety contract (internal — documented for auditability)
///
/// The internal `unsafe { mem::transmute(...) }` is sound iff:
///
/// 1. `jit.lookup(func_id)` returned a pointer to native code that
///    Cranelift compiled with a signature of N `i64` params + `i64`
///    return for some N ≤ 4. The codegen layer guarantees this via
///    [`codegen::map_type`]: `TypeTag::Integer` → `types::I64` always.
/// 2. The function's `JitCompiler::compile_program` succeeded
///    without an `UnsupportedOpcode` tier-down — already implied by
///    the cache hit (failed compiles are never cached).
/// 3. The host platform's calling convention matches Cranelift's
///    `CallConv::SystemV` (or the equivalent Win64) — set in the
///    Cranelift IR at codegen time.
///
/// VM-side caller MUST verify the callee's IR signature is
/// all-Integer before calling this. The
/// [`is_jit_integer_dispatchable`] helper exists for that pre-check.
///
/// [`codegen::map_type`]: crate::codegen
pub fn dispatch_integer(jit: &JitCompiler, func_id: FuncId, args: &[i64]) -> Option<i64> {
    let ptr = jit.lookup(func_id)?;
    if args.len() > 4 {
        return None;
    }
    // SAFETY: see fn-level doc-comment. The transmute is sound under
    // the three invariants enumerated; VM caller is responsible for
    // signature pre-check via `is_jit_integer_dispatchable`.
    #[allow(unsafe_code)]
    let result = unsafe {
        match args.len() {
            0 => {
                let f: extern "C" fn() -> i64 = std::mem::transmute(ptr.addr as *const ());
                f()
            }
            1 => {
                let f: extern "C" fn(i64) -> i64 = std::mem::transmute(ptr.addr as *const ());
                f(args[0])
            }
            2 => {
                let f: extern "C" fn(i64, i64) -> i64 = std::mem::transmute(ptr.addr as *const ());
                f(args[0], args[1])
            }
            3 => {
                let f: extern "C" fn(i64, i64, i64) -> i64 =
                    std::mem::transmute(ptr.addr as *const ());
                f(args[0], args[1], args[2])
            }
            4 => {
                let f: extern "C" fn(i64, i64, i64, i64) -> i64 =
                    std::mem::transmute(ptr.addr as *const ());
                f(args[0], args[1], args[2], args[3])
            }
            // Unreachable per the `if args.len() > 4` guard above.
            _ => return None,
        }
    };
    Some(result)
}

/// v0.10.x.jit.2a — Dispatch an all-`Integer` JIT'd function that may
/// call builtin shims, propagating shim failures as `Err(VmError)` per
/// the [ADR-0032 §4 option-2 resolution].
///
/// Unlike [`dispatch_integer`], this clears the thread-local shim state
/// before the call + reads it after: a shim that fails records a
/// `VmError` + sets `SHIM_FAILED`, the JIT-emitted per-call probe
/// branches the function to its `error_exit` (returning a sentinel),
/// and this dispatcher converts the recorded error into `Err`. No
/// unwinding crosses the Cranelift frame — shims are plain `extern "C"`
/// and return normally.
///
/// Returns:
/// - `Some(Ok(value))` — clean run (no shim set the failure flag).
/// - `Some(Err(vm_error))` — a shim recorded a failure (the function's
///   sentinel return is discarded).
/// - `None` — function not in the JIT cache, or arity > 4.
///
/// # Safety contract
///
/// Identical to [`dispatch_integer`] — the inner transmute is sound
/// under the same three invariants (all-`i64` signature ≤ arity 4;
/// compile success implied by cache hit; host calling convention
/// matches Cranelift's `SystemV`). Shims declare `extern "C"` (never
/// unwind), so no `catch_unwind` is needed and the cranelift-jit 0.132
/// unwind-table gap is sidestepped (ADR-0032 §4 option-2).
///
/// [ADR-0032 §4 option-2]: ../../../docs/decisions/0032-builtin-shim-abi.md
pub fn dispatch_with_shim_errors(
    jit: &JitCompiler,
    func_id: FuncId,
    args: &[i64],
    func_name: &str,
) -> Option<Result<i64, VmError>> {
    let ptr = jit.lookup(func_id)?;
    if args.len() > 4 {
        return None;
    }
    shims::clear_shim_state();
    shims::set_func_name(func_name);
    // SAFETY: see fn-level doc-comment + `dispatch_integer`'s contract.
    // The function + every shim it calls is `extern "C"` and never
    // unwinds, so no panic crosses this frame. Backed by ADR-0032 §4
    // option-2.
    #[allow(unsafe_code)]
    let value = unsafe {
        match args.len() {
            0 => {
                let f: extern "C" fn() -> i64 = std::mem::transmute(ptr.addr as *const ());
                f()
            }
            1 => {
                let f: extern "C" fn(i64) -> i64 = std::mem::transmute(ptr.addr as *const ());
                f(args[0])
            }
            2 => {
                let f: extern "C" fn(i64, i64) -> i64 = std::mem::transmute(ptr.addr as *const ());
                f(args[0], args[1])
            }
            3 => {
                let f: extern "C" fn(i64, i64, i64) -> i64 =
                    std::mem::transmute(ptr.addr as *const ());
                f(args[0], args[1], args[2])
            }
            _ => {
                let f: extern "C" fn(i64, i64, i64, i64) -> i64 =
                    std::mem::transmute(ptr.addr as *const ());
                f(args[0], args[1], args[2], args[3])
            }
        }
    };
    // Shim failure (recorded in TLS) → Err; clean run → Ok(value).
    Some(shims::take_shim_failure().map_or(Ok(value), Err))
}

/// v0.9.x.jit.5 — Pre-check used by [`triet_ir::Vm`] to decide whether
/// a function's signature qualifies for the JIT native-dispatch path.
/// Mirrors the `Integer`-only ABI [`dispatch_integer`] supports.
///
/// Returns `true` iff:
/// - All parameters are `TypeTag::Integer`
/// - The return type is `TypeTag::Integer`
/// - Arity is ≤ 4
///
/// Wider type coverage (Trit / Tryte / Trilean / Long / composites)
/// defers v0.10+ per ADR-0030 §12 backlog (`RuntimeValue` ABI
/// marshaling complexity).
#[must_use]
pub fn is_jit_integer_dispatchable(func: &triet_ir::Function) -> bool {
    if func.params.len() > 4 {
        return false;
    }
    if !matches!(func.return_type, triet_ir::TypeTag::Integer) {
        return false;
    }
    func.params
        .iter()
        .all(|(_, t)| matches!(t, triet_ir::TypeTag::Integer))
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

    // ===== v0.9.x.jit.2: end-to-end codegen tests =====
    // Build small synthetic IR functions, compile, assert success +
    // non-null pointer. Execution validation defers .5 (requires the
    // VM dispatcher integration + unsafe fn-pointer cast).

    use triet_ir::{BasicBlock, BlockId, Function, Instruction, Operand, TypeTag, ValueId};

    fn make_function(
        name: &str,
        params: Vec<(String, TypeTag)>,
        return_type: TypeTag,
        instructions: Vec<Instruction>,
    ) -> Function {
        let mut block = BasicBlock::new(BlockId(0), Some("entry".to_string()));
        block.instructions = instructions;
        let mut func = Function::new(FuncId(0), Some(name.to_string()), params, return_type);
        func.blocks = vec![block];
        func
    }

    #[test]
    fn jit2_compiles_identity_function() {
        // `id(x: Integer) -> Integer = x` — single Ret of param.
        let func = make_function(
            "id",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        let mut jit = JitCompiler::new();
        let ptr = jit.compile(&func).expect("identity compile should succeed");
        assert_ne!(ptr.addr, 0, "native pointer must be non-null");
        assert_eq!(jit.cached_function_count(), 1);
        assert_eq!(jit.lookup(FuncId(0)), Some(ptr));
    }

    #[test]
    fn jit2_compiles_integer_add() {
        // `add(a, b: Integer) -> Integer = a + b`
        let func = make_function(
            "add",
            vec![
                ("a".to_string(), TypeTag::Integer),
                ("b".to_string(), TypeTag::Integer),
            ],
            TypeTag::Integer,
            vec![
                Instruction::Add {
                    dest: ValueId(2),
                    lhs: Operand::Value(ValueId(0)),
                    rhs: Operand::Value(ValueId(1)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(2))),
                },
            ],
        );
        let mut jit = JitCompiler::new();
        jit.compile(&func)
            .expect("integer add compile should succeed");
    }

    #[test]
    fn jit2_compiles_integer_sub_mul_neg() {
        // `mix(a, b: Integer) -> Integer = -(a * b - a)`
        let func = make_function(
            "mix",
            vec![
                ("a".to_string(), TypeTag::Integer),
                ("b".to_string(), TypeTag::Integer),
            ],
            TypeTag::Integer,
            vec![
                Instruction::Mul {
                    dest: ValueId(2),
                    lhs: Operand::Value(ValueId(0)),
                    rhs: Operand::Value(ValueId(1)),
                },
                Instruction::Sub {
                    dest: ValueId(3),
                    lhs: Operand::Value(ValueId(2)),
                    rhs: Operand::Value(ValueId(0)),
                },
                Instruction::Neg {
                    dest: ValueId(4),
                    operand: Operand::Value(ValueId(3)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(4))),
                },
            ],
        );
        let mut jit = JitCompiler::new();
        jit.compile(&func)
            .expect("sub/mul/neg compile should succeed");
    }

    #[test]
    fn jit2_compiles_integer_comparison_returns_trilean() {
        // `lt(a, b: Integer) -> Trilean = a < b`
        let func = make_function(
            "lt",
            vec![
                ("a".to_string(), TypeTag::Integer),
                ("b".to_string(), TypeTag::Integer),
            ],
            TypeTag::Trilean,
            vec![
                Instruction::Lt {
                    dest: ValueId(2),
                    lhs: Operand::Value(ValueId(0)),
                    rhs: Operand::Value(ValueId(1)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(2))),
                },
            ],
        );
        let mut jit = JitCompiler::new();
        jit.compile(&func)
            .expect("Lt → Trilean compile should succeed");
    }

    #[test]
    fn jit3_const_without_pool_fails_with_missing_const_error() {
        // Single-function path uses an empty constant pool — Const
        // instruction therefore looks up an absent entry. v0.9.x.jit.3
        // surfaces this as a Cranelift-class error. Programs needing
        // constants must use `compile_program`, which threads the
        // real pool.
        let func = make_function(
            "with_const",
            vec![],
            TypeTag::Integer,
            vec![
                Instruction::Const {
                    dest: ValueId(0),
                    constant: triet_ir::ConstId(0),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
        );
        let mut jit = JitCompiler::new();
        match jit.compile(&func) {
            Err(JitError::Cranelift { message }) => {
                assert!(
                    message.contains("ConstId"),
                    "error must reference missing ConstId, got: {message}"
                );
            }
            other => panic!("expected Cranelift missing-const error, got {other:?}"),
        }
        assert_eq!(
            jit.cached_function_count(),
            0,
            "failed compile must not cache"
        );
    }

    fn make_multi_block_function(
        name: &str,
        params: Vec<(String, TypeTag)>,
        return_type: TypeTag,
        blocks: Vec<(BlockId, Vec<Instruction>)>,
    ) -> Function {
        let mut func = Function::new(FuncId(0), Some(name.to_string()), params, return_type);
        func.blocks = blocks
            .into_iter()
            .map(|(id, instructions)| {
                let mut b = BasicBlock::new(id, None);
                b.instructions = instructions;
                b
            })
            .collect();
        func
    }

    #[test]
    fn jit2_compiles_unconditional_branch() {
        // entry: Br tail
        // tail:  Ret x
        let func = make_multi_block_function(
            "br",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![
                (BlockId(0), vec![Instruction::Br { target: BlockId(1) }]),
                (
                    BlockId(1),
                    vec![Instruction::Ret {
                        value: Some(Operand::Value(ValueId(0))),
                    }],
                ),
            ],
        );
        let mut jit = JitCompiler::new();
        jit.compile(&func).expect("Br compile should succeed");
    }

    #[test]
    fn jit2_compiles_brif() {
        // entry: BrIf cond, then, else
        // then:  Ret a
        // else:  Ret b
        let func = make_multi_block_function(
            "select",
            vec![
                ("cond".to_string(), TypeTag::Trilean),
                ("a".to_string(), TypeTag::Integer),
                ("b".to_string(), TypeTag::Integer),
            ],
            TypeTag::Integer,
            vec![
                (
                    BlockId(0),
                    vec![Instruction::BrIf {
                        cond: Operand::Value(ValueId(0)),
                        then_block: BlockId(1),
                        else_block: BlockId(2),
                    }],
                ),
                (
                    BlockId(1),
                    vec![Instruction::Ret {
                        value: Some(Operand::Value(ValueId(1))),
                    }],
                ),
                (
                    BlockId(2),
                    vec![Instruction::Ret {
                        value: Some(Operand::Value(ValueId(2))),
                    }],
                ),
            ],
        );
        let mut jit = JitCompiler::new();
        jit.compile(&func).expect("BrIf compile should succeed");
    }

    #[test]
    fn jit2_compiles_brtrilean_per_adr0010() {
        // Three-way branch per ADR-0010 §4 backend table.
        //   entry: BrTrilean cond, t, u, f
        //   t/u/f: each Ret with a distinct value
        let func = make_multi_block_function(
            "trit_select",
            vec![
                ("cond".to_string(), TypeTag::Trilean),
                ("vt".to_string(), TypeTag::Integer),
                ("vu".to_string(), TypeTag::Integer),
                ("vf".to_string(), TypeTag::Integer),
            ],
            TypeTag::Integer,
            vec![
                (
                    BlockId(0),
                    vec![Instruction::BrTrilean {
                        cond: Operand::Value(ValueId(0)),
                        true_block: BlockId(1),
                        unknown_block: BlockId(2),
                        false_block: BlockId(3),
                    }],
                ),
                (
                    BlockId(1),
                    vec![Instruction::Ret {
                        value: Some(Operand::Value(ValueId(1))),
                    }],
                ),
                (
                    BlockId(2),
                    vec![Instruction::Ret {
                        value: Some(Operand::Value(ValueId(2))),
                    }],
                ),
                (
                    BlockId(3),
                    vec![Instruction::Ret {
                        value: Some(Operand::Value(ValueId(3))),
                    }],
                ),
            ],
        );
        let mut jit = JitCompiler::new();
        jit.compile(&func)
            .expect("BrTrilean compile should succeed");
    }

    #[test]
    fn jit3_single_fn_call_to_unknown_target_falls_back() {
        // Single-function path has empty func_id_map. CallLocal
        // to any callee fires UnsupportedOpcode "call target FuncId
        // not in program". Use `compile_program` for cross-call
        // dispatch.
        let func = make_function(
            "with_call",
            vec![],
            TypeTag::Integer,
            vec![
                Instruction::CallLocal {
                    dest: Some(ValueId(0)),
                    callee: FuncId(42),
                    args: vec![],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
        );
        let mut jit = JitCompiler::new();
        match jit.compile(&func) {
            Err(JitError::UnsupportedOpcode { opcode }) => {
                assert!(
                    opcode.contains("FuncId(42)"),
                    "error should name the missing callee, got: {opcode}"
                );
            }
            other => panic!("expected UnsupportedOpcode, got {other:?}"),
        }
    }

    // ===== v0.9.x.jit.3: program-level compilation + call dispatch =====

    use triet_ir::{IrModule, IrProgram};
    use triet_modules::{AbsolutePath, ModulePath};

    fn make_program(modules: Vec<IrModule>, constants: triet_ir::ConstantPool) -> IrProgram {
        IrProgram {
            modules,
            constants,
            witness_tables: Vec::new(),
        }
    }

    fn make_ir_module(module_segments: &[&str], functions: Vec<Function>) -> IrModule {
        let path = AbsolutePath::new(
            ModulePath::new(module_segments.iter().map(|s| (*s).to_string()).collect()),
            String::new(),
        );
        IrModule { path, functions }
    }

    fn make_function_at(
        id: FuncId,
        name: &str,
        params: Vec<(String, TypeTag)>,
        return_type: TypeTag,
        instructions: Vec<Instruction>,
    ) -> Function {
        let mut block = BasicBlock::new(BlockId(0), Some("entry".to_string()));
        block.instructions = instructions;
        let mut func = Function::new(id, Some(name.to_string()), params, return_type);
        func.blocks = vec![block];
        func
    }

    #[test]
    fn jit3_program_with_const_integer() {
        // `function answer() -> Integer = 42` via Const + Ret.
        let mut pool = triet_ir::ConstantPool::new();
        let cid = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(42).unwrap(),
        ));
        let answer = make_function_at(
            FuncId(0),
            "answer",
            vec![],
            TypeTag::Integer,
            vec![
                Instruction::Const {
                    dest: ValueId(0),
                    constant: cid,
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
        );
        let program = make_program(vec![make_ir_module(&["khi"], vec![answer])], pool);
        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("program with Const should compile");
        assert_eq!(jit.cached_function_count(), 1);
        assert!(jit.lookup(FuncId(0)).is_some());
    }

    #[test]
    fn jit3_program_with_call_local() {
        // main calls helper which returns 7.
        let mut pool = triet_ir::ConstantPool::new();
        let seven = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(7).unwrap(),
        ));
        let helper = make_function_at(
            FuncId(0),
            "helper",
            vec![],
            TypeTag::Integer,
            vec![
                Instruction::Const {
                    dest: ValueId(0),
                    constant: seven,
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
        );
        let main = make_function_at(
            FuncId(1),
            "main",
            vec![],
            TypeTag::Integer,
            vec![
                Instruction::CallLocal {
                    dest: Some(ValueId(0)),
                    callee: FuncId(0),
                    args: vec![],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                },
            ],
        );
        let program = make_program(vec![make_ir_module(&["khi"], vec![helper, main])], pool);
        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("CallLocal program should compile");
        assert_eq!(jit.cached_function_count(), 2);
        assert!(jit.lookup(FuncId(0)).is_some());
        assert!(jit.lookup(FuncId(1)).is_some());
    }

    #[test]
    fn jit3_program_with_cross_module_call() {
        // main (module=khi) calls helper (module=khi.utils) via
        // CallCrossModule path resolution.
        let helper = make_function_at(
            FuncId(0),
            "helper",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        let main = make_function_at(
            FuncId(1),
            "main",
            vec![("y".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![
                Instruction::CallCrossModule {
                    dest: Some(ValueId(1)),
                    path: AbsolutePath::new(
                        ModulePath::new(vec!["khi".to_string(), "utils".to_string()]),
                        "helper".to_string(),
                    ),
                    args: vec![Operand::Value(ValueId(0))],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let program = make_program(
            vec![
                make_ir_module(&["khi"], vec![main]),
                make_ir_module(&["khi", "utils"], vec![helper]),
            ],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("CallCrossModule should compile");
        assert_eq!(jit.cached_function_count(), 2);
    }

    #[test]
    fn jit3_program_with_witness_call_dispatches_same_as_cross_module() {
        // WitnessCall lowers identically to CallCrossModule at v0.4
        // semantics per ADR-0012 §2. Verify it compiles.
        let helper = make_function_at(
            FuncId(0),
            "helper",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        let main = make_function_at(
            FuncId(1),
            "main",
            vec![("y".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![
                Instruction::WitnessCall {
                    dest: Some(ValueId(1)),
                    path: AbsolutePath::new(
                        ModulePath::new(vec!["khi".to_string()]),
                        "helper".to_string(),
                    ),
                    witness_idx: 0,
                    args: vec![Operand::Value(ValueId(0))],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![helper, main])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("WitnessCall should compile (v0.4 dispatch = CallCrossModule)");
        assert_eq!(jit.cached_function_count(), 2);
    }

    // ===== v0.9.x.jit.4: structured CallBuiltin tier-down diagnostic =====
    // Full builtin shim layer defers v0.10 per ADR-0030 §12 backlog
    // (RuntimeValue ABI marshaling complexity). This sub-task ships
    // ONLY the diagnostic improvement so functions calling builtins
    // tier-down with a name-bearing error instead of opaque Debug
    // dump. Real shim wiring lands v0.10.

    #[test]
    fn jit4_callbuiltin_without_shim_tiers_down_naming_builtin() {
        // v0.10.x.jit.2a update: a builtin WITHOUT an implemented shim
        // (one of the 38 pending jit.2b) tier-downs with a diagnostic
        // that names the builtin + references the jit.2b backlog.
        // `HashMapNew` has no jit.2a shim.
        use triet_ir::BuiltinName;
        let func = make_function(
            "with_hashmap_new",
            vec![],
            TypeTag::Unit,
            vec![
                Instruction::CallBuiltin {
                    dest: None,
                    name: BuiltinName::HashMapNew,
                    args: vec![],
                },
                Instruction::Ret { value: None },
            ],
        );
        let mut jit = JitCompiler::new();
        match jit.compile(&func) {
            Err(JitError::UnsupportedOpcode { opcode }) => {
                assert!(
                    opcode.contains("CallBuiltin(hashmap_new)"),
                    "diagnostic must name the builtin via its Display impl, got: {opcode}"
                );
                assert!(
                    opcode.contains("jit.2b"),
                    "diagnostic must reference the jit.2b backlog, got: {opcode}"
                );
            }
            other => panic!("expected UnsupportedOpcode, got {other:?}"),
        }
    }

    #[test]
    fn jit4_callbuiltin_arity_mismatch_tiers_down() {
        // v0.10.x.jit.2a update: a builtin WITH a shim but called with
        // the wrong arity (e.g. `Println` with 0 args; the shim takes 1
        // composite ptr) tier-downs with an arity diagnostic rather
        // than miscompiling.
        use triet_ir::BuiltinName;
        let func = make_function(
            "println_wrong_arity",
            vec![],
            TypeTag::Unit,
            vec![
                Instruction::CallBuiltin {
                    dest: None,
                    name: BuiltinName::Println,
                    args: vec![],
                },
                Instruction::Ret { value: None },
            ],
        );
        let mut jit = JitCompiler::new();
        match jit.compile(&func) {
            Err(JitError::UnsupportedOpcode { opcode }) => {
                assert!(
                    opcode.contains("arity"),
                    "diagnostic must flag the arity mismatch, got: {opcode}"
                );
            }
            other => panic!("expected UnsupportedOpcode, got {other:?}"),
        }
    }

    #[test]
    fn jit4_program_with_builtin_caller_skipped_other_compiled() {
        // Program-level tier-down per ADR-0030 §2: function calling
        // builtin skipped, other function compiles. Same shape as
        // .jit.3's ClosureCall test but with CallBuiltin opcode.
        use triet_ir::BuiltinName;
        let pure_fn = make_function_at(
            FuncId(0),
            "pure",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        let builtin_fn = make_function_at(
            FuncId(1),
            "uses_builtin",
            vec![],
            TypeTag::Unit,
            vec![
                Instruction::CallBuiltin {
                    dest: None,
                    name: BuiltinName::Println,
                    args: vec![],
                },
                Instruction::Ret { value: None },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![pure_fn, builtin_fn])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("program should compile (per-fn tier-down)");
        assert!(jit.lookup(FuncId(0)).is_some(), "pure fn must JIT");
        assert!(
            jit.lookup(FuncId(1)).is_none(),
            "builtin-using fn must tier-down (skipped from cache)"
        );
    }

    // ===== v0.9.x.jit.5: native dispatch end-to-end =====
    // First sub-task that actually executes JIT-compiled code (vs
    // just verifying compile succeeds). Uses safe wrapper
    // `dispatch_integer` to localize the unsafe transmute.

    #[test]
    fn jit5_dispatch_integer_signature_check() {
        // Integer-only signature qualifies.
        let int_fn = make_function_at(
            FuncId(0),
            "ok",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        assert!(super::is_jit_integer_dispatchable(&int_fn));

        // Trilean return disqualifies.
        let trilean_fn = make_function_at(
            FuncId(1),
            "trilean",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Trilean,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        assert!(!super::is_jit_integer_dispatchable(&trilean_fn));

        // 5-arg fn disqualifies (max 4).
        let five_arg_fn = make_function_at(
            FuncId(2),
            "five",
            (0..5)
                .map(|i| (format!("a{i}"), TypeTag::Integer))
                .collect(),
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        assert!(!super::is_jit_integer_dispatchable(&five_arg_fn));
    }

    #[test]
    fn jit5_dispatch_integer_identity() {
        // Compile + dispatch `id(x) = x`. Returns input unchanged.
        let id = make_function_at(
            FuncId(0),
            "id",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![id])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        let result = super::dispatch_integer(&jit, FuncId(0), &[42]);
        assert_eq!(result, Some(42), "identity must return its argument");
    }

    #[test]
    fn jit5_dispatch_integer_two_arg_add() {
        // `add(a, b) = a + b`. Result must match Rust integer add.
        let add = make_function_at(
            FuncId(0),
            "add",
            vec![
                ("a".to_string(), TypeTag::Integer),
                ("b".to_string(), TypeTag::Integer),
            ],
            TypeTag::Integer,
            vec![
                Instruction::Add {
                    dest: ValueId(2),
                    lhs: Operand::Value(ValueId(0)),
                    rhs: Operand::Value(ValueId(1)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(2))),
                },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![add])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        assert_eq!(
            super::dispatch_integer(&jit, FuncId(0), &[3, 4]),
            Some(7),
            "3 + 4 = 7 via JIT"
        );
        assert_eq!(
            super::dispatch_integer(&jit, FuncId(0), &[-10, 25]),
            Some(15),
            "-10 + 25 = 15 via JIT (negative arg handled)"
        );
    }

    #[test]
    fn jit5_dispatch_returns_none_on_uncached_fn() {
        // Empty JIT cache → dispatch is None.
        let jit = JitCompiler::new();
        assert_eq!(super::dispatch_integer(&jit, FuncId(999), &[]), None);
    }

    #[test]
    fn jit5_dispatch_returns_none_on_arity_overflow() {
        // 5+ args refused per supported-arity guard.
        let id = make_function_at(
            FuncId(0),
            "id",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![id])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        // Pass 5 args (signature has 1, but dispatch_integer
        // refuses by arity guard before invoking).
        assert_eq!(
            super::dispatch_integer(&jit, FuncId(0), &[1, 2, 3, 4, 5]),
            None,
            "arity > 4 must be refused"
        );
    }

    #[test]
    fn jit3_program_skips_function_with_unsupported_opcode() {
        // Per ADR-0030 §2 tier-down: a function with an unsupported
        // opcode is skipped, but the rest of the program compiles.
        let ok_fn = make_function_at(
            FuncId(0),
            "ok",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![Instruction::Ret {
                value: Some(Operand::Value(ValueId(0))),
            }],
        );
        let bad_fn = make_function_at(
            FuncId(1),
            "bad",
            vec![],
            TypeTag::Unit,
            vec![
                // ClosureCall is not supported through .3.
                Instruction::ClosureCall {
                    dest: None,
                    closure: Operand::Value(ValueId(99)),
                    args: vec![],
                },
                Instruction::Ret { value: None },
            ],
        );
        let program = make_program(
            vec![make_ir_module(&["khi"], vec![ok_fn, bad_fn])],
            triet_ir::ConstantPool::new(),
        );
        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("program should compile despite per-fn tier-down");
        // `ok` compiled; `bad` did not.
        assert!(
            jit.lookup(FuncId(0)).is_some(),
            "ok function should be cached"
        );
        assert!(
            jit.lookup(FuncId(1)).is_none(),
            "bad function should be skipped"
        );
    }

    // ===== v0.9.x.jit.5: JitDispatcher + Vm integration =====
    // End-to-end: install dispatcher → execute via Vm → counter
    // climbs → at threshold compile fires → subsequent calls run
    // native code.

    use triet_ir::{JitDispatch, RuntimeValue, Vm};

    fn make_increment_program() -> (IrProgram, FuncId) {
        // Two functions:
        //   helper(x) = x + 1     // FuncId(0), Integer-only signature
        //   main(seed) = helper(seed)  // FuncId(1)
        // main is what we drive in the loop so the Vm sees CallLocal
        // to helper repeatedly.
        let mut pool = triet_ir::ConstantPool::new();
        let one = pool.intern(triet_ir::Constant::Integer(
            triet_core::Integer::new(1).unwrap(),
        ));
        let helper = make_function_at(
            FuncId(0),
            "helper",
            vec![("x".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![
                Instruction::Const {
                    dest: ValueId(1),
                    constant: one,
                },
                Instruction::Add {
                    dest: ValueId(2),
                    lhs: Operand::Value(ValueId(0)),
                    rhs: Operand::Value(ValueId(1)),
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(2))),
                },
            ],
        );
        let main = make_function_at(
            FuncId(1),
            "main",
            vec![("seed".to_string(), TypeTag::Integer)],
            TypeTag::Integer,
            vec![
                Instruction::CallLocal {
                    dest: Some(ValueId(1)),
                    callee: FuncId(0),
                    args: vec![Operand::Value(ValueId(0))],
                },
                Instruction::Ret {
                    value: Some(Operand::Value(ValueId(1))),
                },
            ],
        );
        let program = make_program(vec![make_ir_module(&["khi"], vec![helper, main])], pool);
        (program, FuncId(1))
    }

    #[test]
    fn jit5_vm_with_dispatcher_returns_correct_result() {
        // Sanity: Vm with JitDispatcher installed produces the
        // correct numeric result on a single execute call. This is
        // a one-shot check — running execute in a tight loop on the
        // same Vm leaves stale frames per `Vm` semantics (entry
        // frame persists). End-to-end threshold-cross + native-
        // dispatch coverage lives in
        // `jit5_dispatcher_record_call_counts` +
        // `jit5_dispatcher_try_dispatch_returns_some_after_compile`
        // which exercise `JitDispatcher` directly.
        let (program, main_id) = make_increment_program();
        let mut vm = Vm::new(program);
        vm.set_jit_dispatcher(Box::new(JitDispatcher::new()));
        let seed = triet_core::Integer::new(7).unwrap();
        let result = vm
            .execute(main_id, vec![RuntimeValue::Integer(seed)])
            .expect("vm.execute");
        match result {
            RuntimeValue::Integer(out) => assert_eq!(out.to_i64(), 8),
            other => panic!("expected Integer, got {other:?}"),
        }
    }

    #[test]
    fn jit5_dispatcher_record_call_counts() {
        // Manual JitDispatcher test: feed record_call N times, verify
        // compile triggers exactly at threshold + later calls hit
        // cache.
        let (program, _) = make_increment_program();
        let mut dispatcher = JitDispatcher::new();

        // Pre-threshold: record_call doesn't compile.
        for _ in 0..(JIT_THRESHOLD - 1) {
            dispatcher.record_call(FuncId(0), &program);
        }
        assert_eq!(
            dispatcher.compiler().cached_function_count(),
            0,
            "no compile before threshold"
        );

        // Threshold crossing — compile fires.
        dispatcher.record_call(FuncId(0), &program);
        assert!(
            dispatcher.compiler().cached_function_count() >= 1,
            "compile must fire at threshold"
        );

        // Post-compile: subsequent record_calls increment counter but
        // don't re-compile.
        let cached_after = dispatcher.compiler().cached_function_count();
        dispatcher.record_call(FuncId(0), &program);
        assert_eq!(
            dispatcher.compiler().cached_function_count(),
            cached_after,
            "no re-compile after first threshold-crossing"
        );
    }

    #[test]
    fn jit5_dispatcher_try_dispatch_returns_some_after_compile() {
        // After threshold crossing, try_dispatch returns the native
        // result for an eligible function.
        let (program, _) = make_increment_program();
        let mut dispatcher = JitDispatcher::new();
        for _ in 0..JIT_THRESHOLD {
            dispatcher.record_call(FuncId(0), &program);
        }
        // helper(5) = 6.
        let result = dispatcher.try_dispatch(FuncId(0), &[5]);
        assert_eq!(result, Some(6));
    }

    #[test]
    fn jit5_vm_without_dispatcher_works_unchanged() {
        // Default Vm (no JIT installed) still works — JIT path is
        // strictly additive.
        let (program, main_id) = make_increment_program();
        let mut vm = Vm::new(program);
        let seed = triet_core::Integer::new(41).unwrap();
        let result = vm
            .execute(main_id, vec![RuntimeValue::Integer(seed)])
            .expect("vm.execute");
        match result {
            RuntimeValue::Integer(out) => assert_eq!(out.to_i64(), 42),
            other => panic!("expected Integer, got {other:?}"),
        }
    }

    #[test]
    fn jit5_disable_jit_clears_dispatcher() {
        // disable_jit removes the dispatcher → fall back to VM-only.
        let (program, main_id) = make_increment_program();
        let mut vm = Vm::new(program);
        vm.set_jit_dispatcher(Box::new(JitDispatcher::new()));
        vm.disable_jit();
        let seed = triet_core::Integer::new(99).unwrap();
        let result = vm
            .execute(main_id, vec![RuntimeValue::Integer(seed)])
            .expect("vm.execute");
        match result {
            RuntimeValue::Integer(out) => assert_eq!(out.to_i64(), 100),
            other => panic!("expected Integer, got {other:?}"),
        }
    }

    // ===== v0.10.x.jit.1: Layer A framework smoke tests (ADR-0032 §7.1) =====
    //
    // These exercise the shim-infrastructure mechanisms in isolation —
    // symbol registration + external-call codegen, catch_unwind → VmError,
    // drop_arc refcount balance, and capability-denied tier-down — WITHOUT
    // requiring any of the 43 production shims (those land in jit.2).

    use crate::shims::{self, AbiScalar, ShimEntry, ShimSignature};

    /// Framework self-test shim: identity over one `i64`. `extern
    /// "C-unwind"` per ADR-0032 §4. Referenced by address (registered
    /// via `JITBuilder::symbol`), so no `#[no_mangle]` needed. Never
    /// panics — exercises the symbol-registration + external-call
    /// codegen path without touching the deferred §4 unwind mechanism.
    extern "C-unwind" fn test_shim_identity(x: i64) -> i64 {
        x
    }

    fn entry(symbol: &'static str, addr: usize, sig: ShimSignature) -> ShimEntry {
        ShimEntry {
            builtin: None,
            symbol,
            addr,
            signature: sig,
        }
    }

    #[test]
    fn framework_shim_call_returns_value() {
        // Register `__triet_test_identity`, build a JIT caller that
        // forwards its i64 param to it, dispatch, assert round-trip.
        let sig = ShimSignature {
            params: &[AbiScalar::I64],
            ret: Some(AbiScalar::I64),
        };
        let shim = entry(
            "__triet_test_identity",
            test_shim_identity as *const () as usize,
            sig,
        );
        let mut jit = JitCompiler::new();
        jit.cache_shim_caller(FuncId(0), &[shim], &sig, "__triet_test_identity", &sig)
            .expect("compile shim caller");
        // Dispatch via the numeric `dispatch_integer` path — the shim
        // never panics, so the deferred §4 catch-unwind wrapper is not
        // needed to validate symbol-registration + external-call codegen.
        let result = dispatch_integer(&jit, FuncId(0), &[42]).expect("cache hit");
        assert_eq!(result, 42);
    }

    // NOTE: `framework_shim_panic_to_vm_error` (ADR-0032 §7.1) is
    // DEFERRED with the §4 error-propagation mechanism — it requires
    // catch_unwind across a Cranelift JIT frame, blocked on
    // cranelift-jit 0.132 (no system unwind-table registration). The
    // test lands when the ADR-0032 Addendum resolves the redesign.

    #[test]
    fn framework_drop_arc_balances_refcount() {
        use std::rc::Rc;
        use triet_ir::RuntimeValue;

        // Box a composite via Rc::into_raw, then hand the raw pointer
        // to __triet_drop_arc — the strong count must return to the
        // pre-box level, and the original handle stays valid.
        let original = Rc::new(RuntimeValue::String("framework".to_owned()));
        assert_eq!(Rc::strong_count(&original), 1);

        // Simulate a box-out: clone (count → 2), leak the clone as the
        // JIT-side owned pointer (into_raw consumes the +1 without
        // dropping → count stays 2).
        let boxed = Rc::clone(&original);
        let raw = Rc::into_raw(boxed) as i64;
        assert_eq!(Rc::strong_count(&original), 2);

        // drop_arc reconstitutes + drops the leaked Rc → count back to 1.
        shims::__triet_drop_arc(raw);
        assert_eq!(Rc::strong_count(&original), 1);

        // Null pointer is a no-op (does not panic, does not touch count).
        shims::__triet_drop_arc(0);
        assert_eq!(Rc::strong_count(&original), 1);
    }

    #[test]
    fn framework_capability_denied_tiers_down() {
        // A function calling AtomicNew, compiled with `sys.atomic`
        // denied, must surface BuiltinCapabilityDenied (not the
        // generic UnsupportedOpcode tier-down).
        use triet_ir::{
            BasicBlock, BlockId, BuiltinName, Constant, Function, IrModule, IrProgram, Operand,
            TypeTag, ValueId,
        };
        let mut constants = triet_ir::ConstantPool::new();
        let zero = constants.intern(Constant::Integer(triet_core::Integer::new(0).unwrap()));
        let func = Function {
            id: FuncId(0),
            name: Some("uses_atomic".to_owned()),
            params: vec![],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".to_owned()),
                instructions: vec![
                    Instruction::Const {
                        dest: ValueId(0),
                        constant: zero,
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(1)),
                        name: BuiltinName::AtomicNew,
                        args: vec![Operand::Value(ValueId(0))],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(0))),
                    },
                ],
            }],
        };
        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::new(vec!["test".to_owned()]),
                    String::new(),
                ),
                functions: vec![func],
            }],
            constants,
            witness_tables: Vec::new(),
        };

        // compile_program_denied silently tiers-down per-function errors
        // (the function is dropped from the cache), so we assert via the
        // direct capability-check helper that the namespace maps + denies
        // correctly — the mechanism the codegen consults.
        let mut jit = JitCompiler::new();
        jit.compile_program_denied(&program, &["sys.atomic"])
            .expect("program-level compile (per-function tier-down is silent)");
        // The function must NOT be cached (it tiered down on the denied
        // builtin).
        assert!(
            jit.lookup(FuncId(0)).is_none(),
            "function calling a denied builtin must tier down (not cached)"
        );
        // And the check helper surfaces the precise diagnostic.
        let err = crate::check_builtin_capability(BuiltinName::AtomicNew, &["sys.atomic"])
            .expect_err("sys.atomic denied");
        match err {
            JitError::BuiltinCapabilityDenied { builtin, namespace } => {
                assert!(builtin.contains("atomic"), "builtin name: {builtin}");
                assert_eq!(namespace, "sys.atomic");
            }
            other => panic!("expected BuiltinCapabilityDenied, got {other:?}"),
        }
    }

    // ===== v0.10.x.jit.2a: composite-flow + shim end-to-end (ADR-0032) =====
    //
    // Build a single-function IR program whose body calls one builtin
    // shim, JIT-compile it, dispatch via `dispatch_with_shim_errors`,
    // and assert the result / error path. Validates the full composite
    // ABI (box-in / borrow / box-out) + §4 option-2 error propagation.

    // Most IR types are already imported elsewhere in the test module
    // (`BasicBlock`/`Function`/`Operand`/`TypeTag`/`ValueId` at ~630;
    // `RuntimeValue` at ~1457). `Constant` is the only new one here;
    // `IrModule`/`IrProgram` are referenced via `triet_ir::` paths.
    use triet_ir::Constant;

    fn single_fn_program(func: Function, constants: triet_ir::ConstantPool) -> triet_ir::IrProgram {
        triet_ir::IrProgram {
            modules: vec![triet_ir::IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::new(vec!["test".to_owned()]),
                    String::new(),
                ),
                functions: vec![func],
            }],
            constants,
            witness_tables: Vec::new(),
        }
    }

    #[test]
    fn jit_text_len_via_shim() {
        // `text_len_worker(s: String) -> Integer { TextLen(s) }`.
        // Composite arg (String ptr) → primitive return.
        let func = Function {
            id: FuncId(0),
            name: Some("text_len_worker".to_owned()),
            params: vec![("s".to_owned(), TypeTag::String)],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".to_owned()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(1)),
                        name: BuiltinName::TextLen,
                        args: vec![Operand::Value(ValueId(0))],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(1))),
                    },
                ],
            }],
        };
        let program = single_fn_program(func, triet_ir::ConstantPool::new());
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        assert!(jit.lookup(FuncId(0)).is_some(), "text_len_worker must JIT");

        let s_ptr = shims::box_for_jit_test(RuntimeValue::String("Triết!".to_owned()));
        let result = dispatch_with_shim_errors(&jit, FuncId(0), &[s_ptr], "text_len_worker")
            .expect("cache hit")
            .expect("no shim failure");
        assert_eq!(result, 6, "char count of \"Triết!\""); // T r i ế t !
        shims::drop_for_jit_test(s_ptr);
    }

    #[test]
    fn jit_vector_new_via_shim() {
        // `make_vec() -> Vector { VectorNew() }`. Composite box-out
        // return; the i64 result is a boxed empty Vector ptr.
        let func = Function {
            id: FuncId(0),
            name: Some("make_vec".to_owned()),
            params: vec![],
            return_type: TypeTag::Vector(Box::new(TypeTag::Integer)),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".to_owned()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(0)),
                        name: BuiltinName::VectorNew,
                        args: vec![],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(0))),
                    },
                ],
            }],
        };
        let program = single_fn_program(func, triet_ir::ConstantPool::new());
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");

        let result_ptr = dispatch_with_shim_errors(&jit, FuncId(0), &[], "make_vec")
            .expect("cache hit")
            .expect("no shim failure");
        // The returned i64 is a boxed empty Vector — verify + drop it.
        assert_ne!(result_ptr, 0);
        shims::drop_for_jit_test(result_ptr);
    }

    #[test]
    fn jit_assert_false_propagates_error() {
        // `assert_worker(x: Integer) -> Integer { Assert(False, null); x }`.
        // The failing Assert records a VmError + sets SHIM_FAILED; in
        // jit.2a's single-shim-call scope the function still runs to
        // `ret x`, and the dispatcher's boundary TLS check converts the
        // recorded error to `Err` (per ADR-0032 §4 option-2).
        let mut constants = triet_ir::ConstantPool::new();
        let false_c = constants.intern(Constant::Trilean(triet_logic::Trilean::False));
        // Integer 0 doubles as the null msg pointer (no message).
        let null_msg = constants.intern(Constant::Integer(triet_core::Integer::new(0).unwrap()));
        let func = Function {
            id: FuncId(0),
            name: Some("assert_worker".to_owned()),
            params: vec![("x".to_owned(), TypeTag::Integer)],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".to_owned()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: None,
                        name: BuiltinName::Assert,
                        args: vec![Operand::Const(false_c), Operand::Const(null_msg)],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(0))),
                    },
                ],
            }],
        };
        let program = single_fn_program(func, constants);
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        assert!(jit.lookup(FuncId(0)).is_some(), "assert_worker must JIT");

        let result =
            dispatch_with_shim_errors(&jit, FuncId(0), &[7], "assert_worker").expect("cache hit");
        match result {
            Err(VmError::AssertionFailed { function, .. }) => {
                assert_eq!(function, "assert_worker");
            }
            other => panic!("expected AssertionFailed via boundary check, got {other:?}"),
        }
    }

    #[test]
    fn jit_assert_true_no_error() {
        // Same shape, cond=True — no failure, function returns x.
        let mut constants = triet_ir::ConstantPool::new();
        let true_c = constants.intern(Constant::Trilean(triet_logic::Trilean::True));
        let null_msg = constants.intern(Constant::Integer(triet_core::Integer::new(0).unwrap()));
        let func = Function {
            id: FuncId(0),
            name: Some("assert_ok".to_owned()),
            params: vec![("x".to_owned(), TypeTag::Integer)],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".to_owned()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: None,
                        name: BuiltinName::Assert,
                        args: vec![Operand::Const(true_c), Operand::Const(null_msg)],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(0))),
                    },
                ],
            }],
        };
        let program = single_fn_program(func, constants);
        let mut jit = JitCompiler::new();
        jit.compile_program(&program).expect("compile");
        let result = dispatch_with_shim_errors(&jit, FuncId(0), &[42], "assert_ok")
            .expect("cache hit")
            .expect("no shim failure");
        assert_eq!(result, 42);
    }

    #[test]
    fn jit_two_shim_calls_tier_down() {
        // jit.2a single-shim-call scope: a 2nd shim call in one function
        // must tier down (per-call sentinel codegen defers jit.2b). The
        // function is dropped from the cache.
        let func = Function {
            id: FuncId(0),
            name: Some("two_lens".to_owned()),
            params: vec![("s".to_owned(), TypeTag::String)],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".to_owned()),
                instructions: vec![
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(1)),
                        name: BuiltinName::TextLen,
                        args: vec![Operand::Value(ValueId(0))],
                    },
                    Instruction::CallBuiltin {
                        dest: Some(ValueId(2)),
                        name: BuiltinName::TextLen,
                        args: vec![Operand::Value(ValueId(0))],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(2))),
                    },
                ],
            }],
        };
        let program = single_fn_program(func, triet_ir::ConstantPool::new());
        let mut jit = JitCompiler::new();
        jit.compile_program(&program)
            .expect("compile (tier-down is silent)");
        assert!(
            jit.lookup(FuncId(0)).is_none(),
            "2-shim-call function must tier down in jit.2a"
        );
    }
}

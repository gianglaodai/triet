//! v0.10.x.jit.1 — Builtin shim infrastructure per [ADR-0032].
//!
//! This module ships the **framework** the v0.10.x.jit.2 production
//! shims build on — NOT the 43 production shims themselves (those land
//! in jit.2). Concretely:
//!
//! - [`AbiScalar`] / [`ShimSignature`] — describe a shim's Cranelift
//!   arg/ret ABI without embedding Cranelift types in static data
//!   (kept decoupled so the registry table is plain data).
//! - [`ShimEntry`] + [`production_shim_entries`] — the registry that
//!   [`crate::codegen::JitBackend::new`] walks to wire shim symbols via
//!   `JITBuilder::symbol`. jit.1 registers only `__triet_drop_arc`;
//!   jit.2 appends the 43 production shims.
//! - [`builtin_namespace`] — the static `BuiltinName → capability
//!   namespace` table used for the §3 compile-time capability check
//!   (defense-in-depth; the real gate is upstream at program-load time
//!   per ADR-0016 §5).
//! - [`__triet_drop_arc`] — the lifetime-management shim (§2): consumes
//!   an `Rc::into_raw` pointer at a value's last use.
//!
//! **§4 error propagation DEFERRED:** the shim-panic → `VmError`
//! mechanism (ADR-0032 §4) is blocked on `cranelift-jit 0.132` (no
//! system unwind-table registration for JIT'd frames) — see the
//! "Error propagation" section below + the ADR-0032 Addendum.
//!
//! **`unsafe` is localized here + in [`crate::codegen`]** per ADR-0032
//! §5 — the crate-local `unsafe_code = "deny"` override (Cargo.toml)
//! permits per-item `#[allow(unsafe_code)]` at documented sites.
//!
//! [ADR-0032]: ../../../docs/decisions/0032-builtin-shim-abi.md
//! [ADR-0016 §5]: ../../../docs/decisions/0016-capability-type-system.md

#![allow(clippy::redundant_pub_crate)]

use std::rc::Rc;

use triet_ir::{BuiltinName, RuntimeValue};

// ── ABI description (decoupled from Cranelift types) ────────────────

/// A scalar slot in a shim's ABI signature. Maps to a Cranelift type
/// at registration time (see [`crate::codegen`]). Decoupled so the
/// static [`ShimEntry`] table stays plain data — no Cranelift type
/// values baked into `static`/`const` context.
///
/// `dead_code`-allowed in jit.1: only `Ptr` is constructed in the
/// production `production_shim_entries` (the `drop_arc` shim). The
/// `I8`/`I16`/`I64` variants describe the 43 production shims'
/// primitive ABI slots — constructed by v0.10.x.jit.2 + the framework
/// test. Not speculative: mandated by ADR-0032 §1 hybrid ABI table.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AbiScalar {
    /// `Trit` / `Trilean` / `Unit` — `i8` per ADR-0030 §3.
    I8,
    /// `Tryte` — `i16`.
    I16,
    /// `Integer` — `i64`.
    I64,
    /// Composite pointer (`String`/`Vector`/`HashMap`/`Atomic`/etc.) —
    /// `i64`-wide raw pointer per ADR-0032 §1 hybrid table.
    Ptr,
}

/// A shim's argument + return ABI shape per ADR-0032 §1 hybrid table.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShimSignature {
    /// Parameter slots, in order.
    pub params: &'static [AbiScalar],
    /// Return slot — `None` for `Unit`-returning shims (which the
    /// caller treats as discarding an `i8 0` placeholder per §1).
    pub ret: Option<AbiScalar>,
}

/// One registry entry binding a [`BuiltinName`] (or framework shim) to
/// its `extern "C-unwind"` symbol + Rust function address + ABI shape +
/// capability namespace. Walked at JIT init to wire symbols.
///
/// `addr` is stored as `usize` (not `*const u8`) so the table can live
/// in a `Vec` built once at init without `!Sync` raw-pointer friction.
/// The `usize` is cast back to `*const u8` at the single registration
/// site in [`crate::codegen`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShimEntry {
    /// The `__triet_*`-prefixed symbol name per ADR-0032 §6.
    pub symbol: &'static str,
    /// Rust function address (`fn as usize`).
    pub addr: usize,
    /// ABI signature for Cranelift declaration. Read by
    /// `compile_shim_caller` (framework test) + v0.10.x.jit.2's
    /// production `CallBuiltin` codegen when declaring the shim's
    /// imported signature; `dead_code`-allowed in jit.1 where only
    /// `symbol` + `addr` are consumed (symbol registration).
    #[allow(dead_code)]
    pub signature: ShimSignature,
}

/// Framework + production shim registry. jit.1 returns ONLY the
/// `__triet_drop_arc` lifetime shim; jit.2 appends the 43 production
/// builtin shims (one [`ShimEntry`] per [`BuiltinName`]). Built once at
/// [`crate::codegen::JitBackend::new`] and registered via
/// `JITBuilder::symbol`.
pub(crate) fn production_shim_entries() -> Vec<ShimEntry> {
    vec![ShimEntry {
        symbol: "__triet_drop_arc",
        // Cast via `*const ()` per clippy `function_casts_as_integer` —
        // fn item → fn pointer → usize. The usize is recovered to
        // `*const u8` at registration (a no-op address round-trip).
        addr: __triet_drop_arc as *const () as usize,
        signature: ShimSignature {
            params: &[AbiScalar::Ptr],
            ret: None,
        },
    }]
}

// ── Capability namespace table (§3 defense-in-depth) ────────────────

/// Map a [`BuiltinName`] to its capability namespace string per the
/// `path_to_builtin` roots in `triet-ir::vm`. Used by the JIT's §3
/// compile-time capability check.
///
/// **Defense-in-depth only:** the authoritative capability gate runs
/// at program-load time (ADR-0016 §5) — by the time a `CallBuiltin`
/// reaches the JIT, the namespace was already granted (else the
/// lowerer would not have emitted the opcode). This table lets the
/// JIT re-assert the invariant cheaply at compile time, and lets the
/// framework test exercise the denied-tier-down path. `std.*`
/// namespaces are ambient (never gated in practice); `sys.*` are the
/// real gateable ones (`sys.atomic`, `sys.raw_thread`).
pub(crate) const fn builtin_namespace(builtin: BuiltinName) -> &'static str {
    match builtin {
        BuiltinName::Println | BuiltinName::Print => "std.io",
        BuiltinName::Assert | BuiltinName::AssertEq => "std.assert",
        BuiltinName::FStringConcat
        | BuiltinName::TextLen
        | BuiltinName::TextConcat
        | BuiltinName::TextFromInteger
        | BuiltinName::ParseInteger
        | BuiltinName::TextIntoBytes
        | BuiltinName::TextFromBytes => "std.text",
        BuiltinName::VectorNew
        | BuiltinName::VectorPush
        | BuiltinName::VectorGet
        | BuiltinName::VectorLength => "std.collections.vector",
        BuiltinName::HashMapNew
        | BuiltinName::HashMapInsert
        | BuiltinName::HashMapGet
        | BuiltinName::HashMapKeys
        | BuiltinName::HashMapContains => "std.collections.hashmap",
        BuiltinName::ReadFile
        | BuiltinName::WriteFile
        | BuiltinName::WriteFileBytes
        | BuiltinName::FileExists
        | BuiltinName::ReadDirRecursive => "std.io.fs",
        BuiltinName::PathJoin | BuiltinName::PathParent | BuiltinName::PathBasename => "std.path",
        BuiltinName::StringSubstring | BuiltinName::StringSplit | BuiltinName::StringIndexOf => {
            "std.string"
        }
        BuiltinName::Blake3Hash => "std.crypto",
        BuiltinName::GetEnv => "std.env",
        BuiltinName::AtomicNew
        | BuiltinName::AtomicLoad
        | BuiltinName::AtomicStore
        | BuiltinName::AtomicSwap
        | BuiltinName::AtomicCompareExchange
        | BuiltinName::AtomicFetchAdd
        | BuiltinName::AtomicFetchSub
        | BuiltinName::AtomicFetchBitwiseAnd
        | BuiltinName::AtomicFetchBitwiseOr
        | BuiltinName::AtomicFetchBitwiseXor => "sys.atomic",
        BuiltinName::RawThreadSpawn | BuiltinName::RawThreadJoin => "sys.raw_thread",
    }
}

// ── Lifetime-management shim (§2) ───────────────────────────────────

/// Drop a composite value boxed for the JIT↔shim ABI boundary per
/// ADR-0032 §2. The JIT emits a call to this shim at a boxed SSA
/// value's last use, consuming the `Rc::into_raw` pointer exactly once.
///
/// Null-safe: `ptr == 0` (a `T?` null arm or sentinel) is a no-op.
///
/// **No `#[unsafe(no_mangle)]` at jit.1:** in-process JIT resolves this
/// shim by the explicit address registered via `JITBuilder::symbol`
/// (see [`production_shim_entries`]), so the symbol need not be
/// exported by name. v0.10.x.jit.3 AOT cache (ELF object emission)
/// adds `no_mangle` when name-based load-time resolution becomes
/// required, per ADR-0033 §3.
pub(crate) extern "C-unwind" fn __triet_drop_arc(ptr: i64) {
    if ptr == 0 {
        return;
    }
    // SAFETY: `ptr` originates from `Rc::into_raw(Rc::new(value))` at a
    // composite box-out site in JIT codegen (ADR-0032 §2 rule 2). The
    // lowerer's ValueKind last-use tracking (ADR-0023) guarantees this
    // pointer is consumed exactly once — no double-free, no use-after.
    // Reconstituting the `Rc` and dropping it balances the `into_raw`.
    // Backed by ADR-0032 §2.
    #[allow(unsafe_code)]
    unsafe {
        let _ = Rc::from_raw(ptr as *const RuntimeValue);
    }
}

// ── Error propagation (§4) — DEFERRED per ADR-0032 Addendum ─────────
//
// ADR-0032 §4 locked `extern "C-unwind"` + dispatcher `catch_unwind`
// for shim-panic → `VmError` propagation. v0.10.x.jit.1 implementation
// discovered this is BLOCKED on `cranelift-jit 0.132`: that backend
// does not register system DWARF unwind tables (`.eh_frame` via
// `__register_frame`) for JIT'd code, so a panic unwinding THROUGH a
// Cranelift-compiled frame aborts (`failed to initiate panic`) instead
// of reaching the dispatcher's `catch_unwind`.
//
// The error-propagation mechanism (thread-local error slot + dispatch
// wrapper) therefore defers to the ADR-0032 Addendum, which records
// the cliff + three redesign options (shim-internal catch + sentinel /
// per-call sentinel check in codegen / Cranelift unwind-table
// registration). The option-agnostic pieces — shim registry, lifetime
// `drop_arc`, capability table, symbol wiring — ship in jit.1; the
// error mechanism lands once the Addendum is resolved (jit.2-adjacent).

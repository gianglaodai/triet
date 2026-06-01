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
//! - Error-propagation substrate (§4 option-2, v0.10.x.jit.2a):
//!   thread-local `VmError` slot + `SHIM_FAILED` flag +
//!   [`record_shim_failure`] / [`take_shim_failure`] /
//!   [`__triet_shim_failed`] per-call probe. Shims are `extern "C"`
//!   (never unwind out); the JIT emits a sentinel check after each
//!   call. Replaces §4's blocked `catch_unwind`-across-JIT approach.
//!
//! **`unsafe` is localized here + in [`crate::codegen`]** per ADR-0032
//! §5 — the crate-local `unsafe_code = "deny"` override (Cargo.toml)
//! permits per-item `#[allow(unsafe_code)]` at documented sites.
//!
//! [ADR-0032]: ../../../docs/decisions/0032-builtin-shim-abi.md
//! [ADR-0016 §5]: ../../../docs/decisions/0016-capability-type-system.md

#![allow(clippy::redundant_pub_crate)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use triet_core::Integer;
use triet_ir::{
    BuiltinName, JitBinOp, JitConstKind, RuntimeValue, VmError, dispatch_builtin, exec_box_const,
    exec_enum_new, exec_enum_payload, exec_enum_tag, exec_field_get, exec_field_set,
    exec_jit_binop, exec_jit_neg, exec_null_check, exec_null_unwrap, exec_null_wrap,
    exec_outcome_discriminant, exec_outcome_new_negative, exec_outcome_new_null,
    exec_outcome_new_positive, exec_outcome_unwrap_error, exec_outcome_unwrap_value,
    exec_struct_new, exec_trilean_tag,
};
use triet_logic::Trilean;

/// The builtin-shim ABI version baked into every AOT cache manifest
/// (v0.11.x.jit.3, ADR-0033 §2).
///
/// The Path-A loader refuses a cache whose recorded `shim_abi_version`
/// differs from this constant (silent fallback to fresh compile +
/// overwrite), so a cache compiled against an older shim ABI can never
/// be loaded against a newer one.
///
/// **Bump this manually on ANY ADR-0032 ABI break** — e.g. adding or
/// removing a [`BuiltinName`] shim, changing the hybrid ABI of an
/// existing shim ([`AbiScalar`] slot layout), or altering the §4
/// error-sentinel contract. A bump = global invalidation of every
/// cache entry that references an affected builtin, mirroring the
/// semver-style `iface_hash` discipline of ADR-0013.
pub const SHIM_ABI_VERSION: u32 = 1;

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
    /// The [`BuiltinName`] this shim implements, or `None` for
    /// framework shims (`__triet_drop_arc`, `__triet_shim_failed`)
    /// that the codegen calls implicitly rather than via `CallBuiltin`.
    pub builtin: Option<BuiltinName>,
    /// The `__triet_*`-prefixed symbol name per ADR-0032 §6.
    pub symbol: &'static str,
    /// Rust function address (`fn as usize`).
    pub addr: usize,
    /// ABI signature for Cranelift declaration + per-arg marshaling
    /// (primitive vs composite-pointer) per ADR-0032 §1.
    pub signature: ShimSignature,
}

/// v0.10.x.jit.2a — Look up the production [`ShimEntry`] for a
/// `CallBuiltin` opcode's [`BuiltinName`], or `None` if no shim is
/// implemented yet (the function then tier-downs to VM dispatch). The
/// codegen drives argument marshaling from the returned entry's
/// [`ShimSignature`].
pub(crate) fn builtin_shim(name: BuiltinName) -> Option<ShimEntry> {
    production_shim_entries()
        .into_iter()
        .find(|e| e.builtin == Some(name))
}

/// v0.11.x.jit.4.agg.1 — look up a registered shim by its `__triet_*`
/// symbol. Used by the boxed codegen to emit calls to the aggregate-op
/// shims (`builtin: None` entries) it doesn't reach via `CallBuiltin`.
pub(crate) fn shim_entry_by_symbol(symbol: &str) -> Option<ShimEntry> {
    production_shim_entries()
        .into_iter()
        .find(|e| e.symbol == symbol)
}

/// Framework + production shim registry. jit.1 returns ONLY the
/// `__triet_drop_arc` lifetime shim; jit.2 appends the 43 production
/// builtin shims (one [`ShimEntry`] per [`BuiltinName`]). Built once at
/// [`crate::codegen::JitBackend::new`] and registered via
/// `JITBuilder::symbol`.
// A flat registry table — one line-group per shim. Length is inherent
// to the 26-entry breadth, not complexity; splitting by category would
// fragment the single audit surface ADR-0032 §6 wants.
#[allow(clippy::too_many_lines)]
pub(crate) fn production_shim_entries() -> Vec<ShimEntry> {
    use AbiScalar::{I8, I64, Ptr};

    // Builtin shim entry. `addr` cast via `*const ()` per clippy
    // `function_casts_as_integer`; recovered to `*const u8` at
    // registration (a no-op address round-trip).
    const fn b(
        builtin: BuiltinName,
        symbol: &'static str,
        addr: usize,
        params: &'static [AbiScalar],
        ret: Option<AbiScalar>,
    ) -> ShimEntry {
        ShimEntry {
            builtin: Some(builtin),
            symbol,
            addr,
            signature: ShimSignature { params, ret },
        }
    }

    vec![
        // ── Framework shims (no BuiltinName — called implicitly) ──
        ShimEntry {
            builtin: None,
            symbol: "__triet_drop_arc",
            addr: __triet_drop_arc as *const () as usize,
            signature: ShimSignature {
                params: &[Ptr],
                ret: None,
            },
        },
        ShimEntry {
            builtin: None,
            symbol: "__triet_shim_failed",
            addr: __triet_shim_failed as *const () as usize,
            signature: ShimSignature {
                params: &[],
                ret: Some(I8),
            },
        },
        // ── Aggregate-opcode shims (ADR-0034 §1/§2, builtin: None —
        // called by boxed codegen directly, not via CallBuiltin) ──
        ShimEntry {
            builtin: None,
            // `fields_ptr` (raw stack-slot addr) + `len` are plain i64s;
            // the return is a boxed Struct ptr.
            symbol: "__triet_struct_new",
            addr: __triet_struct_new as *const () as usize,
            signature: ShimSignature {
                params: &[I64, I64],
                ret: Some(Ptr),
            },
        },
        ShimEntry {
            builtin: None,
            symbol: "__triet_field_get",
            addr: __triet_field_get as *const () as usize,
            signature: ShimSignature {
                params: &[Ptr, I64],
                ret: Some(Ptr),
            },
        },
        ShimEntry {
            builtin: None,
            symbol: "__triet_field_set",
            addr: __triet_field_set as *const () as usize,
            signature: ShimSignature {
                params: &[Ptr, I64, Ptr],
                ret: Some(Ptr),
            },
        },
        ShimEntry {
            builtin: None,
            // op discriminant (i8) + two boxed operands → boxed result.
            symbol: "__triet_binop",
            addr: __triet_binop as *const () as usize,
            signature: ShimSignature {
                params: &[I8, Ptr, Ptr],
                ret: Some(Ptr),
            },
        },
        ShimEntry {
            builtin: None,
            symbol: "__triet_neg",
            addr: __triet_neg as *const () as usize,
            signature: ShimSignature {
                params: &[Ptr],
                ret: Some(Ptr),
            },
        },
        ShimEntry {
            builtin: None,
            // const kind discriminant (i8) + scalar payload (i64).
            symbol: "__triet_box_const",
            addr: __triet_box_const as *const () as usize,
            signature: ShimSignature {
                params: &[I8, I64],
                ret: Some(Ptr),
            },
        },
        ShimEntry {
            builtin: None,
            // boxed ptr + kind (i8) → raw scalar bits (i64). Cross-mode
            // marshaling (agg.cross-call); inverse of __triet_box_const.
            symbol: "__triet_unbox_scalar",
            addr: __triet_unbox_scalar as *const () as usize,
            signature: ShimSignature {
                params: &[Ptr, I8],
                ret: Some(I64),
            },
        },
        ShimEntry {
            builtin: None,
            // boxed branch cond → its three-way tag (i8 `{-1,0,+1}`).
            symbol: "__triet_trilean_tag",
            addr: __triet_trilean_tag as *const () as usize,
            signature: ShimSignature {
                params: &[Ptr],
                ret: Some(I8),
            },
        },
        // ── Enum-opcode shims (ADR-0034 agg.2a, builtin: None) ──
        ShimEntry {
            builtin: None,
            // variant (i64) + has_payload flag (i8) + payload ptr → Enum.
            symbol: "__triet_enum_new",
            addr: __triet_enum_new as *const () as usize,
            signature: ShimSignature {
                params: &[I64, I8, Ptr],
                ret: Some(Ptr),
            },
        },
        ShimEntry {
            builtin: None,
            // scrutinee ptr → boxed Integer variant index.
            symbol: "__triet_enum_tag",
            addr: __triet_enum_tag as *const () as usize,
            signature: ShimSignature {
                params: &[Ptr],
                ret: Some(Ptr),
            },
        },
        ShimEntry {
            builtin: None,
            // scrutinee ptr → boxed payload (or failure sentinel).
            symbol: "__triet_enum_payload",
            addr: __triet_enum_payload as *const () as usize,
            signature: ShimSignature {
                params: &[Ptr],
                ret: Some(Ptr),
            },
        },
        // ── Outcome-opcode shims (ADR-0034 agg.2b, builtin: None) ──
        ShimEntry {
            builtin: None,
            // payload ptr → boxed Outcome (Positive arm).
            symbol: "__triet_outcome_new_positive",
            addr: __triet_outcome_new_positive as *const () as usize,
            signature: ShimSignature {
                params: &[Ptr],
                ret: Some(Ptr),
            },
        },
        ShimEntry {
            builtin: None,
            // payload ptr → boxed Outcome (Negative arm).
            symbol: "__triet_outcome_new_negative",
            addr: __triet_outcome_new_negative as *const () as usize,
            signature: ShimSignature {
                params: &[Ptr],
                ret: Some(Ptr),
            },
        },
        ShimEntry {
            builtin: None,
            // (no args) → boxed Outcome (Zero/null arm).
            symbol: "__triet_outcome_new_null",
            addr: __triet_outcome_new_null as *const () as usize,
            signature: ShimSignature {
                params: &[],
                ret: Some(Ptr),
            },
        },
        ShimEntry {
            builtin: None,
            // source ptr → boxed Trit discriminant. Total.
            symbol: "__triet_outcome_discriminant",
            addr: __triet_outcome_discriminant as *const () as usize,
            signature: ShimSignature {
                params: &[Ptr],
                ret: Some(Ptr),
            },
        },
        ShimEntry {
            builtin: None,
            // source ptr → boxed success payload (or failure sentinel).
            symbol: "__triet_outcome_unwrap_value",
            addr: __triet_outcome_unwrap_value as *const () as usize,
            signature: ShimSignature {
                params: &[Ptr],
                ret: Some(Ptr),
            },
        },
        ShimEntry {
            builtin: None,
            // source ptr → boxed failure payload (or failure sentinel).
            symbol: "__triet_outcome_unwrap_error",
            addr: __triet_outcome_unwrap_error as *const () as usize,
            signature: ShimSignature {
                params: &[Ptr],
                ret: Some(Ptr),
            },
        },
        // ── Nullable-opcode shims (ADR-0034 agg.3a, builtin: None) ──
        ShimEntry {
            builtin: None,
            // value ptr → boxed Some-carrier (Enum variant 0).
            symbol: "__triet_null_wrap",
            addr: __triet_null_wrap as *const () as usize,
            signature: ShimSignature {
                params: &[Ptr],
                ret: Some(Ptr),
            },
        },
        ShimEntry {
            builtin: None,
            // nullable ptr → boxed value (or NullUnwrap failure sentinel).
            symbol: "__triet_null_unwrap",
            addr: __triet_null_unwrap as *const () as usize,
            signature: ShimSignature {
                params: &[Ptr],
                ret: Some(Ptr),
            },
        },
        ShimEntry {
            builtin: None,
            // nullable ptr → boxed Trit discriminant. Total.
            symbol: "__triet_null_check",
            addr: __triet_null_check as *const () as usize,
            signature: ShimSignature {
                params: &[Ptr],
                ret: Some(Ptr),
            },
        },
        // ── Production builtin shims — all delegate semantics to
        // `triet_ir::dispatch_builtin` (single source of truth). ──
        // I/O + assert (Unit returns).
        b(
            BuiltinName::Assert,
            "__triet_assert",
            __triet_assert as *const () as usize,
            &[I8, Ptr],
            Some(I8),
        ),
        b(
            BuiltinName::AssertEq,
            "__triet_assert_eq",
            __triet_assert_eq as *const () as usize,
            &[Ptr, Ptr],
            Some(I8),
        ),
        b(
            BuiltinName::Println,
            "__triet_println",
            __triet_println as *const () as usize,
            &[Ptr],
            Some(I8),
        ),
        b(
            BuiltinName::Print,
            "__triet_print",
            __triet_print as *const () as usize,
            &[Ptr],
            Some(I8),
        ),
        // Text.
        b(
            BuiltinName::TextLen,
            "__triet_text_len",
            __triet_text_len as *const () as usize,
            &[Ptr],
            Some(I64),
        ),
        b(
            BuiltinName::TextFromInteger,
            "__triet_text_from_integer",
            __triet_text_from_integer as *const () as usize,
            &[I64],
            Some(Ptr),
        ),
        b(
            BuiltinName::ParseInteger,
            "__triet_parse_integer",
            __triet_parse_integer as *const () as usize,
            &[Ptr],
            Some(Ptr),
        ),
        b(
            BuiltinName::TextIntoBytes,
            "__triet_text_into_bytes",
            __triet_text_into_bytes as *const () as usize,
            &[Ptr],
            Some(Ptr),
        ),
        b(
            BuiltinName::TextFromBytes,
            "__triet_text_from_bytes",
            __triet_text_from_bytes as *const () as usize,
            &[Ptr],
            Some(Ptr),
        ),
        // Vector.
        b(
            BuiltinName::VectorNew,
            "__triet_vector_new",
            __triet_vector_new as *const () as usize,
            &[],
            Some(Ptr),
        ),
        b(
            BuiltinName::VectorPush,
            "__triet_vector_push",
            __triet_vector_push as *const () as usize,
            &[Ptr, Ptr],
            Some(Ptr),
        ),
        b(
            BuiltinName::VectorGet,
            "__triet_vector_get",
            __triet_vector_get as *const () as usize,
            &[Ptr, I64],
            Some(Ptr),
        ),
        b(
            BuiltinName::VectorLength,
            "__triet_vector_length",
            __triet_vector_length as *const () as usize,
            &[Ptr],
            Some(I64),
        ),
        // HashMap.
        b(
            BuiltinName::HashMapNew,
            "__triet_hashmap_new",
            __triet_hashmap_new as *const () as usize,
            &[],
            Some(Ptr),
        ),
        b(
            BuiltinName::HashMapInsert,
            "__triet_hashmap_insert",
            __triet_hashmap_insert as *const () as usize,
            &[Ptr, Ptr, Ptr],
            Some(Ptr),
        ),
        b(
            BuiltinName::HashMapGet,
            "__triet_hashmap_get",
            __triet_hashmap_get as *const () as usize,
            &[Ptr, Ptr],
            Some(Ptr),
        ),
        b(
            BuiltinName::HashMapKeys,
            "__triet_hashmap_keys",
            __triet_hashmap_keys as *const () as usize,
            &[Ptr],
            Some(Ptr),
        ),
        b(
            BuiltinName::HashMapContains,
            "__triet_hashmap_contains",
            __triet_hashmap_contains as *const () as usize,
            &[Ptr, Ptr],
            Some(I8),
        ),
        // Path.
        b(
            BuiltinName::PathJoin,
            "__triet_path_join",
            __triet_path_join as *const () as usize,
            &[Ptr, Ptr],
            Some(Ptr),
        ),
        b(
            BuiltinName::PathParent,
            "__triet_path_parent",
            __triet_path_parent as *const () as usize,
            &[Ptr],
            Some(Ptr),
        ),
        b(
            BuiltinName::PathBasename,
            "__triet_path_basename",
            __triet_path_basename as *const () as usize,
            &[Ptr],
            Some(Ptr),
        ),
        // String.
        b(
            BuiltinName::StringSubstring,
            "__triet_string_substring",
            __triet_string_substring as *const () as usize,
            &[Ptr, I64, I64],
            Some(Ptr),
        ),
        b(
            BuiltinName::StringSplit,
            "__triet_string_split",
            __triet_string_split as *const () as usize,
            &[Ptr, Ptr],
            Some(Ptr),
        ),
        b(
            BuiltinName::StringIndexOf,
            "__triet_string_index_of",
            __triet_string_index_of as *const () as usize,
            &[Ptr, Ptr],
            Some(Ptr),
        ),
        // Misc.
        b(
            BuiltinName::Blake3Hash,
            "__triet_blake3_hash",
            __triet_blake3_hash as *const () as usize,
            &[Ptr],
            Some(Ptr),
        ),
        b(
            BuiltinName::GetEnv,
            "__triet_get_env",
            __triet_get_env as *const () as usize,
            &[Ptr],
            Some(Ptr),
        ),
        // Atomic ×10 (jit.2b-ii) — Atomic<Integer>; `self` + `ordering`
        // are composite ptrs, value/delta/mask are i64. `Arc<Mutex>`
        // repr (thread.2) supersedes ADR-0032 §1 `Rc<RefCell>` text.
        b(
            BuiltinName::AtomicNew,
            "__triet_atomic_new",
            __triet_atomic_new as *const () as usize,
            &[I64],
            Some(Ptr),
        ),
        b(
            BuiltinName::AtomicLoad,
            "__triet_atomic_load",
            __triet_atomic_load as *const () as usize,
            &[Ptr, Ptr],
            Some(I64),
        ),
        b(
            BuiltinName::AtomicStore,
            "__triet_atomic_store",
            __triet_atomic_store as *const () as usize,
            &[Ptr, I64, Ptr],
            Some(I8),
        ),
        b(
            BuiltinName::AtomicSwap,
            "__triet_atomic_swap",
            __triet_atomic_swap as *const () as usize,
            &[Ptr, I64, Ptr],
            Some(I64),
        ),
        b(
            BuiltinName::AtomicCompareExchange,
            "__triet_atomic_compare_exchange",
            __triet_atomic_compare_exchange as *const () as usize,
            &[Ptr, I64, I64, Ptr, Ptr],
            Some(Ptr),
        ),
        b(
            BuiltinName::AtomicFetchAdd,
            "__triet_atomic_fetch_add",
            __triet_atomic_fetch_add as *const () as usize,
            &[Ptr, I64, Ptr],
            Some(I64),
        ),
        b(
            BuiltinName::AtomicFetchSub,
            "__triet_atomic_fetch_sub",
            __triet_atomic_fetch_sub as *const () as usize,
            &[Ptr, I64, Ptr],
            Some(I64),
        ),
        b(
            BuiltinName::AtomicFetchBitwiseAnd,
            "__triet_atomic_fetch_bitwise_and",
            __triet_atomic_fetch_bitwise_and as *const () as usize,
            &[Ptr, I64, Ptr],
            Some(I64),
        ),
        b(
            BuiltinName::AtomicFetchBitwiseOr,
            "__triet_atomic_fetch_bitwise_or",
            __triet_atomic_fetch_bitwise_or as *const () as usize,
            &[Ptr, I64, Ptr],
            Some(I64),
        ),
        b(
            BuiltinName::AtomicFetchBitwiseXor,
            "__triet_atomic_fetch_bitwise_xor",
            __triet_atomic_fetch_bitwise_xor as *const () as usize,
            &[Ptr, I64, Ptr],
            Some(I64),
        ),
        // File I/O ×5 (jit.2b-iii) — fixed-arity delegating shims.
        b(
            BuiltinName::ReadFile,
            "__triet_read_file",
            __triet_read_file as *const () as usize,
            &[Ptr],
            Some(Ptr),
        ),
        b(
            BuiltinName::WriteFile,
            "__triet_write_file",
            __triet_write_file as *const () as usize,
            &[Ptr, Ptr],
            Some(I8),
        ),
        b(
            BuiltinName::WriteFileBytes,
            "__triet_write_file_bytes",
            __triet_write_file_bytes as *const () as usize,
            &[Ptr, Ptr],
            Some(I8),
        ),
        b(
            BuiltinName::FileExists,
            "__triet_file_exists",
            __triet_file_exists as *const () as usize,
            &[Ptr],
            Some(I8),
        ),
        b(
            BuiltinName::ReadDirRecursive,
            "__triet_read_dir_recursive",
            __triet_read_dir_recursive as *const () as usize,
            &[Ptr],
            Some(Ptr),
        ),
    ]
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
pub(crate) extern "C" fn __triet_drop_arc(ptr: i64) {
    if ptr == 0 {
        return;
    }
    // SAFETY: `ptr` originates from `Rc::into_raw(Rc::new(value))` at a
    // composite box-out site in JIT codegen (ADR-0032 §2 rule 2). The
    // JIT's per-function last-use pass (jit.2a, single-block scope)
    // guarantees this pointer is consumed exactly once — no double-free,
    // no use-after. Reconstituting the `Rc` and dropping it balances the
    // `into_raw`. Backed by ADR-0032 §2.
    #[allow(unsafe_code)]
    unsafe {
        let _ = Rc::from_raw(ptr as *const RuntimeValue);
    }
}

// ── Composite ABI helpers (§1/§2) ───────────────────────────────────

/// Box a `RuntimeValue` for the shim-ABI boundary per ADR-0032 §2
/// rule 2 — fresh `Rc` (refcount 1), returned as an `i64` pointer the
/// JIT register owns (and later drops via [`__triet_drop_arc`]).
/// Pure-safe: `Rc::into_raw` + pointer→integer cast are both safe.
fn box_rv(value: RuntimeValue) -> i64 {
    Rc::into_raw(Rc::new(value)) as i64
}

/// v0.10.x.jit.2a (test-support) — box a `RuntimeValue` into a shim-ABI
/// pointer so an integration test can feed a composite argument to a
/// JIT'd function through [`crate::dispatch_with_shim_errors`]. The
/// caller is responsible for the matching [`__triet_drop_arc`].
#[cfg(test)]
pub(crate) fn box_for_jit_test(value: RuntimeValue) -> i64 {
    box_rv(value)
}

/// v0.10.x.jit.2a (test-support) — drop a test-boxed pointer.
#[cfg(test)]
pub(crate) fn drop_for_jit_test(ptr: i64) {
    __triet_drop_arc(ptr);
}

/// v0.11.x.jit.4.agg.1b (test-support) — clone the `RuntimeValue` a boxed
/// shim-ABI pointer points at, so a test can inspect a boxed function's
/// result. Borrows (does not consume) — caller still drops `ptr`.
/// `ptr == 0` → `Null`.
#[cfg(test)]
pub(crate) fn read_for_jit_test(ptr: i64) -> RuntimeValue {
    with_rv(ptr, |rv| rv.cloned().unwrap_or(RuntimeValue::Null))
}

/// v0.10.x.jit.2b-ii (test-support) — read the Integer inside a boxed
/// `Atomic<Integer>` (locks the shared Mutex). For end-to-end atomic
/// JIT assertions.
#[cfg(test)]
pub(crate) fn with_atomic_for_test(ptr: i64, check: impl FnOnce(i64)) {
    with_rv(ptr, |rv| match rv {
        Some(RuntimeValue::Atomic(arc)) => {
            let guard = arc.lock().expect("atomic mutex");
            match &*guard {
                RuntimeValue::Integer(i) => check(i.to_i64()),
                other => panic!("expected Integer inside Atomic, got {other:?}"),
            }
        }
        other => panic!("expected Atomic, got {other:?}"),
    });
}

/// Borrow a boxed `RuntimeValue` for the duration of `f` per ADR-0032
/// §2 rule 1 (borrowed view — refcount unchanged, NOT consumed). The
/// closure scopes the borrow so no dangling reference can escape.
/// `ptr == 0` (null / `T?` null arm) yields `None`.
fn with_rv<R>(ptr: i64, f: impl FnOnce(Option<&RuntimeValue>) -> R) -> R {
    if ptr == 0 {
        return f(None);
    }
    // SAFETY: per ADR-0032 §2 rule 1, `ptr` is a borrowed shim-ABI
    // pointer from `Rc::into_raw(Rc::new(RuntimeValue))` in JIT codegen;
    // it points to a live `RuntimeValue` for the shim-call duration (the
    // JIT register holds the +1 refcount). We borrow within `f`'s scope
    // only — no consume, no escape. Backed by ADR-0032 §2.
    #[allow(unsafe_code)]
    let rv = unsafe { &*(ptr as *const RuntimeValue) };
    f(Some(rv))
}

// ── Production shims (ADR-0032 §1 hybrid ABI + §4 option-2) ─────────
//
// Design: each shim MARSHALS its ABI arguments into `RuntimeValue`s,
// delegates the SEMANTICS to `triet_ir::dispatch_builtin` (the exact
// code the VM runs — zero VM↔JIT divergence by construction), then
// marshals the result back. All are `extern "C"` (never unwind out);
// on a `VmError` the result-marshaling tail records it via
// `record_shim_failure` + returns a sentinel for the per-call probe.

// ── Argument marshaling (ABI → RuntimeValue) ─────────────────────────

/// Borrow a composite-ptr arg into an owned `RuntimeValue` clone.
/// Null ptr (`T?` null arm) → `RuntimeValue::Null`.
fn arg_composite(ptr: i64) -> RuntimeValue {
    with_rv(ptr, |rv| rv.cloned().unwrap_or(RuntimeValue::Null))
}

/// Wrap a primitive `i64` (Integer ABI slot) into a `RuntimeValue`.
fn arg_integer(v: i64) -> RuntimeValue {
    RuntimeValue::Integer(Integer::new(v).unwrap_or_default())
}

/// Wrap a primitive `i8` (Trilean ABI slot, `{-1,0,+1}` per ADR-0010).
const fn arg_trilean(v: i8) -> RuntimeValue {
    let t = match v {
        1 => Trilean::True,
        -1 => Trilean::False,
        _ => Trilean::Unknown,
    };
    RuntimeValue::Trilean(t)
}

// ── Result marshaling (RuntimeValue → ABI) ───────────────────────────

/// Marshal a dispatch result into a composite-ptr (`Ptr`) ABI return:
/// box the value, or on error record + return null sentinel.
fn finish_ptr(result: Result<RuntimeValue, VmError>) -> i64 {
    match result {
        Ok(rv) => box_rv(rv),
        Err(e) => {
            record_shim_failure(e);
            0
        }
    }
}

/// Marshal into an `i64` (Integer) ABI return. A non-Integer success
/// (shouldn't happen for Integer-returning builtins) records a fault.
fn finish_i64(result: Result<RuntimeValue, VmError>) -> i64 {
    match result {
        Ok(RuntimeValue::Integer(i)) => i.to_i64(),
        Ok(other) => {
            record_shim_failure(VmError::JitShimFault {
                reason: format!("expected Integer return, got {:?}", other.type_tag()),
                function: current_func_name(),
            });
            0
        }
        Err(e) => {
            record_shim_failure(e);
            0
        }
    }
}

/// Marshal into an `i8` (Trilean / Unit) ABI return. Trilean uses the
/// `{-1,0,+1}` encoding (ADR-0010); Unit → `0`.
fn finish_i8(result: Result<RuntimeValue, VmError>) -> i8 {
    match result {
        Ok(RuntimeValue::Trilean(t)) => match t {
            Trilean::True => 1,
            Trilean::Unknown => 0,
            Trilean::False => -1,
        },
        Ok(RuntimeValue::Unit) => 0,
        Ok(other) => {
            record_shim_failure(VmError::JitShimFault {
                reason: format!("expected Trilean/Unit return, got {:?}", other.type_tag()),
                function: current_func_name(),
            });
            0
        }
        Err(e) => {
            record_shim_failure(e);
            0
        }
    }
}

/// Dispatch a builtin with the current function name for attribution.
fn dispatch(name: BuiltinName, args: &[RuntimeValue]) -> Result<RuntimeValue, VmError> {
    dispatch_builtin(name, args, &current_func_name())
}

// ── Aggregate-opcode shims (ADR-0034 §1/§2) ─────────────────────────
//
// The JIT's boxed codegen mode lowers struct IR opcodes to these shims,
// which delegate to the `triet_ir::exec_*` helpers — the SAME logic the
// VM instruction loop runs (no VM↔JIT divergence by construction).
// Composite args/returns use the ADR-0032 §1/§2 boxed-ptr ABI
// (`arg_composite` borrows + clones; `box_rv`/`finish_ptr` box out).
// `builtin: None` in the registry (called by codegen directly, like
// `__triet_drop_arc`, not via a `CallBuiltin`).

/// `struct_new(fields_ptr, len) -> Struct` — the variadic array-ptr+len
/// ABI (ADR-0034 §2). `fields_ptr` addresses `len` consecutive `i64`
/// field-value pointers the JIT spilled into a caller-owned stack slot.
extern "C" fn __triet_struct_new(fields_ptr: i64, len: i64) -> i64 {
    let n = usize::try_from(len).unwrap_or(0);
    let mut fields = Vec::with_capacity(n);
    if n > 0 {
        // SAFETY: per ADR-0034 §2, JIT codegen spills exactly `len`
        // consecutive `i64` field-value pointers into a caller-owned
        // stack slot at `fields_ptr` that lives for this call. Each slot
        // is an `Rc::into_raw` `RuntimeValue` pointer (or 0 = null). We
        // read exactly `n` of them and borrow (not consume) each via
        // `arg_composite` — refcounts unchanged. `n > 0` guards against a
        // dangling/zero base for empty structs.
        #[allow(unsafe_code)]
        let slots: &[i64] = unsafe { std::slice::from_raw_parts(fields_ptr as *const i64, n) };
        for &p in slots {
            fields.push(arg_composite(p));
        }
    }
    box_rv(exec_struct_new(fields))
}

/// `field_get(obj, idx) -> field` — read field `idx` of a boxed struct.
extern "C" fn __triet_field_get(obj_ptr: i64, idx: i64) -> i64 {
    let obj = arg_composite(obj_ptr);
    let field_idx = u32::try_from(idx).unwrap_or(0);
    finish_ptr(exec_field_get(&obj, field_idx, &current_func_name()))
}

/// `field_set(obj, idx, val) -> Struct` — functional field update.
extern "C" fn __triet_field_set(obj_ptr: i64, idx: i64, val_ptr: i64) -> i64 {
    let obj = arg_composite(obj_ptr);
    let val = arg_composite(val_ptr);
    let field_idx = u32::try_from(idx).unwrap_or(0);
    finish_ptr(exec_field_set(&obj, field_idx, val, &current_func_name()))
}

/// `binop(op, a, b) -> result` — the boxed-mode binary scalar op
/// (ADR-0034 §1, agg.1c). `op` is a [`JitBinOp`] discriminant; `a`/`b`
/// are boxed operands. Delegates to `exec_jit_binop` (the VM's own
/// arithmetic/comparison/logic) → identical results, boxed back out.
extern "C" fn __triet_binop(op: i8, a_ptr: i64, b_ptr: i64) -> i64 {
    let Some(op) = u8::try_from(op).ok().and_then(JitBinOp::from_u8) else {
        record_shim_failure(VmError::JitShimFault {
            reason: format!("invalid JitBinOp discriminant {op}"),
            function: current_func_name(),
        });
        return 0;
    };
    let a = arg_composite(a_ptr);
    let b = arg_composite(b_ptr);
    finish_ptr(exec_jit_binop(op, &a, &b, &current_func_name()))
}

/// `neg(v) -> -v` — the boxed-mode unary negation (ADR-0034 §1).
extern "C" fn __triet_neg(v_ptr: i64) -> i64 {
    let v = arg_composite(v_ptr);
    finish_ptr(exec_jit_neg(&v, &current_func_name()))
}

/// `box_const(kind, payload) -> boxed` — materialize a primitive
/// constant in boxed mode (ADR-0034 §1, agg.1c). `kind` is a
/// [`JitConstKind`] discriminant; `payload` carries the scalar value
/// (i8/i16/i64-range; ignored for Unit/Null). Delegates to
/// `exec_box_const` (the VM's `Constant`→`RuntimeValue` shape).
extern "C" fn __triet_box_const(kind: i8, payload: i64) -> i64 {
    let Some(kind) = u8::try_from(kind).ok().and_then(JitConstKind::from_u8) else {
        record_shim_failure(VmError::JitShimFault {
            reason: format!("invalid JitConstKind discriminant {kind}"),
            function: current_func_name(),
        });
        return 0;
    };
    box_rv(exec_box_const(kind, payload))
}

/// `unbox_scalar(ptr, kind) -> i64` — the inverse of `__triet_box_const`
/// for cross-mode call marshaling (ADR-0034 agg.cross-call).
///
/// Reads the boxed `RuntimeValue` at `ptr` and returns its scalar value
/// as raw `i64` bits (the codegen narrows to i8/i16 for the callee's slot
/// type). `kind` is the expected [`JitConstKind`]. A type mismatch records
/// a `JitShimFault` (the per-call sentinel surfaces it) — by construction
/// the callee's param type guarantees the match, so a mismatch is a bug.
extern "C" fn __triet_unbox_scalar(ptr: i64, kind: i8) -> i64 {
    let Some(kind) = u8::try_from(kind).ok().and_then(JitConstKind::from_u8) else {
        record_shim_failure(VmError::JitShimFault {
            reason: format!("invalid JitConstKind discriminant {kind}"),
            function: current_func_name(),
        });
        return 0;
    };
    let rv = arg_composite(ptr);
    match (kind, &rv) {
        (JitConstKind::Integer, RuntimeValue::Integer(i)) => i.to_i64(),
        // Trilean uses the {-1,0,+1} encoding; delegate to `as_trilean`
        // (via `exec_trilean_tag`) so Trit/Integer carriers also work.
        (JitConstKind::Trilean, _) => i64::from(exec_trilean_tag(&rv)),
        (JitConstKind::Trit, RuntimeValue::Trit(t)) => i64::from(t.to_i8()),
        (JitConstKind::Tryte, RuntimeValue::Tryte(t)) => t.to_i64(),
        _ => {
            record_shim_failure(VmError::JitShimFault {
                reason: format!("unbox_scalar: expected {kind:?}, got {:?}", rv.type_tag()),
                function: current_func_name(),
            });
            0
        }
    }
}

/// `trilean_tag(cond) -> i8` — read a boxed branch condition's three-way
/// tag (`{-1,0,+1}` = `False/Unknown/True`) for boxed-mode `BrIf` /
/// `BrTrilean` (ADR-0034 agg.1c-iv). Delegates to `exec_trilean_tag`
/// (the VM's `as_trilean`) so JIT branch dispatch matches the VM. Total:
/// null / non-Trilean values map through `as_trilean` — never faults, so
/// no sentinel probe is needed (a branch is a terminator).
extern "C" fn __triet_trilean_tag(cond_ptr: i64) -> i8 {
    let cond = arg_composite(cond_ptr);
    exec_trilean_tag(&cond)
}

// ── Enum-opcode shims (ADR-0034 agg.2a) ─────────────────────────────

/// `enum_new(variant, has_payload, payload) -> Enum` — construct an enum
/// variant. `has_payload == 0` → unit variant (`payload` ignored, may be
/// a 0 sentinel); otherwise `payload` is the boxed payload value. The
/// presence flag is needed because a genuine payload could itself be a
/// boxed `Null` (a non-zero ptr), so ptr==0 alone can't mean "no
/// payload". Delegates to `exec_enum_new`.
extern "C" fn __triet_enum_new(variant: i64, has_payload: i8, payload_ptr: i64) -> i64 {
    let variant = u32::try_from(variant).unwrap_or(0);
    let payload = if has_payload != 0 {
        Some(arg_composite(payload_ptr))
    } else {
        None
    };
    box_rv(exec_enum_new(variant, payload))
}

/// `enum_tag(scrutinee) -> Integer` — the variant index as a boxed
/// Integer (`Null` → -1, bare value → 0). Delegates to `exec_enum_tag`;
/// total, never faults.
extern "C" fn __triet_enum_tag(scrutinee_ptr: i64) -> i64 {
    let scr = arg_composite(scrutinee_ptr);
    box_rv(exec_enum_tag(&scr))
}

/// `enum_payload(scrutinee) -> payload` — unpack a variant's payload.
/// A payload-less / non-enum scrutinee records an `InvalidVariant`
/// failure + returns the null sentinel (per-call probe). Delegates to
/// `exec_enum_payload`.
extern "C" fn __triet_enum_payload(scrutinee_ptr: i64) -> i64 {
    let scr = arg_composite(scrutinee_ptr);
    finish_ptr(exec_enum_payload(&scr, &current_func_name()))
}

// ── Outcome-opcode shims (ADR-0034 agg.2b) ──────────────────────────

/// `outcome_new_positive(payload) -> Outcome` — wrap in the success arm.
extern "C" fn __triet_outcome_new_positive(payload_ptr: i64) -> i64 {
    box_rv(exec_outcome_new_positive(arg_composite(payload_ptr)))
}

/// `outcome_new_negative(payload) -> Outcome` — wrap in the failure arm.
extern "C" fn __triet_outcome_new_negative(payload_ptr: i64) -> i64 {
    box_rv(exec_outcome_new_negative(arg_composite(payload_ptr)))
}

/// `outcome_new_null() -> Outcome` — the Zero/null arm (no payload).
extern "C" fn __triet_outcome_new_null() -> i64 {
    box_rv(exec_outcome_new_null())
}

/// `outcome_discriminant(source) -> Trit` — the arm trit, boxed. Total
/// (cross-tolerance: Null→Zero, bare value→Positive); never faults.
extern "C" fn __triet_outcome_discriminant(source_ptr: i64) -> i64 {
    let src = arg_composite(source_ptr);
    box_rv(exec_outcome_discriminant(&src))
}

/// `outcome_unwrap_value(source) -> payload` — extract the success
/// payload (bare value passes through); a null / non-success arm records
/// an `InvalidOutcomeState` + returns the failure sentinel.
extern "C" fn __triet_outcome_unwrap_value(source_ptr: i64) -> i64 {
    let src = arg_composite(source_ptr);
    finish_ptr(exec_outcome_unwrap_value(src, &current_func_name()))
}

/// `outcome_unwrap_error(source) -> payload` — extract the failure
/// payload; a null / non-failure arm or non-Outcome value records a
/// failure + returns the sentinel.
extern "C" fn __triet_outcome_unwrap_error(source_ptr: i64) -> i64 {
    let src = arg_composite(source_ptr);
    finish_ptr(exec_outcome_unwrap_error(src, &current_func_name()))
}

// ── Nullable-opcode shims (ADR-0034 agg.3a) ─────────────────────────

/// `null_wrap(value) -> Some-carrier` — wrap as the non-null carrier.
extern "C" fn __triet_null_wrap(value_ptr: i64) -> i64 {
    box_rv(exec_null_wrap(arg_composite(value_ptr)))
}

/// `null_unwrap(nullable) -> value` — force-unwrap; `Null` records a
/// `NullUnwrap` failure + returns the sentinel (per-call probe).
extern "C" fn __triet_null_unwrap(nullable_ptr: i64) -> i64 {
    let v = arg_composite(nullable_ptr);
    finish_ptr(exec_null_unwrap(v, &current_func_name()))
}

/// `null_check(nullable) -> Trit` — the discriminator trit, boxed. Total
/// (Null / `Outcome{Zero,None}` → Zero, else Positive); never faults.
extern "C" fn __triet_null_check(nullable_ptr: i64) -> i64 {
    let v = arg_composite(nullable_ptr);
    box_rv(exec_null_check(&v))
}

// ── jit.2a shims (5) — now delegating ───────────────────────────────

/// `assert(cond: Trilean, msg: String?)` → Unit.
extern "C" fn __triet_assert(cond: i8, msg_ptr: i64) -> i8 {
    finish_i8(dispatch(
        BuiltinName::Assert,
        &[arg_trilean(cond), arg_composite(msg_ptr)],
    ))
}

/// `println(value)` → Unit.
extern "C" fn __triet_println(val_ptr: i64) -> i8 {
    finish_i8(dispatch(BuiltinName::Println, &[arg_composite(val_ptr)]))
}

/// `text.len(s: String) -> Integer`.
extern "C" fn __triet_text_len(s_ptr: i64) -> i64 {
    finish_i64(dispatch(BuiltinName::TextLen, &[arg_composite(s_ptr)]))
}

/// `vector.new() -> Vector`.
extern "C" fn __triet_vector_new() -> i64 {
    finish_ptr(dispatch(BuiltinName::VectorNew, &[]))
}

/// `vector.push(v: Vector, x) -> Vector` (functional return-new).
extern "C" fn __triet_vector_push(vec_ptr: i64, val_ptr: i64) -> i64 {
    finish_ptr(dispatch(
        BuiltinName::VectorPush,
        &[arg_composite(vec_ptr), arg_composite(val_ptr)],
    ))
}

// ── jit.2b-i clean shims (16) ────────────────────────────────────────

/// `print(value)` → Unit (no newline).
extern "C" fn __triet_print(val_ptr: i64) -> i8 {
    finish_i8(dispatch(BuiltinName::Print, &[arg_composite(val_ptr)]))
}

/// `assert_eq(a, b)` → Unit (structural equality; fail records error).
extern "C" fn __triet_assert_eq(a_ptr: i64, b_ptr: i64) -> i8 {
    finish_i8(dispatch(
        BuiltinName::AssertEq,
        &[arg_composite(a_ptr), arg_composite(b_ptr)],
    ))
}

/// `text.from_integer(n: Integer) -> String`.
extern "C" fn __triet_text_from_integer(n: i64) -> i64 {
    finish_ptr(dispatch(BuiltinName::TextFromInteger, &[arg_integer(n)]))
}

/// `vector.get(v: Vector, i: Integer) -> T?` (boxed Null-or-element).
extern "C" fn __triet_vector_get(vec_ptr: i64, idx: i64) -> i64 {
    finish_ptr(dispatch(
        BuiltinName::VectorGet,
        &[arg_composite(vec_ptr), arg_integer(idx)],
    ))
}

/// `vector.length(v: Vector) -> Integer`.
extern "C" fn __triet_vector_length(vec_ptr: i64) -> i64 {
    finish_i64(dispatch(
        BuiltinName::VectorLength,
        &[arg_composite(vec_ptr)],
    ))
}

/// `hashmap.new() -> HashMap`.
extern "C" fn __triet_hashmap_new() -> i64 {
    finish_ptr(dispatch(BuiltinName::HashMapNew, &[]))
}

/// `hashmap.insert(m, k, v) -> HashMap` (functional return-new).
extern "C" fn __triet_hashmap_insert(map_ptr: i64, key_ptr: i64, val_ptr: i64) -> i64 {
    finish_ptr(dispatch(
        BuiltinName::HashMapInsert,
        &[
            arg_composite(map_ptr),
            arg_composite(key_ptr),
            arg_composite(val_ptr),
        ],
    ))
}

/// `hashmap.get(m, k) -> V?` (boxed Null-or-value).
extern "C" fn __triet_hashmap_get(map_ptr: i64, key_ptr: i64) -> i64 {
    finish_ptr(dispatch(
        BuiltinName::HashMapGet,
        &[arg_composite(map_ptr), arg_composite(key_ptr)],
    ))
}

/// `hashmap.keys(m) -> Vector<K>`.
extern "C" fn __triet_hashmap_keys(map_ptr: i64) -> i64 {
    finish_ptr(dispatch(
        BuiltinName::HashMapKeys,
        &[arg_composite(map_ptr)],
    ))
}

/// `hashmap.contains(m, k) -> Trilean`.
extern "C" fn __triet_hashmap_contains(map_ptr: i64, key_ptr: i64) -> i8 {
    finish_i8(dispatch(
        BuiltinName::HashMapContains,
        &[arg_composite(map_ptr), arg_composite(key_ptr)],
    ))
}

/// `path.join(base, segment) -> String`.
extern "C" fn __triet_path_join(base_ptr: i64, seg_ptr: i64) -> i64 {
    finish_ptr(dispatch(
        BuiltinName::PathJoin,
        &[arg_composite(base_ptr), arg_composite(seg_ptr)],
    ))
}

/// `path.parent(path) -> String?` (boxed Null-or-String).
extern "C" fn __triet_path_parent(path_ptr: i64) -> i64 {
    finish_ptr(dispatch(
        BuiltinName::PathParent,
        &[arg_composite(path_ptr)],
    ))
}

/// `path.basename(path) -> String`.
extern "C" fn __triet_path_basename(path_ptr: i64) -> i64 {
    finish_ptr(dispatch(
        BuiltinName::PathBasename,
        &[arg_composite(path_ptr)],
    ))
}

/// `string.substring(s, start: Integer, end: Integer) -> String`.
extern "C" fn __triet_string_substring(s_ptr: i64, start: i64, end: i64) -> i64 {
    finish_ptr(dispatch(
        BuiltinName::StringSubstring,
        &[arg_composite(s_ptr), arg_integer(start), arg_integer(end)],
    ))
}

/// `string.split(s, sep) -> Vector<String>`.
extern "C" fn __triet_string_split(s_ptr: i64, sep_ptr: i64) -> i64 {
    finish_ptr(dispatch(
        BuiltinName::StringSplit,
        &[arg_composite(s_ptr), arg_composite(sep_ptr)],
    ))
}

/// `string.index_of(s, needle) -> Integer?` (boxed Null-or-Integer).
extern "C" fn __triet_string_index_of(s_ptr: i64, needle_ptr: i64) -> i64 {
    finish_ptr(dispatch(
        BuiltinName::StringIndexOf,
        &[arg_composite(s_ptr), arg_composite(needle_ptr)],
    ))
}

/// `text.parse_integer(s) -> Integer?` (boxed Null-or-Integer).
extern "C" fn __triet_parse_integer(s_ptr: i64) -> i64 {
    finish_ptr(dispatch(BuiltinName::ParseInteger, &[arg_composite(s_ptr)]))
}

/// `text.into_bytes(s) -> Vector<Integer>`.
extern "C" fn __triet_text_into_bytes(s_ptr: i64) -> i64 {
    finish_ptr(dispatch(
        BuiltinName::TextIntoBytes,
        &[arg_composite(s_ptr)],
    ))
}

/// `text.from_bytes(bytes: Vector<Integer>) -> String?`.
extern "C" fn __triet_text_from_bytes(bytes_ptr: i64) -> i64 {
    finish_ptr(dispatch(
        BuiltinName::TextFromBytes,
        &[arg_composite(bytes_ptr)],
    ))
}

/// `crypto.blake3_hash(s) -> Vector<Integer>`.
extern "C" fn __triet_blake3_hash(s_ptr: i64) -> i64 {
    finish_ptr(dispatch(BuiltinName::Blake3Hash, &[arg_composite(s_ptr)]))
}

/// `env.get(name) -> String?` (boxed Null-or-String).
extern "C" fn __triet_get_env(name_ptr: i64) -> i64 {
    finish_ptr(dispatch(BuiltinName::GetEnv, &[arg_composite(name_ptr)]))
}

// ── jit.2b-ii Atomic shims (10) ──────────────────────────────────────
//
// Atomic delegation works because `RuntimeValue::Atomic(Arc<Mutex<…>>)`
// clones SHARE the underlying cell (Arc::clone) — `arg_composite` clones
// the borrowed Atomic, `dispatch_builtin` mutates the shared Mutex, and
// the caller's boxed Atomic observes the change. The `ordering` arg is a
// composite (Ordering enum) ptr; the VM ignores it on the dev tier
// (ADR-0028 §9), so it's marshaled but inert.
//
// **Scope: Atomic<Integer>** (value/delta/mask slots = `i64`). The
// fetch_*/bitwise ops are Integer-only per ADR-0028 §4.2/§4.3.
// load/store/swap/compare_exchange are polymorphic over AtomicValue but
// jit.2b-ii wires the Integer instantiation (the counter case); non-
// Integer atomic value ops tier-down. **End-to-end JIT of a full
// atomic-using function is additionally gated on Ordering-enum codegen
// (`EnumNew` is not JIT-supported), so these shims are validated by
// direct unit tests; a real `fetch_add(c, 1, Synchronized)` function
// tier-downs at the enum construction until enum codegen lands.**

/// `atomic.new(initial: Integer) -> Atomic<Integer>` (boxed Atomic).
extern "C" fn __triet_atomic_new(initial: i64) -> i64 {
    finish_ptr(dispatch(BuiltinName::AtomicNew, &[arg_integer(initial)]))
}

/// `atomic.load(self, ordering) -> Integer`.
extern "C" fn __triet_atomic_load(self_ptr: i64, ord_ptr: i64) -> i64 {
    finish_i64(dispatch(
        BuiltinName::AtomicLoad,
        &[arg_composite(self_ptr), arg_composite(ord_ptr)],
    ))
}

/// `atomic.store(self, value: Integer, ordering) -> Unit`.
extern "C" fn __triet_atomic_store(self_ptr: i64, value: i64, ord_ptr: i64) -> i8 {
    finish_i8(dispatch(
        BuiltinName::AtomicStore,
        &[
            arg_composite(self_ptr),
            arg_integer(value),
            arg_composite(ord_ptr),
        ],
    ))
}

/// `atomic.swap(self, value: Integer, ordering) -> Integer` (prev).
extern "C" fn __triet_atomic_swap(self_ptr: i64, value: i64, ord_ptr: i64) -> i64 {
    finish_i64(dispatch(
        BuiltinName::AtomicSwap,
        &[
            arg_composite(self_ptr),
            arg_integer(value),
            arg_composite(ord_ptr),
        ],
    ))
}

/// `atomic.compare_exchange(self, expected, new, succ_ord, fail_ord)
/// -> Integer~CompareExchangeFailed` (boxed Outcome).
extern "C" fn __triet_atomic_compare_exchange(
    self_ptr: i64,
    expected: i64,
    new_value: i64,
    succ_ord_ptr: i64,
    fail_ord_ptr: i64,
) -> i64 {
    finish_ptr(dispatch(
        BuiltinName::AtomicCompareExchange,
        &[
            arg_composite(self_ptr),
            arg_integer(expected),
            arg_integer(new_value),
            arg_composite(succ_ord_ptr),
            arg_composite(fail_ord_ptr),
        ],
    ))
}

/// `atomic.fetch_add(self, delta: Integer, ordering) -> Integer` (prev).
extern "C" fn __triet_atomic_fetch_add(self_ptr: i64, delta: i64, ord_ptr: i64) -> i64 {
    finish_i64(dispatch(
        BuiltinName::AtomicFetchAdd,
        &[
            arg_composite(self_ptr),
            arg_integer(delta),
            arg_composite(ord_ptr),
        ],
    ))
}

/// `atomic.fetch_sub(self, delta: Integer, ordering) -> Integer` (prev).
extern "C" fn __triet_atomic_fetch_sub(self_ptr: i64, delta: i64, ord_ptr: i64) -> i64 {
    finish_i64(dispatch(
        BuiltinName::AtomicFetchSub,
        &[
            arg_composite(self_ptr),
            arg_integer(delta),
            arg_composite(ord_ptr),
        ],
    ))
}

/// `atomic.fetch_bitwise_and(self, mask: Integer, ordering) -> Integer`.
extern "C" fn __triet_atomic_fetch_bitwise_and(self_ptr: i64, mask: i64, ord_ptr: i64) -> i64 {
    finish_i64(dispatch(
        BuiltinName::AtomicFetchBitwiseAnd,
        &[
            arg_composite(self_ptr),
            arg_integer(mask),
            arg_composite(ord_ptr),
        ],
    ))
}

/// `atomic.fetch_bitwise_or(self, mask: Integer, ordering) -> Integer`.
extern "C" fn __triet_atomic_fetch_bitwise_or(self_ptr: i64, mask: i64, ord_ptr: i64) -> i64 {
    finish_i64(dispatch(
        BuiltinName::AtomicFetchBitwiseOr,
        &[
            arg_composite(self_ptr),
            arg_integer(mask),
            arg_composite(ord_ptr),
        ],
    ))
}

/// `atomic.fetch_bitwise_xor(self, mask: Integer, ordering) -> Integer`.
extern "C" fn __triet_atomic_fetch_bitwise_xor(self_ptr: i64, mask: i64, ord_ptr: i64) -> i64 {
    finish_i64(dispatch(
        BuiltinName::AtomicFetchBitwiseXor,
        &[
            arg_composite(self_ptr),
            arg_integer(mask),
            arg_composite(ord_ptr),
        ],
    ))
}

// ── jit.2b-iii file I/O shims (5) ────────────────────────────────────
//
// Fixed-arity, delegate to `dispatch_builtin` (which performs the real
// filesystem I/O) — same clean pattern as the jit.2b-i shims, NOT an
// ABI cliff. Side-effecting, so parity tests use tempdir fixtures.
// Capability (`std.io.fs`) is enforced at program-load time (ADR-0016
// §5), not here. (The varargs builtins FStringConcat/TextConcat ARE a
// genuine ABI cliff — they need an array-ptr+len ABI — and defer to
// v0.11 per the jit.2b-iii decision; they tier-down to VM until then.)

/// `fs.read(path: String) -> String?` (boxed Null-on-error / contents).
extern "C" fn __triet_read_file(path_ptr: i64) -> i64 {
    finish_ptr(dispatch(BuiltinName::ReadFile, &[arg_composite(path_ptr)]))
}

/// `fs.write(path: String, contents: String) -> Trilean` (strict 2-state).
extern "C" fn __triet_write_file(path_ptr: i64, contents_ptr: i64) -> i8 {
    finish_i8(dispatch(
        BuiltinName::WriteFile,
        &[arg_composite(path_ptr), arg_composite(contents_ptr)],
    ))
}

/// `fs.write_bytes(path: String, bytes: Vector<Integer>) -> Trilean`.
extern "C" fn __triet_write_file_bytes(path_ptr: i64, bytes_ptr: i64) -> i8 {
    finish_i8(dispatch(
        BuiltinName::WriteFileBytes,
        &[arg_composite(path_ptr), arg_composite(bytes_ptr)],
    ))
}

/// `fs.exists(path: String) -> Trilean` (strict 2-state).
extern "C" fn __triet_file_exists(path_ptr: i64) -> i8 {
    finish_i8(dispatch(
        BuiltinName::FileExists,
        &[arg_composite(path_ptr)],
    ))
}

/// `fs.read_dir_recursive(dir: String) -> Vector<Tuple>` (composite-in-
/// composite — still a single boxed `RuntimeValue`).
extern "C" fn __triet_read_dir_recursive(dir_ptr: i64) -> i64 {
    finish_ptr(dispatch(
        BuiltinName::ReadDirRecursive,
        &[arg_composite(dir_ptr)],
    ))
}

// ── Error propagation (§4 option-2 resolution, v0.10.x.jit.2a) ──────
//
// ADR-0032 §4's original `extern "C-unwind"` + dispatcher
// `catch_unwind` mechanism is BLOCKED on `cranelift-jit 0.132` (no
// system DWARF unwind-table registration for JIT'd frames → abort).
// The Addendum-locked replacement (option 2, per-call sentinel):
//
// - Shims are plain `extern "C"` (never unwind out). On a `VmError`-
//   class failure a shim calls [`record_shim_failure`] (records the
//   structured error + sets the `SHIM_FAILED` flag) and returns a
//   sentinel (`0` / null).
// - The JIT emits a [`__triet_shim_failed`] probe after each shim call
//   and branches to the function's `error_exit` block when the flag
//   is set — so the JIT frame returns NORMALLY (no unwinding).
// - The dispatcher reads the slot via [`take_shim_failure`] after the
//   native return; `Some` → `Err(VmError)`, `None` → `Ok(value)`.
//
// Single-thread VM dev tier (ADR-0028 §9) makes the per-thread slot
// trivially correct; multi-thread inherits per-thread semantics.

thread_local! {
    /// Structured error a failing shim records via
    /// [`record_shim_failure`]; the dispatcher consumes it via
    /// [`take_shim_failure`] after the native call returns.
    static CURRENT_VM_ERROR: RefCell<Option<VmError>> = const { RefCell::new(None) };

    /// Failure flag set alongside [`CURRENT_VM_ERROR`]. Read by the
    /// JIT-emitted per-call probe [`__triet_shim_failed`] so primitive-
    /// returning shims (where `0` is a valid value) can still signal
    /// failure without poisoning their value space.
    static SHIM_FAILED: Cell<bool> = const { Cell::new(false) };

    /// Name of the Triết function whose JIT'd body is executing — set
    /// by the dispatcher before the native call so a shim can attribute
    /// `VmError { function, .. }`.
    static CURRENT_FUNC_NAME: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Record a structured `VmError` + raise the `SHIM_FAILED` flag. A
/// shim calls this immediately before returning its sentinel.
pub(crate) fn record_shim_failure(error: VmError) {
    CURRENT_VM_ERROR.with(|slot| *slot.borrow_mut() = Some(error));
    SHIM_FAILED.with(|flag| flag.set(true));
}

/// Clear the failure state. Called by the dispatcher before a native
/// call so a stale flag from a prior call can't leak in.
pub(crate) fn clear_shim_state() {
    CURRENT_VM_ERROR.with(|slot| *slot.borrow_mut() = None);
    SHIM_FAILED.with(|flag| flag.set(false));
}

/// Consume the recorded failure, if any. Called by the dispatcher
/// after the native return: `Some` → the JIT'd function hit a shim
/// failure (returned a sentinel); `None` → clean run.
pub(crate) fn take_shim_failure() -> Option<VmError> {
    if SHIM_FAILED.with(Cell::get) {
        SHIM_FAILED.with(|flag| flag.set(false));
        Some(
            CURRENT_VM_ERROR
                .with(|slot| slot.borrow_mut().take())
                .unwrap_or_else(|| VmError::JitShimFault {
                    reason: "shim set SHIM_FAILED without recording a structured VmError"
                        .to_owned(),
                    function: current_func_name(),
                }),
        )
    } else {
        None
    }
}

/// Set the executing function's name (dispatcher → shim attribution).
pub(crate) fn set_func_name(name: &str) {
    CURRENT_FUNC_NAME.with(|slot| name.clone_into(&mut slot.borrow_mut()));
}

/// Read the executing function's name (shim error construction).
pub(crate) fn current_func_name() -> String {
    CURRENT_FUNC_NAME.with(|slot| slot.borrow().clone())
}

/// JIT-emitted per-call probe (ADR-0032 §4 option-2). Returns `1` if
/// the most recent shim set `SHIM_FAILED`, else `0`. The JIT branches
/// to `error_exit` on `1`. Does NOT reset the flag — the dispatcher's
/// [`take_shim_failure`] owns the reset so the error survives until
/// consumed.
pub(crate) extern "C" fn __triet_shim_failed() -> i8 {
    i8::from(SHIM_FAILED.with(Cell::get))
}

#[cfg(test)]
mod tests {
    use super::*;
    use triet_core::Integer;

    /// Box a value the way JIT codegen would, returning the i64 ptr +
    /// keeping it valid until the caller drops it via `__triet_drop_arc`.
    fn box_for_test(value: RuntimeValue) -> i64 {
        box_rv(value)
    }

    fn integer(n: i64) -> RuntimeValue {
        RuntimeValue::Integer(Integer::new(n).unwrap())
    }

    /// `RuntimeValue` does not implement `PartialEq`; assert the
    /// `Integer` arm's value directly.
    fn expect_integer(rv: &RuntimeValue, want: i64) {
        match rv {
            RuntimeValue::Integer(i) => assert_eq!(i.to_i64(), want),
            other => panic!("expected Integer({want}), got {other:?}"),
        }
    }

    #[test]
    fn struct_shims_round_trip_and_delegate() {
        // v0.11.x.jit.4.agg.1 — exercise the struct shims directly (the
        // boxed codegen that calls them lands in agg.1b). Build {7, 9} via
        // the array-ptr+len ABI, read fields, functional-update one.
        let _ = take_shim_failure(); // clear any prior thread-local state

        let f0 = box_for_test(integer(7));
        let f1 = box_for_test(integer(9));
        let slots: [i64; 2] = [f0, f1];
        let s = __triet_struct_new(
            i64::try_from(slots.as_ptr() as usize).expect("addr fits i64"),
            2,
        );

        let g0 = __triet_field_get(s, 0);
        with_rv(g0, |rv| expect_integer(rv.expect("field 0"), 7));
        let g1 = __triet_field_get(s, 1);
        with_rv(g1, |rv| expect_integer(rv.expect("field 1"), 9));

        // Functional update: field_set returns a NEW struct (s unchanged).
        let v = box_for_test(integer(42));
        let s2 = __triet_field_set(s, 0, v);
        let h0 = __triet_field_get(s2, 0);
        with_rv(h0, |rv| expect_integer(rv.expect("updated field 0"), 42));
        let h1 = __triet_field_get(s2, 1);
        with_rv(h1, |rv| expect_integer(rv.expect("field 1 unchanged"), 9));
        // Original struct's field 0 is still 7 (functional, not mutated).
        let orig0 = __triet_field_get(s, 0);
        with_rv(orig0, |rv| expect_integer(rv.expect("orig field 0"), 7));

        assert!(
            take_shim_failure().is_none(),
            "valid struct ops must not record a shim failure"
        );

        // Each box is independent (shims clone) → drop each exactly once.
        for p in [f0, f1, s, g0, g1, v, s2, h0, h1, orig0] {
            __triet_drop_arc(p);
        }
    }

    #[test]
    fn field_get_on_non_struct_records_failure() {
        // A non-struct receiver is a TypeMismatch the shim surfaces via
        // the SHIM_FAILED flag (per ADR-0032 §4), returning the 0 sentinel.
        let _ = take_shim_failure();
        let not_a_struct = box_for_test(integer(5));
        let r = __triet_field_get(not_a_struct, 0);
        assert_eq!(r, 0, "failure returns the null sentinel");
        assert!(
            take_shim_failure().is_some(),
            "field_get on a non-struct must record a shim failure"
        );
        __triet_drop_arc(not_a_struct);
    }

    #[test]
    fn drop_arc_balances_into_raw() {
        let ptr = box_for_test(RuntimeValue::String("x".to_owned()));
        // Reconstitute to check strong_count == 1, then re-leak so the
        // drop_arc below is the sole consumer (no double-free).
        // SAFETY: ptr just came from box_rv (Rc::into_raw of refcount-1).
        #[allow(unsafe_code)]
        let rc = unsafe { Rc::from_raw(ptr as *const RuntimeValue) };
        assert_eq!(Rc::strong_count(&rc), 1);
        let reptr = Rc::into_raw(rc) as i64;
        __triet_drop_arc(reptr); // consumes the box, frees it
    }

    #[test]
    fn drop_arc_null_is_noop() {
        __triet_drop_arc(0); // must not panic / segfault
    }

    #[test]
    fn text_len_counts_utf8_chars() {
        let ptr = box_for_test(RuntimeValue::String("Triết".to_owned()));
        // 5 chars: T r i ế t (ế is one Unicode scalar).
        assert_eq!(__triet_text_len(ptr), 5);
        __triet_drop_arc(ptr);
    }

    #[test]
    fn text_len_empty_string() {
        let ptr = box_for_test(RuntimeValue::String(String::new()));
        assert_eq!(__triet_text_len(ptr), 0);
        __triet_drop_arc(ptr);
    }

    #[test]
    fn vector_new_returns_empty_vector() {
        let ptr = __triet_vector_new();
        with_rv(ptr, |rv| match rv {
            Some(RuntimeValue::Vector(v)) => assert!(v.is_empty()),
            other => panic!("expected empty Vector, got {other:?}"),
        });
        __triet_drop_arc(ptr);
    }

    #[test]
    fn vector_push_returns_new_with_appended_element() {
        // Functional: original unchanged, return has +1 element.
        let vec_ptr = box_for_test(RuntimeValue::Vector(vec![integer(1)]));
        let elem_ptr = box_for_test(integer(2));
        let result_ptr = __triet_vector_push(vec_ptr, elem_ptr);
        // Result has [1, 2].
        with_rv(result_ptr, |rv| match rv {
            Some(RuntimeValue::Vector(v)) => {
                assert_eq!(v.len(), 2);
                expect_integer(&v[0], 1);
                expect_integer(&v[1], 2);
            }
            other => panic!("expected Vector[1,2], got {other:?}"),
        });
        // Original vec UNCHANGED (functional semantics).
        with_rv(vec_ptr, |rv| match rv {
            Some(RuntimeValue::Vector(v)) => assert_eq!(v.len(), 1),
            other => panic!("expected original Vector[1], got {other:?}"),
        });
        __triet_drop_arc(vec_ptr);
        __triet_drop_arc(elem_ptr);
        __triet_drop_arc(result_ptr);
    }

    #[test]
    fn assert_true_passes_no_failure() {
        clear_shim_state();
        set_func_name("test_fn");
        let ret = __triet_assert(1, 0); // cond=True, no message
        assert_eq!(ret, 0); // Unit sentinel
        assert!(take_shim_failure().is_none());
    }

    #[test]
    fn assert_false_records_failure_with_message() {
        clear_shim_state();
        set_func_name("test_fn");
        let msg_ptr = box_for_test(RuntimeValue::String("boom".to_owned()));
        let ret = __triet_assert(-1, msg_ptr); // cond=False
        assert_eq!(ret, 0);
        match take_shim_failure() {
            Some(VmError::AssertionFailed { message, function }) => {
                assert_eq!(message.as_deref(), Some("boom"));
                assert_eq!(function, "test_fn");
            }
            other => panic!("expected AssertionFailed, got {other:?}"),
        }
        __triet_drop_arc(msg_ptr);
    }

    #[test]
    fn assert_false_null_msg_matches_vm() {
        // A null msg ptr marshals to `RuntimeValue::Null`; the shim
        // delegates to `dispatch_builtin`, so the message matches what
        // the VM produces for `assert(False, Null)` exactly — namely
        // `Some("null")` (VM formats the Null arg via Display). This is
        // VM↔JIT parity by construction; the jit.2a hand-rolled shim's
        // `None` was a divergence that delegation corrected.
        clear_shim_state();
        set_func_name("f");
        let ret = __triet_assert(-1, 0); // cond=False, null msg ptr
        assert_eq!(ret, 0);
        match take_shim_failure() {
            Some(VmError::AssertionFailed { message, .. }) => {
                assert_eq!(message.as_deref(), Some("null"));
            }
            other => panic!("expected AssertionFailed, got {other:?}"),
        }
    }

    #[test]
    fn shim_failed_probe_reflects_flag() {
        clear_shim_state();
        assert_eq!(__triet_shim_failed(), 0);
        record_shim_failure(VmError::JitShimFault {
            reason: "x".to_owned(),
            function: "f".to_owned(),
        });
        assert_eq!(__triet_shim_failed(), 1);
        // take resets the flag.
        assert!(take_shim_failure().is_some());
        assert_eq!(__triet_shim_failed(), 0);
    }

    // (Full implemented-vs-tier-down coverage is asserted by
    // `shim_coverage_matrix` below — the authoritative matrix.)

    // ── jit.2b-i delegating-shim parity (shim ABI ↔ dispatch_builtin) ──
    //
    // Each shim marshals ABI args → RuntimeValues → dispatch_builtin →
    // marshals back. These tests pin the marshaling against the known
    // VM semantics. (Semantics themselves can't diverge — same
    // dispatch_builtin the VM runs.)

    #[test]
    fn vector_get_in_bounds_and_oob() {
        let vec_ptr = box_for_test(RuntimeValue::Vector(vec![integer(10), integer(20)]));
        // In-bounds → boxed Integer.
        let got = __triet_vector_get(vec_ptr, 1);
        with_rv(got, |rv| expect_integer(rv.unwrap(), 20));
        __triet_drop_arc(got);
        // OOB → boxed Null.
        let oob = __triet_vector_get(vec_ptr, 5);
        with_rv(oob, |rv| assert!(matches!(rv, Some(RuntimeValue::Null))));
        __triet_drop_arc(oob);
        __triet_drop_arc(vec_ptr);
    }

    #[test]
    fn vector_length_matches() {
        let vec_ptr = box_for_test(RuntimeValue::Vector(vec![
            integer(1),
            integer(2),
            integer(3),
        ]));
        assert_eq!(__triet_vector_length(vec_ptr), 3);
        __triet_drop_arc(vec_ptr);
    }

    #[test]
    fn text_from_integer_produces_decimal_string() {
        let ptr = __triet_text_from_integer(-42);
        with_rv(ptr, |rv| match rv {
            Some(RuntimeValue::String(s)) => assert_eq!(s.as_str(), "-42"),
            other => panic!("expected String, got {other:?}"),
        });
        __triet_drop_arc(ptr);
    }

    #[test]
    fn parse_integer_valid_and_invalid() {
        let ok = box_for_test(RuntimeValue::String("123".to_owned()));
        let r = __triet_parse_integer(ok);
        with_rv(r, |rv| expect_integer(rv.unwrap(), 123));
        __triet_drop_arc(r);
        __triet_drop_arc(ok);
        // Non-numeric → Null.
        let bad = box_for_test(RuntimeValue::String("xyz".to_owned()));
        let rb = __triet_parse_integer(bad);
        with_rv(rb, |rv| assert!(matches!(rv, Some(RuntimeValue::Null))));
        __triet_drop_arc(rb);
        __triet_drop_arc(bad);
    }

    #[test]
    fn hashmap_insert_get_contains_roundtrip() {
        let m0 = __triet_hashmap_new();
        let k = box_for_test(RuntimeValue::String("key".to_owned()));
        let v = box_for_test(integer(7));
        let m1 = __triet_hashmap_insert(m0, k, v);
        // get → boxed value 7.
        let got = __triet_hashmap_get(m1, k);
        with_rv(got, |rv| expect_integer(rv.unwrap(), 7));
        __triet_drop_arc(got);
        // contains → True (i8 +1).
        assert_eq!(__triet_hashmap_contains(m1, k), 1);
        // get a missing key → Null.
        let missing = box_for_test(RuntimeValue::String("nope".to_owned()));
        let g2 = __triet_hashmap_get(m1, missing);
        with_rv(g2, |rv| assert!(matches!(rv, Some(RuntimeValue::Null))));
        __triet_drop_arc(g2);
        __triet_drop_arc(missing);
        __triet_drop_arc(m0);
        __triet_drop_arc(m1);
        __triet_drop_arc(k);
        __triet_drop_arc(v);
    }

    #[test]
    fn assert_eq_pass_and_fail() {
        clear_shim_state();
        set_func_name("f");
        let a = box_for_test(integer(5));
        let b = box_for_test(integer(5));
        assert_eq!(__triet_assert_eq(a, b), 0);
        assert!(take_shim_failure().is_none());
        // Mismatch → records failure.
        clear_shim_state();
        let c = box_for_test(integer(6));
        let _ = __triet_assert_eq(a, c);
        assert!(matches!(
            take_shim_failure(),
            Some(VmError::AssertionFailed { .. })
        ));
        __triet_drop_arc(a);
        __triet_drop_arc(b);
        __triet_drop_arc(c);
    }

    #[test]
    fn path_join_and_basename() {
        let base = box_for_test(RuntimeValue::String("a/b".to_owned()));
        let seg = box_for_test(RuntimeValue::String("c".to_owned()));
        let joined = __triet_path_join(base, seg);
        with_rv(joined, |rv| match rv {
            Some(RuntimeValue::String(s)) => assert_eq!(s.as_str(), "a/b/c"),
            other => panic!("expected String, got {other:?}"),
        });
        let bn = __triet_path_basename(joined);
        with_rv(bn, |rv| match rv {
            Some(RuntimeValue::String(s)) => assert_eq!(s.as_str(), "c"),
            other => panic!("expected String, got {other:?}"),
        });
        __triet_drop_arc(joined);
        __triet_drop_arc(bn);
        __triet_drop_arc(base);
        __triet_drop_arc(seg);
    }

    #[test]
    fn string_index_of_found_and_missing() {
        let s = box_for_test(RuntimeValue::String("hello".to_owned()));
        let needle = box_for_test(RuntimeValue::String("ll".to_owned()));
        let found = __triet_string_index_of(s, needle);
        with_rv(found, |rv| expect_integer(rv.unwrap(), 2));
        __triet_drop_arc(found);
        let absent = box_for_test(RuntimeValue::String("z".to_owned()));
        let miss = __triet_string_index_of(s, absent);
        with_rv(miss, |rv| assert!(matches!(rv, Some(RuntimeValue::Null))));
        __triet_drop_arc(miss);
        __triet_drop_arc(s);
        __triet_drop_arc(needle);
        __triet_drop_arc(absent);
    }

    #[test]
    fn text_into_bytes_roundtrip() {
        let s = box_for_test(RuntimeValue::String("Hi".to_owned()));
        let bytes = __triet_text_into_bytes(s);
        // "Hi" → [72, 105].
        with_rv(bytes, |rv| match rv {
            Some(RuntimeValue::Vector(v)) => {
                assert_eq!(v.len(), 2);
                expect_integer(&v[0], 72);
                expect_integer(&v[1], 105);
            }
            other => panic!("expected Vector, got {other:?}"),
        });
        __triet_drop_arc(bytes);
        __triet_drop_arc(s);
    }

    // ── jit.2b-ii Atomic shims ───────────────────────────────────────
    //
    // ordering arg passed as null (0) — VM ignores it on the dev tier
    // (ADR-0028 §9). These exercise the Arc<Mutex> delegation +
    // Outcome marshaling directly.

    #[test]
    fn atomic_new_load_roundtrip() {
        let a = __triet_atomic_new(7);
        assert_eq!(__triet_atomic_load(a, 0), 7);
        __triet_drop_arc(a);
    }

    #[test]
    fn atomic_fetch_add_returns_prev_and_mutates_shared_cell() {
        let a = __triet_atomic_new(10);
        // fetch_add returns the PREVIOUS value.
        assert_eq!(__triet_atomic_fetch_add(a, 5, 0), 10);
        // Subsequent load sees the mutation through the shared Arc<Mutex>.
        assert_eq!(__triet_atomic_load(a, 0), 15);
        __triet_drop_arc(a);
    }

    #[test]
    fn atomic_store_and_swap() {
        let a = __triet_atomic_new(1);
        assert_eq!(__triet_atomic_store(a, 99, 0), 0); // Unit sentinel
        assert_eq!(__triet_atomic_load(a, 0), 99);
        // swap returns previous, installs new.
        assert_eq!(__triet_atomic_swap(a, 42, 0), 99);
        assert_eq!(__triet_atomic_load(a, 0), 42);
        __triet_drop_arc(a);
    }

    #[test]
    fn atomic_compare_exchange_success_and_failure_outcome() {
        let a = __triet_atomic_new(5);
        // Success: expected=5 matches → Outcome ~+ (positive arm),
        // payload = previous value (5), cell becomes 8.
        let ok = __triet_atomic_compare_exchange(a, 5, 8, 0, 0);
        with_rv(ok, |rv| match rv {
            Some(RuntimeValue::Outcome {
                discriminator,
                payload,
            }) => {
                assert!(discriminator.is_positive());
                match payload.as_deref() {
                    Some(RuntimeValue::Integer(i)) => assert_eq!(i.to_i64(), 5),
                    other => panic!("expected prev=5 payload, got {other:?}"),
                }
            }
            other => panic!("expected Outcome, got {other:?}"),
        });
        __triet_drop_arc(ok);
        assert_eq!(__triet_atomic_load(a, 0), 8);
        // Failure: expected=999 ≠ current(8) → Outcome ~- (negative arm).
        let fail = __triet_atomic_compare_exchange(a, 999, 0, 0, 0);
        with_rv(fail, |rv| match rv {
            Some(RuntimeValue::Outcome { discriminator, .. }) => {
                assert!(discriminator.is_negative());
            }
            other => panic!("expected Outcome, got {other:?}"),
        });
        __triet_drop_arc(fail);
        // Cell unchanged on failure.
        assert_eq!(__triet_atomic_load(a, 0), 8);
        __triet_drop_arc(a);
    }

    #[test]
    fn atomic_fetch_bitwise_ops() {
        let a = __triet_atomic_new(0b1100);
        assert_eq!(__triet_atomic_fetch_bitwise_and(a, 0b1010, 0), 0b1100); // prev
        assert_eq!(__triet_atomic_load(a, 0), 0b1000); // 1100 & 1010
        assert_eq!(__triet_atomic_fetch_bitwise_or(a, 0b0011, 0), 0b1000);
        assert_eq!(__triet_atomic_load(a, 0), 0b1011);
        assert_eq!(__triet_atomic_fetch_bitwise_xor(a, 0b1111, 0), 0b1011);
        assert_eq!(__triet_atomic_load(a, 0), 0b0100);
        __triet_drop_arc(a);
    }

    #[test]
    fn atomic_fetch_sub() {
        let a = __triet_atomic_new(20);
        assert_eq!(__triet_atomic_fetch_sub(a, 7, 0), 20); // prev
        assert_eq!(__triet_atomic_load(a, 0), 13);
        __triet_drop_arc(a);
    }

    #[test]
    fn atomic_cross_thread_share_via_shims() {
        // jit.2b-ii deliverable: a counter boxed for the shim ABI can be
        // shared across OS threads (Arc<Mutex>), each thread doing a
        // fetch_add via the shim. After join, the count reflects all
        // increments — proving the JIT shim path is Send-correct.
        use std::sync::Arc;
        use std::sync::Mutex;

        let counter = __triet_atomic_new(0);
        // Extract the inner Arc to share across threads (the boxed
        // RuntimeValue::Atomic holds it).
        let shared: Arc<Mutex<RuntimeValue>> = with_rv(counter, |rv| match rv {
            Some(RuntimeValue::Atomic(arc)) => arc.clone(),
            other => panic!("expected Atomic, got {other:?}"),
        });

        let mut handles = Vec::new();
        for _ in 0..4 {
            let arc = shared.clone();
            handles.push(std::thread::spawn(move || {
                // Re-box the shared Atomic for the shim ABI, fetch_add 1.
                let boxed = box_rv(RuntimeValue::Atomic(arc));
                let _prev = __triet_atomic_fetch_add(boxed, 1, 0);
                __triet_drop_arc(boxed);
            }));
        }
        for h in handles {
            h.join().expect("worker joined");
        }
        // 4 increments from 0 → 4.
        assert_eq!(__triet_atomic_load(counter, 0), 4);
        __triet_drop_arc(counter);
    }

    // ── jit.2b-iii file I/O shims ────────────────────────────────────

    /// Unique temp path under the OS temp dir (no external dep).
    fn temp_path(tag: &str) -> String {
        let mut p = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        p.push(format!("triet_jit_test_{tag}_{nanos}"));
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn file_write_read_exists_roundtrip() {
        let path = temp_path("rw");
        let path_rv = box_for_test(RuntimeValue::String(path.clone()));
        let contents = box_for_test(RuntimeValue::String("Triết I/O".to_owned()));

        // exists → False before write.
        assert_eq!(__triet_file_exists(path_rv), -1);
        // write → True.
        assert_eq!(__triet_write_file(path_rv, contents), 1);
        // exists → True after.
        assert_eq!(__triet_file_exists(path_rv), 1);
        // read → boxed String matching contents.
        let read = __triet_read_file(path_rv);
        with_rv(read, |rv| match rv {
            Some(RuntimeValue::String(s)) => assert_eq!(s.as_str(), "Triết I/O"),
            other => panic!("expected String, got {other:?}"),
        });
        __triet_drop_arc(read);

        // Cleanup.
        let _ = std::fs::remove_file(&path);
        __triet_drop_arc(path_rv);
        __triet_drop_arc(contents);
    }

    #[test]
    fn file_read_missing_returns_null() {
        let path_rv = box_for_test(RuntimeValue::String(temp_path("missing")));
        let read = __triet_read_file(path_rv);
        with_rv(read, |rv| assert!(matches!(rv, Some(RuntimeValue::Null))));
        __triet_drop_arc(read);
        __triet_drop_arc(path_rv);
    }

    #[test]
    fn file_write_bytes_then_read() {
        let path = temp_path("bytes");
        let path_rv = box_for_test(RuntimeValue::String(path.clone()));
        // bytes [72, 105] = "Hi".
        let bytes = box_for_test(RuntimeValue::Vector(vec![integer(72), integer(105)]));
        assert_eq!(__triet_write_file_bytes(path_rv, bytes), 1);
        let read = __triet_read_file(path_rv);
        with_rv(read, |rv| match rv {
            Some(RuntimeValue::String(s)) => assert_eq!(s.as_str(), "Hi"),
            other => panic!("expected String, got {other:?}"),
        });
        __triet_drop_arc(read);
        let _ = std::fs::remove_file(&path);
        __triet_drop_arc(path_rv);
        __triet_drop_arc(bytes);
    }

    // ── Coverage matrix (ADR-0032 §7.2) ──────────────────────────────

    #[test]
    fn shim_coverage_matrix() {
        use BuiltinName as B;
        // 36 JIT-relevant builtins have a shim after jit.2b-i/ii/iii.
        let covered = [
            B::Println,
            B::Print,
            B::Assert,
            B::AssertEq,
            B::TextLen,
            B::TextFromInteger,
            B::ParseInteger,
            B::TextIntoBytes,
            B::TextFromBytes,
            B::VectorNew,
            B::VectorPush,
            B::VectorGet,
            B::VectorLength,
            B::HashMapNew,
            B::HashMapInsert,
            B::HashMapGet,
            B::HashMapKeys,
            B::HashMapContains,
            B::PathJoin,
            B::PathParent,
            B::PathBasename,
            B::StringSubstring,
            B::StringSplit,
            B::StringIndexOf,
            B::Blake3Hash,
            B::GetEnv,
            B::AtomicNew,
            B::AtomicLoad,
            B::AtomicStore,
            B::AtomicSwap,
            B::AtomicCompareExchange,
            B::AtomicFetchAdd,
            B::AtomicFetchSub,
            B::AtomicFetchBitwiseAnd,
            B::AtomicFetchBitwiseOr,
            B::AtomicFetchBitwiseXor,
            B::ReadFile,
            B::WriteFile,
            B::WriteFileBytes,
            B::FileExists,
            B::ReadDirRecursive,
        ];
        for b in covered {
            assert!(builtin_shim(b).is_some(), "{b:?} must have a shim");
        }
        // Cliffs (varargs) + raw_thread (VM-registry) have NO shim —
        // they tier-down to VM. Documented jit.2b-iii deferrals.
        for b in [
            B::FStringConcat,  // varargs — defer v0.11
            B::TextConcat,     // varargs — defer v0.11
            B::RawThreadSpawn, // VM thread registry — not JIT-able
            B::RawThreadJoin,
        ] {
            assert!(builtin_shim(b).is_none(), "{b:?} must tier-down (no shim)");
        }
    }
}

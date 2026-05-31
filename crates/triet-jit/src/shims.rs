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

use triet_ir::{BuiltinName, RuntimeValue, VmError};

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

/// Framework + production shim registry. jit.1 returns ONLY the
/// `__triet_drop_arc` lifetime shim; jit.2 appends the 43 production
/// builtin shims (one [`ShimEntry`] per [`BuiltinName`]). Built once at
/// [`crate::codegen::JitBackend::new`] and registered via
/// `JITBuilder::symbol`.
pub(crate) fn production_shim_entries() -> Vec<ShimEntry> {
    vec![
        // ── Framework shims (no BuiltinName — called implicitly) ──
        ShimEntry {
            builtin: None,
            symbol: "__triet_drop_arc",
            // Cast via `*const ()` per clippy `function_casts_as_integer`
            // — fn item → fn pointer → usize. Recovered to `*const u8`
            // at registration (a no-op address round-trip).
            addr: __triet_drop_arc as *const () as usize,
            signature: ShimSignature {
                params: &[AbiScalar::Ptr],
                ret: None,
            },
        },
        ShimEntry {
            builtin: None,
            symbol: "__triet_shim_failed",
            addr: __triet_shim_failed as *const () as usize,
            signature: ShimSignature {
                params: &[],
                ret: Some(AbiScalar::I8),
            },
        },
        // ── jit.2a representative production shims (5) ──
        // `assert(cond: Trilean, msg: String?)` — cond is i8 (Trilean
        // encoding), msg is a composite ptr (0 = no message). Failure
        // path records VmError + sets SHIM_FAILED, returns Unit sentinel.
        ShimEntry {
            builtin: Some(BuiltinName::Assert),
            symbol: "__triet_assert",
            addr: __triet_assert as *const () as usize,
            signature: ShimSignature {
                params: &[AbiScalar::I8, AbiScalar::Ptr],
                ret: Some(AbiScalar::I8),
            },
        },
        // `println(value)` — composite ptr arg; prints + newline.
        ShimEntry {
            builtin: Some(BuiltinName::Println),
            symbol: "__triet_println",
            addr: __triet_println as *const () as usize,
            signature: ShimSignature {
                params: &[AbiScalar::Ptr],
                ret: Some(AbiScalar::I8),
            },
        },
        // `text.len(s: String) -> Integer` — composite arg → primitive ret.
        ShimEntry {
            builtin: Some(BuiltinName::TextLen),
            symbol: "__triet_text_len",
            addr: __triet_text_len as *const () as usize,
            signature: ShimSignature {
                params: &[AbiScalar::Ptr],
                ret: Some(AbiScalar::I64),
            },
        },
        // `vector.new() -> Vector` — composite ret (box-out).
        ShimEntry {
            builtin: Some(BuiltinName::VectorNew),
            symbol: "__triet_vector_new",
            addr: __triet_vector_new as *const () as usize,
            signature: ShimSignature {
                params: &[],
                ret: Some(AbiScalar::Ptr),
            },
        },
        // `vector.push(v: Vector, x) -> Vector` — functional return-new
        // (clone-and-extend per `triet_vector_functional`).
        ShimEntry {
            builtin: Some(BuiltinName::VectorPush),
            symbol: "__triet_vector_push",
            addr: __triet_vector_push as *const () as usize,
            signature: ShimSignature {
                params: &[AbiScalar::Ptr, AbiScalar::Ptr],
                ret: Some(AbiScalar::Ptr),
            },
        },
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

// ── Representative production shims (jit.2a, ADR-0032 §4 option-2) ───
//
// All are `extern "C"` (never unwind out). On a `VmError`-class failure
// a shim calls `record_shim_failure` + returns a sentinel; the JIT's
// per-call probe branches to `error_exit`.

/// `assert(cond: Trilean, msg: String?)`. Trilean encoding: True = +1
/// (ADR-0010). Passes iff True; otherwise records `AssertionFailed`
/// (with the optional message) + signals failure. Returns Unit (`0`).
extern "C" fn __triet_assert(cond: i8, msg_ptr: i64) -> i8 {
    if cond == 1 {
        return 0;
    }
    let message = with_rv(msg_ptr, |rv| match rv {
        Some(RuntimeValue::String(s)) => Some(s.clone()),
        _ => None,
    });
    record_shim_failure(VmError::AssertionFailed {
        message,
        function: current_func_name(),
    });
    0
}

/// `println(value)` — borrow the composite, print via `Display` +
/// newline. Returns Unit (`0`).
extern "C" fn __triet_println(val_ptr: i64) -> i8 {
    with_rv(val_ptr, |rv| match rv {
        Some(v) => println!("{v}"),
        None => println!(),
    });
    0
}

/// `text.len(s: String) -> Integer` — UTF-8 char count of the borrowed
/// String, returned as a primitive `i64` (Integer is unboxed per §1).
/// Non-String / null borrows yield `0` (defensive — typecheck ensures
/// the arg is a String upstream).
extern "C" fn __triet_text_len(s_ptr: i64) -> i64 {
    with_rv(s_ptr, |rv| match rv {
        Some(RuntimeValue::String(s)) => i64::try_from(s.chars().count()).unwrap_or(i64::MAX),
        _ => 0,
    })
}

/// `vector.new() -> Vector` — fresh empty Vector, boxed out (§2 rule 2).
extern "C" fn __triet_vector_new() -> i64 {
    box_rv(RuntimeValue::Vector(Vec::new()))
}

/// `vector.push(v: Vector, x) -> Vector` — functional return-new per
/// `triet_vector_functional`: clone the borrowed Vector, append a clone
/// of the borrowed element, box the result. Both args are borrowed (not
/// consumed); the JIT drops them at their last use.
extern "C" fn __triet_vector_push(vec_ptr: i64, val_ptr: i64) -> i64 {
    let new_vec = with_rv(vec_ptr, |v| {
        with_rv(val_ptr, |x| match (v, x) {
            (Some(RuntimeValue::Vector(elems)), Some(item)) => {
                let mut next = elems.clone();
                next.push(item.clone());
                RuntimeValue::Vector(next)
            }
            // Defensive: typecheck guarantees (Vector, _) upstream.
            _ => RuntimeValue::Vector(Vec::new()),
        })
    });
    box_rv(new_vec)
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
    fn assert_false_no_message_records_none() {
        clear_shim_state();
        set_func_name("f");
        let ret = __triet_assert(-1, 0); // cond=False, null msg ptr
        assert_eq!(ret, 0);
        match take_shim_failure() {
            Some(VmError::AssertionFailed { message, .. }) => assert!(message.is_none()),
            other => panic!("expected AssertionFailed with no message, got {other:?}"),
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

    #[test]
    fn builtin_shim_lookup_finds_implemented() {
        assert!(builtin_shim(BuiltinName::TextLen).is_some());
        assert!(builtin_shim(BuiltinName::VectorPush).is_some());
        // Not-yet-implemented builtin (jit.2b): no shim.
        assert!(builtin_shim(BuiltinName::HashMapNew).is_none());
    }
}

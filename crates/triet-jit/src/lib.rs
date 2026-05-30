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

mod codegen;

use std::collections::HashMap;

use thiserror::Error;
use triet_ir::FuncId;

use crate::codegen::JitBackend;

/// JIT compiler instance per Triết runtime.
///
/// Owns the Cranelift JIT module + a cache of compiled function
/// pointers indexed by [`FuncId`]. One instance per
/// [`triet_ir::Vm`] (created lazily on first JIT trigger per
/// [ADR-0030 §5] dispatcher integration).
///
/// **v0.9.x.jit.2 status:** `compile` translates arithmetic +
/// comparison + control-flow opcodes (`Add`/`Sub`/`Mul`/`Neg` /
/// `Eq`/`Ne`/`Lt`/`Le`/`Gt`/`Ge` / `Br`/`BrIf`/`BrTrilean`/`Ret`).
/// `Const` operands, calls, builtins, and other opcodes raise
/// [`JitError::UnsupportedOpcode`] and the caller tiers down to
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
    /// initialized lazily on first `compile()` call.
    #[must_use]
    pub fn new() -> Self {
        Self {
            function_cache: HashMap::new(),
            backend: None,
        }
    }

    /// Attempt to JIT-compile `func` and return the native code
    /// pointer on success. v0.9.x.jit.2 ships translation for
    /// arithmetic + comparison + control-flow opcodes; unsupported
    /// opcodes raise [`JitError::UnsupportedOpcode`] for tier-down to
    /// VM-only dispatch per ADR-0030 §2. Inline `Const` operands
    /// + cross-module calls + builtin dispatch defer to `.3`/`.4`.
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
    fn jit2_unsupported_const_opcode_falls_back() {
        // Inline Const operand requires program-level pool wiring.
        // Defer to .3 per ADR-0030 §3; for now compile() must err.
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
            Err(JitError::UnsupportedOpcode { .. }) => {}
            other => panic!("expected UnsupportedOpcode, got {other:?}"),
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
    fn jit2_unsupported_call_opcode_falls_back() {
        // CallLocal defers .3. Verify the tier-down path fires.
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
            Err(JitError::UnsupportedOpcode { .. }) => {}
            other => panic!("expected UnsupportedOpcode, got {other:?}"),
        }
    }
}

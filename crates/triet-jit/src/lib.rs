//! Triết JIT — Cranelift-backed Tier 2 backend per [ADR-0030].
//!
//! v0.9 JIT subsystem. Sub-task progression per [ADR-0030 §11]:
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
        backend.compile_program(program, &mut self.function_cache)
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
    fn jit4_callbuiltin_tierdown_names_the_builtin() {
        // Function calling `println` should tier-down with a
        // diagnostic that names `println` + references the v0.10
        // backlog.
        use triet_ir::BuiltinName;
        let func = make_function(
            "with_println",
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
                    opcode.contains("CallBuiltin(println)"),
                    "diagnostic must name the builtin via its Display impl, got: {opcode}"
                );
                assert!(
                    opcode.contains("v0.10"),
                    "diagnostic must reference the v0.10 backlog, got: {opcode}"
                );
            }
            other => panic!("expected UnsupportedOpcode, got {other:?}"),
        }
    }

    #[test]
    fn jit4_callbuiltin_arg_count_in_diagnostic() {
        // `assert_eq(a, b)` — verify arg count appears in diagnostic.
        use triet_ir::BuiltinName;
        let func = make_function(
            "with_assert_eq",
            vec![],
            TypeTag::Unit,
            vec![
                Instruction::CallBuiltin {
                    dest: None,
                    name: BuiltinName::AssertEq,
                    args: vec![Operand::Value(ValueId(0)), Operand::Value(ValueId(1))],
                },
                Instruction::Ret { value: None },
            ],
        );
        let mut jit = JitCompiler::new();
        match jit.compile(&func) {
            Err(JitError::UnsupportedOpcode { opcode }) => {
                assert!(
                    opcode.contains("2 arg"),
                    "diagnostic must include arg count, got: {opcode}"
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
}

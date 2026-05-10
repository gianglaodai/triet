//! Triết IR — register-based SSA intermediate representation.
//!
//! Per [ADR-0007], this crate defines the stable IR that sits between the
//! AST (parse + typecheck output) and every backend:
//!
//! - v0.3 bytecode VM (development tier)
//! - v0.9 JIT (Cranelift)
//! - v2.0 AOT native (LLVM)
//! - v∞ trytecode (ternary hardware)
//!
//! The IR is **register-based, SSA form, with virtual registers
//! (unlimited count), and type-tagged per register**. Constants are
//! interned in a shared pool and referenced inline as operands —
//! they don't consume a register.
//!
//! # Crate structure
//!
//! | Module | Purpose |
//! |---|---|
//! | [`types`] | `ValueId`, `BlockId`, `FuncId`, `ConstId`, `TypeTag` |
//! | [`constant`] | `Constant` enum + `ConstantPool` with deduplication |
//! | [`instr`] | `Instruction` enum (50+ opcodes), `Operand`, `BuiltinName` |
//! | [`module`] | `BasicBlock`, `Function`, `IrModule`, `IrProgram` |
//! | [`verify`] | SSA invariant verifier (`verify_function`, `verify_program`) |
//! | [`display`] | `Display` impls for disassembly output |
//!
//! [ADR-0007]: ../../../docs/decisions/0007-ir-design.md
//!
//! # Example
//!
//! ```text
//! function @factorial(%n: Integer) -> Integer {
//! entry:
//!     %0 = eq %n, const c0
//!     br_if %0, base_case, recursive_case
//!
//! base_case:
//!     ret const c1
//!
//! recursive_case:
//!     %1 = sub %n, const c1
//!     %2 = call @f0(%1)
//!     %3 = mul %n, %2
//!     ret %3
//! }
//! ```

#![warn(missing_docs)]
// Internal details behind the public types are `pub(crate)`. Silence
// the nursery lint to keep the trade-off consistent across the
// workspace (matches `triet-parser`, `triet-typecheck`, etc.).
#![allow(clippy::redundant_pub_crate)]

mod constant;
mod display;
mod instr;
mod module;
mod types;
mod verify;

// Re-export everything needed by consumers (lowerer, VM, backends).
pub use constant::{Constant, ConstantPool};
pub use instr::{BuiltinName, Instruction, Operand, PhiIncoming};
pub use module::{BasicBlock, Function, IrModule, IrProgram};
pub use types::{BlockId, ConstId, FuncId, TypeTag, ValueId};
pub use verify::{verify_function, verify_program, VerifierResult, VerifierViolation};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instr::PhiIncoming;

    /// Convenience: create an Integer constant from an i64 in range.
    fn int(value: i64) -> triet_core::Integer {
        triet_core::Integer::new(value).unwrap()
    }

    // ── Helper: build a minimal factorial-like function ──────────

    fn make_factorial_func(
        id: FuncId,
        c0: ConstId, // const Integer 0
        c1: ConstId, // const Integer 1
    ) -> Function {
        let v_eq = ValueId(0);
        let v_sub = ValueId(1);
        let v_call = ValueId(2);
        let v_mul = ValueId(3);
        let v_phi = ValueId(4);

        let entry = BlockId(0);
        let base = BlockId(1);
        let recurse = BlockId(2);
        let merge = BlockId(3);

        let func = Function {
            id,
            name: Some("factorial".into()),
            params: vec![("%n".into(), TypeTag::Integer)],
            return_type: TypeTag::Integer,
            blocks: vec![
                // entry block — check n == 0
                BasicBlock {
                    id: entry,
                    name: Some("entry".into()),
                    instructions: vec![
                        Instruction::Eq {
                            dest: v_eq,
                            lhs: Operand::Value(ValueId(0)), // param %n
                            rhs: Operand::Const(c0),
                        },
                        Instruction::BrIf {
                            cond: Operand::Value(v_eq),
                            then_block: base,
                            else_block: recurse,
                        },
                    ],
                },
                // base_case block — return 1
                BasicBlock {
                    id: base,
                    name: Some("base_case".into()),
                    instructions: vec![Instruction::Ret {
                        value: Some(Operand::Const(c1)),
                    }],
                },
                // recursive_case block — n * factorial(n-1)
                BasicBlock {
                    id: recurse,
                    name: Some("recursive_case".into()),
                    instructions: vec![
                        Instruction::Sub {
                            dest: v_sub,
                            lhs: Operand::Value(ValueId(0)), // %n
                            rhs: Operand::Const(c1),
                        },
                        Instruction::CallLocal {
                            dest: Some(v_call),
                            callee: id, // self-recursive
                            args: vec![Operand::Value(v_sub)],
                        },
                        Instruction::Mul {
                            dest: v_mul,
                            lhs: Operand::Value(ValueId(0)), // %n
                            rhs: Operand::Value(v_call),
                        },
                        Instruction::Br {
                            target: merge,
                        },
                    ],
                },
                // merge block (for future phi; just returns recurse result for now)
                BasicBlock {
                    id: merge,
                    name: Some("merge".into()),
                    instructions: vec![
                        Instruction::Phi {
                            dest: v_phi,
                            incoming: vec![
                                PhiIncoming { value: v_mul, block: recurse },
                                PhiIncoming { value: v_eq, block: base },
                            ],
                        },
                        Instruction::Ret {
                            value: Some(Operand::Value(v_phi)),
                        },
                    ],
                },
            ],
        };
        func
    }

    // ── Constant pool ────────────────────────────────────────────

    #[test]
    fn constant_pool_intern_deduplicates() {
        let mut pool = ConstantPool::new();
        let a = pool.intern(Constant::Integer(int(42)));
        let b = pool.intern(Constant::Integer(int(42)));
        assert_eq!(a, b);
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn constant_pool_different_values() {
        let mut pool = ConstantPool::new();
        let a = pool.intern(Constant::Integer(int(1)));
        let b = pool.intern(Constant::Integer(int(2)));
        assert_ne!(a, b);
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn constant_pool_trilean() {
        let mut pool = ConstantPool::new();
        let t = pool.intern(Constant::Trilean(triet_logic::Trilean::True));
        let f = pool.intern(Constant::Trilean(triet_logic::Trilean::False));
        let u = pool.intern(Constant::Trilean(triet_logic::Trilean::Unknown));
        assert_eq!(pool.len(), 3);
        assert_eq!(pool.get(t).unwrap().type_tag(), TypeTag::Trilean);
        assert_eq!(pool.get(f).unwrap().type_tag(), TypeTag::Trilean);
        assert_eq!(pool.get(u).unwrap().type_tag(), TypeTag::Trilean);
    }

    #[test]
    fn constant_pool_string_and_unit() {
        let mut pool = ConstantPool::new();
        let s = pool.intern(Constant::String("hello".into()));
        let u = pool.intern(Constant::Unit);
        assert_eq!(pool.get(s).unwrap().type_tag(), TypeTag::String);
        assert_eq!(pool.get(u).unwrap().type_tag(), TypeTag::Unit);
    }

    // ── Function well-formedness ──────────────────────────────────

    #[test]
    fn factorial_is_well_formed() {
        let mut pool = ConstantPool::new();
        let c0 = pool.intern(Constant::Integer(int(0)));
        let c1 = pool.intern(Constant::Integer(int(1)));
        let func = make_factorial_func(FuncId(0), c0, c1);
        assert!(func.is_well_formed());
    }

    #[test]
    fn function_without_blocks_is_not_well_formed() {
        let func = Function::new(
            FuncId(0),
            Some("empty".into()),
            vec![],
            TypeTag::Unit,
        );
        assert!(!func.is_well_formed());
    }

    // ── Verifier: SSA invariant ──────────────────────────────────

    #[test]
    fn verifier_accepts_factorial() {
        let mut pool = ConstantPool::new();
        let c0 = pool.intern(Constant::Integer(int(0)));
        let c1 = pool.intern(Constant::Integer(int(1)));
        let func = make_factorial_func(FuncId(0), c0, c1);
        let result = verify_function(&func);
        assert!(result.is_ok(), "unexpected violations: {:?}", result.violations);
    }

    #[test]
    fn verifier_rejects_duplicate_definition() {
        // Two instructions writing to the same ValueId.
        let mut pool = ConstantPool::new();
        let c0 = pool.intern(Constant::Integer(int(0)));
        let func = Function {
            id: FuncId(0),
            name: Some("dup_def".into()),
            params: vec![],
            return_type: TypeTag::Unit,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Const {
                        dest: ValueId(0),
                        constant: c0,
                    },
                    Instruction::Const {
                        dest: ValueId(0), // same dest — SSA violation
                        constant: c0,
                    },
                    Instruction::Ret { value: None },
                ],
            }],
        };
        let result = verify_function(&func);
        assert!(result.is_err());
        assert!(result
            .violations
            .iter()
            .any(|v| matches!(v, VerifierViolation::DuplicateDefinition { .. })));
    }

    #[test]
    fn verifier_rejects_undefined_value() {
        // Use ValueId(99) that was never defined.
        let func = Function {
            id: FuncId(0),
            name: Some("undef".into()),
            params: vec![],
            return_type: TypeTag::Unit,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![Instruction::Ret {
                    value: Some(Operand::Value(ValueId(99))),
                }],
            }],
        };
        let result = verify_function(&func);
        assert!(result.is_err());
        assert!(result
            .violations
            .iter()
            .any(|v| matches!(v, VerifierViolation::UndefinedValue { .. })));
    }

    #[test]
    fn verifier_rejects_missing_terminator() {
        let mut pool = ConstantPool::new();
        let c0 = pool.intern(Constant::Integer(int(0)));
        // A Const instruction is not a terminator — block has no terminator.
        let func = Function {
            id: FuncId(0),
            name: Some("no_term".into()),
            params: vec![],
            return_type: TypeTag::Unit,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![Instruction::Const {
                    dest: ValueId(0),
                    constant: c0,
                }],
            }],
        };
        let result = verify_function(&func);
        assert!(result.is_err());
        assert!(result
            .violations
            .iter()
            .any(|v| matches!(v, VerifierViolation::MissingTerminator { .. })));
    }

    #[test]
    fn verifier_rejects_empty_function() {
        let func = Function::new(
            FuncId(0),
            Some("empty".into()),
            vec![],
            TypeTag::Unit,
        );
        let result = verify_function(&func);
        assert!(result.is_err());
        assert!(result
            .violations
            .iter()
            .any(|v| matches!(v, VerifierViolation::EmptyFunction { .. })));
    }

    #[test]
    fn verifier_rejects_phi_out_of_order() {
        // Phi after a non-phi instruction.
        let mut pool = ConstantPool::new();
        let c0 = pool.intern(Constant::Integer(int(0)));
        let func = Function {
            id: FuncId(0),
            name: Some("phi_ooo".into()),
            params: vec![],
            return_type: TypeTag::Unit,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Const {
                        dest: ValueId(0),
                        constant: c0,
                    },
                    Instruction::Phi {
                        dest: ValueId(1),
                        incoming: vec![PhiIncoming {
                            value: ValueId(0),
                            block: BlockId(0),
                        }],
                    },
                    Instruction::Ret { value: None },
                ],
            }],
        };
        let result = verify_function(&func);
        assert!(result.is_err());
        assert!(result
            .violations
            .iter()
            .any(|v| matches!(v, VerifierViolation::PhiOutOfOrder { .. })));
    }

    // ── Display formatting ────────────────────────────────────────

    #[test]
    fn display_function_emits_blocks() {
        let mut pool = ConstantPool::new();
        let c0 = pool.intern(Constant::Integer(int(0)));
        let c1 = pool.intern(Constant::Integer(int(1)));
        let func = make_factorial_func(FuncId(0), c0, c1);
        let s = func.to_string();
        assert!(s.contains("function factorial"));
        assert!(s.contains("entry:"));
        assert!(s.contains("base_case:"));
        assert!(s.contains("recursive_case:"));
        assert!(s.contains("%0 = eq"));
        assert!(s.contains("ret"));
        assert!(s.contains("-> Integer"));
    }

    #[test]
    fn display_operand_value() {
        let op = Operand::Value(ValueId(42));
        assert_eq!(op.to_string(), "%42");
    }

    #[test]
    fn display_operand_const() {
        let op = Operand::Const(ConstId(7));
        assert_eq!(op.to_string(), "const c7");
    }

    #[test]
    fn display_type_tags() {
        assert_eq!(TypeTag::Trit.to_string(), "Trit");
        assert_eq!(TypeTag::Integer.to_string(), "Integer");
        assert_eq!(
            TypeTag::Nullable(Box::new(TypeTag::String)).to_string(),
            "String?"
        );
    }

    // ── Operand extraction ───────────────────────────────────────

    #[test]
    fn instruction_value_operands() {
        let i = Instruction::Add {
            dest: ValueId(0),
            lhs: Operand::Value(ValueId(1)),
            rhs: Operand::Value(ValueId(2)),
        };
        assert_eq!(i.value_operands(), vec![ValueId(1), ValueId(2)]);
    }

    #[test]
    fn instruction_value_operands_mixed() {
        let i = Instruction::Add {
            dest: ValueId(0),
            lhs: Operand::Value(ValueId(3)),
            rhs: Operand::Const(ConstId(1)),
        };
        assert_eq!(i.value_operands(), vec![ValueId(3)]);
    }

    #[test]
    fn terminator_has_no_value_operands() {
        let i = Instruction::Br {
            target: BlockId(0),
        };
        assert!(i.value_operands().is_empty());
    }

    // ── IrProgram ─────────────────────────────────────────────────

    #[test]
    fn empty_ir_program_is_empty() {
        let prog = IrProgram::new();
        assert!(prog.is_empty());
        assert_eq!(prog.function_count(), 0);
    }

    #[test]
    fn ir_program_with_one_module() {
        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::crate_root(),
                    "test".into(),
                ),
                functions: vec![Function::new(
                    FuncId(0),
                    Some("main".into()),
                    vec![],
                    TypeTag::Unit,
                )],
            }],
            constants: ConstantPool::new(),
        };
        assert!(!program.is_empty());
        assert_eq!(program.function_count(), 1);
    }
}

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
//! | [`lowerer`] | AST → IR lowerer (`lower_program`) |
//! | [`serde`] | `.triv` binary format serializer/deserializer |
//! | [`vm`] | Bytecode VM (`Vm`, `RuntimeValue`, `VmError`) |
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
mod lowerer;
mod module;
mod serde;
mod types;
mod verify;
mod vm;

// Re-export everything needed by consumers (lowerer, VM, backends).
pub use constant::{Constant, ConstantPool};
pub use instr::{BuiltinName, Instruction, Operand, PhiIncoming};
pub use lowerer::lower_program;
pub use module::{BasicBlock, Function, IrModule, IrProgram};
pub use serde::{read_program, write_program, TrivError};
pub use types::{BlockId, ConstId, FuncId, TypeTag, ValueId};
pub use verify::{verify_function, verify_program, VerifierResult, VerifierViolation};
pub use vm::{RuntimeValue, Vm, VmError};

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

        
        Function {
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
        }
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

    // ── Verifier: mixed valid/invalid ──────────────────────────────

    #[test]
    fn verifier_accumulates_errors_across_functions() {
        // Valid function
        let mut pool = ConstantPool::new();
        let c0 = pool.intern(Constant::Integer(int(0)));
        let c1 = pool.intern(Constant::Integer(int(1)));
        let valid = make_factorial_func(FuncId(0), c0, c1);

        // Invalid function: missing terminator
        let invalid = Function {
            id: FuncId(1),
            name: Some("broken".into()),
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

        let program = IrProgram {
            modules: vec![IrModule {
                path: triet_modules::AbsolutePath::new(
                    triet_modules::ModulePath::crate_root(),
                    "test".into(),
                ),
                functions: vec![valid, invalid],
            }],
            constants: pool,
        };

        let result = verify_program(&program);
        assert!(result.is_err());
        assert!(result
            .violations
            .iter()
            .any(|v| matches!(v, VerifierViolation::MissingTerminator { .. })));
    }

    #[test]
    fn verifier_detects_multiple_violations_in_one_function() {
        let mut pool = ConstantPool::new();
        let c0 = pool.intern(Constant::Integer(int(0)));
        // Function with: missing terminator AND undefined value use
        let func = Function {
            id: FuncId(0),
            name: Some("multi_bad".into()),
            params: vec![],
            return_type: TypeTag::Unit,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Eq {
                        dest: ValueId(0),
                        lhs: Operand::Value(ValueId(99)), // undefined
                        rhs: Operand::Const(c0),
                    },
                    // No terminator
                ],
            }],
        };
        let result = verify_function(&func);
        assert!(result.is_err());
        let has_undef = result
            .violations
            .iter()
            .any(|v| matches!(v, VerifierViolation::UndefinedValue { .. }));
        let has_missing_term = result
            .violations
            .iter()
            .any(|v| matches!(v, VerifierViolation::MissingTerminator { .. }));
        assert!(has_undef, "should detect undefined value");
        assert!(has_missing_term, "should detect missing terminator");
    }

    #[test]
    fn verifier_function_with_params_treats_params_as_defined() {
        let func = Function {
            id: FuncId(0),
            name: Some("add".into()),
            params: vec![
                ("a".into(), TypeTag::Integer),
                ("b".into(), TypeTag::Integer),
            ],
            return_type: TypeTag::Integer,
            blocks: vec![BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Add {
                        dest: ValueId(2),
                        lhs: Operand::Value(ValueId(0)), // param a
                        rhs: Operand::Value(ValueId(1)), // param b
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(2))),
                    },
                ],
            }],
        };
        let result = verify_function(&func);
        assert!(result.is_ok(), "violations: {:?}", result.violations);
    }

    // ── Verifier: phi predecessor validation ───────────────────────

    #[test]
    fn verifier_rejects_phi_predecessor_from_nonexistent_block() {
        let func = Function {
            id: FuncId(0),
            name: Some("bad_phi".into()),
            params: vec![],
            return_type: TypeTag::Unit,
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    name: Some("entry".into()),
                    instructions: vec![Instruction::Br {
                        target: BlockId(1),
                    }],
                },
                BasicBlock {
                    id: BlockId(1),
                    name: Some("merge".into()),
                    instructions: vec![
                        Instruction::Phi {
                            dest: ValueId(0),
                            incoming: vec![PhiIncoming {
                                value: ValueId(0),
                                block: BlockId(99), // nonexistent!
                            }],
                        },
                        Instruction::Ret { value: None },
                    ],
                },
            ],
        };
        let result = verify_function(&func);
        assert!(result.is_err());
        assert!(result
            .violations
            .iter()
            .any(|v| matches!(v, VerifierViolation::InvalidPhiPredecessor { .. })));
    }

    // ── IR types: BasicBlock edge cases ────────────────────────────

    #[test]
    fn basic_block_phis_returns_only_phi_instructions() {
        let block = BasicBlock {
            id: BlockId(0),
            name: Some("test".into()),
            instructions: vec![
                Instruction::Phi {
                    dest: ValueId(0),
                    incoming: vec![],
                },
                Instruction::Phi {
                    dest: ValueId(1),
                    incoming: vec![],
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
        };
        let phis: Vec<_> = block.phis().collect();
        assert_eq!(phis.len(), 2);
        assert!(phis[0].is_phi());
        assert!(phis[1].is_phi());
    }

    #[test]
    fn basic_block_body_skips_phis_and_terminator() {
        let block = BasicBlock {
            id: BlockId(0),
            name: Some("test".into()),
            instructions: vec![
                Instruction::Phi {
                    dest: ValueId(0),
                    incoming: vec![],
                },
                Instruction::Const {
                    dest: ValueId(1),
                    constant: ConstId(0),
                },
                Instruction::Ret { value: None },
            ],
        };
        let body: Vec<_> = block.body().collect();
        assert_eq!(body.len(), 1);
        assert!(matches!(body[0], Instruction::Const { .. }));
    }

    #[test]
    fn basic_block_incoming_edges_collects_predecessors() {
        let b0 = BlockId(0);
        let b1 = BlockId(1);
        let block = BasicBlock {
            id: BlockId(2),
            name: Some("merge".into()),
            instructions: vec![
                Instruction::Phi {
                    dest: ValueId(0),
                    incoming: vec![
                        PhiIncoming { value: ValueId(1), block: b0 },
                        PhiIncoming { value: ValueId(2), block: b1 },
                    ],
                },
                Instruction::Ret { value: None },
            ],
        };
        let edges = block.incoming_edges();
        assert_eq!(edges.len(), 2);
    }

    // ── IR types: Function edge cases ──────────────────────────────

    #[test]
    fn function_all_value_dests_with_multiple_blocks() {
        let func = Function {
            id: FuncId(0),
            name: Some("multi".into()),
            params: vec![("x".into(), TypeTag::Integer)],
            return_type: TypeTag::Integer,
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    name: Some("entry".into()),
                    instructions: vec![
                        Instruction::Eq {
                            dest: ValueId(1),
                            lhs: Operand::Value(ValueId(0)),
                            rhs: Operand::Const(ConstId(0)),
                        },
                        Instruction::BrIf {
                            cond: Operand::Value(ValueId(1)),
                            then_block: BlockId(1),
                            else_block: BlockId(2),
                        },
                    ],
                },
                BasicBlock {
                    id: BlockId(1),
                    name: Some("zero".into()),
                    instructions: vec![Instruction::Ret {
                        value: Some(Operand::Value(ValueId(0))),
                    }],
                },
                BasicBlock {
                    id: BlockId(2),
                    name: Some("non_zero".into()),
                    instructions: vec![
                        Instruction::Add {
                            dest: ValueId(2),
                            lhs: Operand::Value(ValueId(0)),
                            rhs: Operand::Const(ConstId(1)),
                        },
                        Instruction::Ret {
                            value: Some(Operand::Value(ValueId(2))),
                        },
                    ],
                },
            ],
        };
        let dests = func.all_value_dests();
        // ValueId(0) is param (not from instruction). Values 1 and 2 are dests.
        assert_eq!(dests.len(), 2);
    }

    // ── IR types: multi-module program ─────────────────────────────

    #[test]
    fn ir_program_with_multiple_modules_counts_functions() {
        let program = IrProgram {
            modules: vec![
                IrModule {
                    path: triet_modules::AbsolutePath::new(
                        triet_modules::ModulePath::crate_root(),
                        "root".into(),
                    ),
                    functions: vec![
                        Function::new(FuncId(0), Some("main".into()), vec![], TypeTag::Unit),
                    ],
                },
                IrModule {
                    path: triet_modules::AbsolutePath::new(
                        triet_modules::ModulePath::crate_root().child("utils"),
                        "utils".into(),
                    ),
                    functions: vec![
                        Function::new(FuncId(1), Some("helper".into()), vec![], TypeTag::Integer),
                        Function::new(
                            FuncId(2),
                            Some("another".into()),
                            vec![],
                            TypeTag::Trilean,
                        ),
                    ],
                },
            ],
            constants: ConstantPool::new(),
        };
        assert!(!program.is_empty());
        assert_eq!(program.function_count(), 3);
    }

    // ── Operand: edge cases ────────────────────────────────────────

    #[test]
    fn operand_from_value_id() {
        let v = ValueId(5);
        let op: Operand = v.into();
        assert_eq!(op, Operand::Value(ValueId(5)));
    }

    #[test]
    fn operand_from_const_id() {
        let c = ConstId(3);
        let op: Operand = c.into();
        assert_eq!(op, Operand::Const(ConstId(3)));
    }

    #[test]
    fn instruction_destinations_for_all_variants() {
        // Ensure destination() returns Some for value-producing instructions
        let instrs: Vec<(Instruction, bool)> = vec![
            (Instruction::Const { dest: ValueId(0), constant: ConstId(0) }, true),
            (Instruction::Add { dest: ValueId(1), lhs: Operand::Value(ValueId(0)), rhs: Operand::Const(ConstId(0)) }, true),
            (Instruction::Br { target: BlockId(0) }, false),
            (Instruction::BrIf { cond: Operand::Value(ValueId(0)), then_block: BlockId(0), else_block: BlockId(1) }, false),
            (Instruction::Ret { value: None }, false),
            (Instruction::Unreachable, false),
        ];
        for (instr, should_have_dest) in &instrs {
            assert_eq!(instr.destination().is_some(), *should_have_dest, "mismatch for {instr:?}");
        }
    }

    // ── Phi edge cases ─────────────────────────────────────────────

    #[test]
    fn phi_with_multiple_incoming_edges_collects_value_operands() {
        let phi = Instruction::Phi {
            dest: ValueId(0),
            incoming: vec![
                PhiIncoming { value: ValueId(10), block: BlockId(0) },
                PhiIncoming { value: ValueId(11), block: BlockId(1) },
                PhiIncoming { value: ValueId(12), block: BlockId(2) },
            ],
        };
        let operands = phi.value_operands();
        assert_eq!(operands, vec![ValueId(10), ValueId(11), ValueId(12)]);
    }
}

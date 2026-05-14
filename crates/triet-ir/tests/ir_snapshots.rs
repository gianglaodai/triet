//! Snapshot tests for IR display output.
//!
//! These tests capture the Display rendering of key IR programs.
//! If a change to the lowerer or instruction set alters the output,
//! these snapshots will fail — review the diff and update with
//! `cargo insta review` or `cargo test -- --accept`.

use triet_core::Integer;
use triet_ir::{
    BasicBlock, BlockId, Constant, ConstantPool, FuncId, Function, Instruction, IrModule,
    IrProgram, Operand, PhiIncoming, TypeTag, ValueId,
};
use triet_modules::{AbsolutePath, ModulePath};

const fn int(n: i64) -> Integer {
    Integer::new(n).unwrap()
}

/// Build the factorial function in IR form.
fn make_factorial_ir() -> IrProgram {
    let mut pool = ConstantPool::new();
    let c0 = pool.intern(Constant::Integer(int(0)));
    let c1 = pool.intern(Constant::Integer(int(1)));

    let v_eq = ValueId(1);
    let v_sub = ValueId(2);
    let v_call = ValueId(3);
    let v_mul = ValueId(4);

    let func = Function {
        id: FuncId(0),
        name: Some("factorial".into()),
        params: vec![("%n".into(), TypeTag::Integer)],
        return_type: TypeTag::Integer,
        blocks: vec![
            BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![
                    Instruction::Eq {
                        dest: v_eq,
                        lhs: Operand::Value(ValueId(0)),
                        rhs: Operand::Const(c0),
                    },
                    Instruction::BrIf {
                        cond: Operand::Value(v_eq),
                        then_block: BlockId(1),
                        else_block: BlockId(2),
                    },
                ],
            },
            BasicBlock {
                id: BlockId(1),
                name: Some("base_case".into()),
                instructions: vec![Instruction::Ret {
                    value: Some(Operand::Const(c1)),
                }],
            },
            BasicBlock {
                id: BlockId(2),
                name: Some("recursive_case".into()),
                instructions: vec![
                    Instruction::Sub {
                        dest: v_sub,
                        lhs: Operand::Value(ValueId(0)),
                        rhs: Operand::Const(c1),
                    },
                    Instruction::CallLocal {
                        dest: Some(v_call),
                        callee: FuncId(0),
                        args: vec![Operand::Value(v_sub)],
                    },
                    Instruction::Mul {
                        dest: v_mul,
                        lhs: Operand::Value(ValueId(0)),
                        rhs: Operand::Value(v_call),
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(v_mul)),
                    },
                ],
            },
        ],
    };

    IrProgram {
        modules: vec![IrModule {
            path: AbsolutePath::new(ModulePath::crate_root(), "test".into()),
            functions: vec![func],
        }],
        constants: pool,
    }
}

/// Build an if-else IR program with phi merge.
fn make_if_else_ir() -> IrProgram {
    let mut pool = ConstantPool::new();
    let c1 = pool.intern(Constant::Integer(int(1)));
    let c0 = pool.intern(Constant::Integer(int(0)));

    let func = Function {
        id: FuncId(0),
        name: Some("choose".into()),
        params: vec![("%cond".into(), TypeTag::Trilean)],
        return_type: TypeTag::Integer,
        blocks: vec![
            BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![Instruction::BrIf {
                    cond: Operand::Value(ValueId(0)),
                    then_block: BlockId(1),
                    else_block: BlockId(2),
                }],
            },
            BasicBlock {
                id: BlockId(1),
                name: Some("then".into()),
                instructions: vec![
                    Instruction::Const {
                        dest: ValueId(1),
                        constant: c1,
                    },
                    Instruction::Br { target: BlockId(3) },
                ],
            },
            BasicBlock {
                id: BlockId(2),
                name: Some("else".into()),
                instructions: vec![
                    Instruction::Const {
                        dest: ValueId(2),
                        constant: c0,
                    },
                    Instruction::Br { target: BlockId(3) },
                ],
            },
            BasicBlock {
                id: BlockId(3),
                name: Some("merge".into()),
                instructions: vec![
                    Instruction::Phi {
                        dest: ValueId(3),
                        incoming: vec![
                            PhiIncoming {
                                value: ValueId(1),
                                block: BlockId(1),
                            },
                            PhiIncoming {
                                value: ValueId(2),
                                block: BlockId(2),
                            },
                        ],
                    },
                    Instruction::Ret {
                        value: Some(Operand::Value(ValueId(3))),
                    },
                ],
            },
        ],
    };

    IrProgram {
        modules: vec![IrModule {
            path: AbsolutePath::new(ModulePath::crate_root(), "test".into()),
            functions: vec![func],
        }],
        constants: pool,
    }
}

/// Build a while-like loop IR program.
fn make_while_loop_ir() -> IrProgram {
    let mut pool = ConstantPool::new();
    let c0 = pool.intern(Constant::Integer(int(0)));
    let c1 = pool.intern(Constant::Integer(int(1)));

    let func = Function {
        id: FuncId(0),
        name: Some("countdown".into()),
        params: vec![("%n".into(), TypeTag::Integer)],
        return_type: TypeTag::Integer,
        blocks: vec![
            BasicBlock {
                id: BlockId(0),
                name: Some("entry".into()),
                instructions: vec![Instruction::Br { target: BlockId(1) }],
            },
            BasicBlock {
                id: BlockId(1),
                name: Some("while_header".into()),
                instructions: vec![
                    Instruction::Gt {
                        dest: ValueId(1),
                        lhs: Operand::Value(ValueId(0)),
                        rhs: Operand::Const(c0),
                    },
                    Instruction::BrIf {
                        cond: Operand::Value(ValueId(1)),
                        then_block: BlockId(2),
                        else_block: BlockId(3),
                    },
                ],
            },
            BasicBlock {
                id: BlockId(2),
                name: Some("while_body".into()),
                instructions: vec![
                    Instruction::Sub {
                        dest: ValueId(2),
                        lhs: Operand::Value(ValueId(0)),
                        rhs: Operand::Const(c1),
                    },
                    Instruction::Br { target: BlockId(1) },
                ],
            },
            BasicBlock {
                id: BlockId(3),
                name: Some("while_exit".into()),
                instructions: vec![Instruction::Ret {
                    value: Some(Operand::Value(ValueId(0))),
                }],
            },
        ],
    };

    IrProgram {
        modules: vec![IrModule {
            path: AbsolutePath::new(ModulePath::crate_root(), "test".into()),
            functions: vec![func],
        }],
        constants: pool,
    }
}

// ── Snapshot tests ─────────────────────────────────────────────────

#[test]
fn snapshot_factorial_ir() {
    let ir = make_factorial_ir();
    insta::assert_yaml_snapshot!(ir.to_string());
}

#[test]
fn snapshot_if_else_ir() {
    let ir = make_if_else_ir();
    insta::assert_yaml_snapshot!(ir.to_string());
}

#[test]
fn snapshot_while_loop_ir() {
    let ir = make_while_loop_ir();
    insta::assert_yaml_snapshot!(ir.to_string());
}

#[test]
fn snapshot_empty_ir_program() {
    let ir = IrProgram::new();
    insta::assert_yaml_snapshot!(ir.to_string());
}

//! IR module and program types — the top-level containers for functions,
//! basic blocks, and the full resolved program in IR form.
//!
//! Per [ADR-0007], an `IrProgram` is the IR equivalent of
//! `triet_modules::ResolvedProgram`: a flat list of modules each
//! containing IR functions, ready for lowering to any backend.
//!
//! [ADR-0007]: ../../../docs/decisions/0007-ir-design.md

use triet_modules::AbsolutePath;

use crate::instr::{Instruction, PhiIncoming};
use crate::types::{BlockId, FuncId, TypeTag, ValueId};

/// A basic block — a straight-line sequence of non-terminator instructions
/// ending with exactly one terminator. Phi nodes must appear contiguously
/// at the top of the block (before any other instruction).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BasicBlock {
    /// Block label (unique within the function).
    pub id: BlockId,
    /// Human-readable label for debugging (e.g. `entry`, `loop_body`).
    pub name: Option<String>,
    /// Instructions in execution order, phi nodes first, terminator last.
    pub instructions: Vec<Instruction>,
}

impl BasicBlock {
    /// Create a new empty block. The caller must add instructions and
    /// ensure a terminator is present.
    #[must_use]
    pub const fn new(id: BlockId, name: Option<String>) -> Self {
        Self {
            id,
            name,
            instructions: Vec::new(),
        }
    }

    /// Return all phi nodes in this block (must be at the top).
    pub fn phis(&self) -> impl Iterator<Item = &Instruction> {
        self.instructions.iter().take_while(|i| matches!(i, Instruction::Phi { .. }))
    }

    /// Return all non-phi, non-terminator instructions.
    pub fn body(&self) -> impl Iterator<Item = &Instruction> {
        self.instructions
            .iter()
            .skip_while(|i| matches!(i, Instruction::Phi { .. }))
            .filter(|i| !i.is_terminator())
    }

    /// Return the terminator instruction, or None if the block is malformed.
    #[must_use]
    pub fn terminator(&self) -> Option<&Instruction> {
        self.instructions.last().filter(|i| i.is_terminator())
    }

    /// Iterator over all `PhiIncoming` for predecessor blocks of this
    /// basic block. Used by the SSA verifier.
    pub fn incoming_edges(&self) -> Vec<(BlockId, PhiIncoming)> {
        self.phis()
            .filter_map(|i| {
                if let Instruction::Phi { incoming, .. } = i {
                    Some(incoming.iter().map(|p| (p.block, *p)))
                } else {
                    None
                }
            })
            .flatten()
            .collect()
    }

    /// The value defined by each phi in this block.
    pub fn phi_dests(&self) -> Vec<ValueId> {
        self.phis()
            .filter_map(|i| {
                if let Instruction::Phi { dest, .. } = i {
                    Some(*dest)
                } else {
                    None
                }
            })
            .collect()
    }
}

/// An IR function — a sequence of basic blocks forming a control-flow graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    /// Unique function ID within the IR program.
    pub id: FuncId,
    /// Human-readable name (e.g. `factorial`, `main`).
    pub name: Option<String>,
    /// Parameter names — each parameter is a value available in the
    /// entry block. Parameters are assigned `ValueId`s in order starting
    /// from a function-local counter.
    pub params: Vec<(String, TypeTag)>,
    /// Return type.
    pub return_type: TypeTag,
    /// All basic blocks. The first block is the entry block.
    pub blocks: Vec<BasicBlock>,
}

impl Function {
    /// Create a new function with the given signature.
    #[must_use]
    pub const fn new(
        id: FuncId,
        name: Option<String>,
        params: Vec<(String, TypeTag)>,
        return_type: TypeTag,
    ) -> Self {
        Self {
            id,
            name,
            params,
            return_type,
            blocks: Vec::new(),
        }
    }

    /// The entry block (must exist in a valid function).
    #[must_use]
    pub fn entry_block(&self) -> Option<&BasicBlock> {
        self.blocks.first()
    }

    /// Collect all value destinations across all blocks. Used by the
    /// SSA verifier to ensure each value is defined exactly once.
    #[must_use]
    pub fn all_value_dests(&self) -> Vec<(ValueId, &Instruction)> {
        self.blocks
            .iter()
            .flat_map(|b| {
                b.instructions.iter().filter_map(|i| {
                    i.destination().map(|dest| (dest, i))
                })
            })
            .collect()
    }

    /// Collect all value uses (operands) across all blocks. Used by the
    /// SSA verifier to ensure every use has a definition.
    #[must_use]
    pub fn all_value_uses(&self) -> Vec<ValueId> {
        self.blocks
            .iter()
            .flat_map(|b| {
                b.instructions.iter().flat_map(super::instr::Instruction::value_operands)
            })
            .collect()
    }

    /// True if the function is well-formed (has at least an entry block,
    /// every block ends with a terminator).
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        if self.blocks.is_empty() {
            return false;
        }
        self.blocks
            .iter()
            .all(|b| b.terminator().is_some())
    }
}

/// An IR module — the IR content of one source file after lowering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IrModule {
    /// The module's absolute path (from name resolution).
    pub path: AbsolutePath,
    /// Functions defined in this module.
    pub functions: Vec<Function>,
}

/// A resolved program in IR form — the output of the lowerer, ready for
/// any backend (VM, JIT, AOT, trytecode).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IrProgram {
    /// All modules in dependency order.
    pub modules: Vec<IrModule>,
    /// The shared constant pool.
    pub constants: crate::constant::ConstantPool,
}

impl IrProgram {
    /// Create an empty IR program.
    #[must_use]
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
            constants: crate::constant::ConstantPool::new(),
        }
    }

    /// True if the program contains no functions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.modules.iter().all(|m| m.functions.is_empty())
    }

    /// Total function count across all modules.
    #[must_use]
    pub fn function_count(&self) -> usize {
        self.modules.iter().map(|m| m.functions.len()).sum()
    }
}

impl Default for IrProgram {
    fn default() -> Self {
        Self::new()
    }
}

impl Instruction {
    /// Return the destination `ValueId` if this instruction produces a value.
    #[must_use]
    pub const fn destination(&self) -> Option<ValueId> {
        match self {
            Self::Const { dest, .. }
            | Self::Add { dest, .. }
            | Self::Sub { dest, .. }
            | Self::Mul { dest, .. }
            | Self::Div { dest, .. }
            | Self::Mod { dest, .. }
            | Self::Pow { dest, .. }
            | Self::Neg { dest, .. }
            | Self::LukAnd { dest, .. }
            | Self::LukOr { dest, .. }
            | Self::LukImplies { dest, .. }
            | Self::LukXor { dest, .. }
            | Self::LukIff { dest, .. }
            | Self::KleeneImplies { dest, .. }
            | Self::KleeneXor { dest, .. }
            | Self::KleeneIff { dest, .. }
            | Self::Eq { dest, .. }
            | Self::Ne { dest, .. }
            | Self::Lt { dest, .. }
            | Self::Le { dest, .. }
            | Self::Gt { dest, .. }
            | Self::Ge { dest, .. }
            | Self::ToInteger { dest, .. }
            | Self::ToTryte { dest, .. }
            | Self::ToLong { dest, .. }
            | Self::ToTrit { dest, .. }
            | Self::ToTrilean { dest, .. }
            | Self::StructNew { dest, .. }
            | Self::FieldGet { dest, .. }
            | Self::FieldSet { dest, .. }
            | Self::EnumNew { dest, .. }
            | Self::EnumTag { dest, .. }
            | Self::EnumPayload { dest, .. }
            | Self::NullWrap { dest, .. }
            | Self::NullUnwrap { dest, .. }
            | Self::NullCheck { dest, .. }
            | Self::ClosureNew { dest, .. }
            | Self::Phi { dest, .. } => Some(*dest),
            Self::CallLocal { dest, .. }
            | Self::CallCrossModule { dest, .. }
            | Self::CallBuiltin { dest, .. }
            | Self::ClosureCall { dest, .. } => *dest,
            Self::Br { .. }
            | Self::BrIf { .. }
            | Self::Ret { .. }
            | Self::Unreachable => None,
        }
    }

    /// True if this is a terminator (must appear last in a block).
    #[must_use]
    pub const fn is_terminator(&self) -> bool {
        matches!(
            self,
            Self::Br { .. }
                | Self::BrIf { .. }
                | Self::Ret { .. }
                | Self::Unreachable
        )
    }

    /// True if this is a phi node (must appear first in a block).
    #[must_use]
    pub const fn is_phi(&self) -> bool {
        matches!(self, Self::Phi { .. })
    }

    /// Extract all `ValueId` operands from this instruction. Used by
    /// the SSA verifier to check that every use has a definition.
    #[must_use]
    pub fn value_operands(&self) -> Vec<ValueId> {
        let mut operands = Vec::new();
        self.collect_value_operands(&mut operands);
        operands
    }

    fn collect_value_operands(&self, out: &mut Vec<ValueId>) {
        match self {
            Self::Const { .. } => {}
            Self::Add { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::Sub { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::Mul { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::Div { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::Mod { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::Eq { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::Ne { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::Lt { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::Le { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::Gt { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::Ge { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::LukAnd { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::LukOr { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::LukImplies { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::LukXor { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::LukIff { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::KleeneImplies { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::KleeneXor { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::KleeneIff { lhs, rhs, .. } => {
                lhs.collect_value(out);
                rhs.collect_value(out);
            }
            Self::Pow { base, exp, .. } => {
                base.collect_value(out);
                exp.collect_value(out);
            }
            Self::Neg { operand, .. } => {
                operand.collect_value(out);
            }
            Self::ToInteger { operand, .. } => {
                operand.collect_value(out);
            }
            Self::ToTryte { operand, .. } => {
                operand.collect_value(out);
            }
            Self::ToLong { operand, .. } => {
                operand.collect_value(out);
            }
            Self::ToTrit { operand, .. } => {
                operand.collect_value(out);
            }
            Self::ToTrilean { operand, .. } => {
                operand.collect_value(out);
            }
            Self::EnumTag { scrutinee, .. } => {
                scrutinee.collect_value(out);
            }
            Self::EnumPayload { scrutinee, .. } => {
                scrutinee.collect_value(out);
            }
            Self::NullUnwrap { nullable, .. } => {
                nullable.collect_value(out);
            }
            Self::NullCheck { nullable, .. } => {
                nullable.collect_value(out);
            }
            Self::NullWrap { value, .. } => {
                value.collect_value(out);
            }
            Self::FieldGet { object, .. } => {
                object.collect_value(out);
            }
            Self::FieldSet {
                object, value, ..
            } => {
                object.collect_value(out);
                value.collect_value(out);
            }
            Self::StructNew { fields, .. } => {
                for f in fields {
                    f.collect_value(out);
                }
            }
            Self::EnumNew { payload, .. } => {
                if let Some(p) = payload {
                    p.collect_value(out);
                }
            }
            Self::CallLocal { args, .. } => {
                for a in args {
                    a.collect_value(out);
                }
            }
            Self::CallCrossModule { args, .. } => {
                for a in args {
                    a.collect_value(out);
                }
            }
            Self::CallBuiltin { args, .. } => {
                for a in args {
                    a.collect_value(out);
                }
            }
            Self::ClosureCall { closure, args, .. } => {
                closure.collect_value(out);
                for a in args {
                    a.collect_value(out);
                }
            }
            Self::ClosureNew { captures, .. } => {
                out.extend(captures);
            }
            Self::BrIf { cond, .. } => {
                cond.collect_value(out);
            }
            Self::Ret { value, .. } => {
                if let Some(v) = value {
                    v.collect_value(out);
                }
            }
            Self::Phi { incoming, .. } => {
                for p in incoming {
                    out.push(p.value);
                }
            }
            Self::Br { .. } | Self::Unreachable => {}
        }
    }
}

impl super::instr::Operand {
    fn collect_value(self, out: &mut Vec<ValueId>) {
        if let Self::Value(v) = self {
            out.push(v);
        }
    }
}

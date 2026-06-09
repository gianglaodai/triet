//! Triết Borrow Checker — S6 ownership enforcement.
//!
//! Performs NLL (Non-Lexical Lifetime) borrow checking on MIR via
//! backward dataflow liveness analysis + forward loan tracking over
//! the control-flow graph.
//!
//! # Architecture
//!
//! ```text
//! MIR Body → build_cfg() → ControlFlowGraph
//!                          → liveness analysis (backward dataflow)
//!                          → loan tracking (forward dataflow)
//!                          → conflict detection
//!                          → BorrowCheckResult (pass / Vec<E24XX>)
//! ```
//!
//! Currently implements: MIR construction, CFG building, liveness
//! analysis, and intra-block loan/conflict detection.
//! Inter-procedural + full dataflow is Phase 2.2.

#![warn(missing_docs)]

pub mod checker;
pub mod liveness;

use triet_mir::{
    BasicBlock, BinOp, BlockData, Body, CallTarget, ConstValue, DUMMY_SPAN, FunctionId,
    FunctionSignature, Local, LocalDecl, MirType, ParameterPassing, Place, ReferenceForm,
    ReturnShape, Statement, Terminator,
};

use std::collections::{BTreeMap, HashMap};

// ── MIR builder (convenience API for constructing MIR by hand) ─

/// A builder for constructing MIR bodies programmatically.
///
/// Used by tests and the AST→MIR lowering pass.
#[derive(Debug)]
pub struct MirBuilder {
    signature: FunctionSignature,
    blocks: Vec<BlockData>,
    next_local: usize,
    next_block: usize,
    next_func: usize,
    /// Cache of function name → FunctionId for `func_id_for`.
    func_ids: HashMap<String, FunctionId>,
    /// Types for locals.
    local_types: HashMap<Local, String>,
    /// Struct layouts for type resolution.
    struct_layouts: Vec<triet_mir::StructLayout>,
    /// Enum layouts for type resolution.
    enum_layouts: Vec<triet_mir::EnumLayout>,
}

impl MirBuilder {
    /// Create a new MIR builder for a function.
    #[must_use]
    pub fn new(name: &str, return_type: impl Into<MirType>) -> Self {
        Self {
            signature: FunctionSignature {
                name: name.to_string(),
                params: Vec::new(),
                return_type: return_type.into(),
                return_borrow_map: triet_mir::ReturnBorrowMap::new(),
                return_shape: triet_mir::ReturnShape::Scalar,
            },
            blocks: Vec::new(),
            next_local: 0,
            next_block: 0,
            next_func: 0,
            func_ids: HashMap::new(),
            local_types: HashMap::new(),
            struct_layouts: Vec::new(),
            enum_layouts: Vec::new(),
        }
    }

    /// Add a parameter with the given passing mode.
    pub fn add_param(&mut self, name: &str, passing: ParameterPassing) -> Local {
        let local = self.new_local();
        self.signature.params.push((name.to_string(), passing));
        local
    }

    /// Record that the given return field path borrows from the given
    /// parameter indices (drives cross-call loan propagation).
    pub fn set_return_borrow(&mut self, path: triet_mir::FieldPath, param_indices: Vec<usize>) {
        self.signature
            .return_borrow_map
            .insert(path, param_indices.into_iter().collect());
    }

    /// Set the return shape of the function.
    pub fn set_return_shape(&mut self, shape: triet_mir::ReturnShape) {
        self.signature.return_shape = shape;
    }

    /// Allocate a fresh local.
    pub fn new_local(&mut self) -> Local {
        let l = Local(self.next_local);
        self.next_local += 1;
        l
    }

    /// Set the type of a local.
    pub fn set_local_type(&mut self, local: Local, ty: &str) {
        self.local_types.insert(local, ty.to_string());
    }

    /// Add a struct layout for type resolution (used by `place_type`).
    pub fn add_struct_layout(&mut self, layout: triet_mir::StructLayout) {
        self.struct_layouts.push(layout);
    }

    /// Add an enum layout for type resolution (used by `place_type`).
    pub fn add_enum_layout(&mut self, layout: triet_mir::EnumLayout) {
        self.enum_layouts.push(layout);
    }

    /// Allocate a fresh basic block.
    pub fn new_block(&mut self) -> BasicBlock {
        let bb = BasicBlock(self.next_block);
        self.next_block += 1;
        self.blocks.push(BlockData {
            statements: Vec::new(),
            terminator: Terminator::Unreachable { span: DUMMY_SPAN },
        });
        bb
    }

    /// Allocate a fresh function ID.
    pub fn new_func_id(&mut self) -> FunctionId {
        let f = FunctionId(self.next_func);
        self.next_func += 1;
        f
    }

    /// Get or allocate a function ID for a named callee.
    ///
    /// Returns the same ID for the same name within a builder session,
    /// so that multiple `CallDispatch` sites referencing the same callee
    /// share the same `FunctionId`.
    pub fn func_id_for(&mut self, name: &str) -> FunctionId {
        if let Some(&id) = self.func_ids.get(name) {
            return id;
        }
        let id = self.new_func_id();
        self.func_ids.insert(name.to_string(), id);
        id
    }

    /// Push a statement to the given block.
    pub fn push(&mut self, block: BasicBlock, stmt: Statement) {
        self.blocks[block.0].statements.push(stmt);
    }

    /// Set the terminator of a block.
    pub fn set_terminator(&mut self, block: BasicBlock, term: Terminator) {
        self.blocks[block.0].terminator = term;
    }

    /// Build the MIR body.
    #[must_use]
    pub fn build(self, entry: BasicBlock) -> Body {
        let local_decls = (0..self.next_local)
            .map(|i| {
                let ty = self
                    .local_types
                    .get(&Local(i))
                    .map(|s| s.as_str())
                    .unwrap_or("?");
                LocalDecl::new(ty)
            })
            .collect();
        Body {
            signature: self.signature,
            blocks: self.blocks,
            entry_block: entry,
            num_locals: self.next_local,
            local_decls,
            struct_layouts: self.struct_layouts,
            enum_layouts: self.enum_layouts,
            local_names: BTreeMap::new(),
        }
    }
}

// ── Helpers for constructing statements ─────────────────────

/// `StorageLive(local)`
#[must_use]
pub fn storage_live(local: Local) -> Statement {
    Statement::StorageLive(local, DUMMY_SPAN)
}

/// `StorageDead(local)`
#[must_use]
pub fn storage_dead(local: Local) -> Statement {
    Statement::StorageDead(local, DUMMY_SPAN)
}

/// `dest = &form source` (whole-local borrow).
#[must_use]
pub fn borrow(dest: Local, form: ReferenceForm, source: Local) -> Statement {
    Statement::Borrow {
        dest: dest.into(),
        form,
        source: source.into(),
        span: DUMMY_SPAN,
    }
}

/// `dest = &form source` where `source` is an arbitrary projected place
/// (e.g. `obj.x`).
#[must_use]
pub fn borrow_place(dest: Local, form: ReferenceForm, source: Place) -> Statement {
    Statement::Borrow {
        dest: dest.into(),
        form,
        source,
        span: DUMMY_SPAN,
    }
}

/// `dest = move source`
#[must_use]
pub fn assign(dest: Local, source: Local) -> Statement {
    Statement::Assign {
        dest: dest.into(),
        source: source.into(),
        span: DUMMY_SPAN,
    }
}

/// `dest = const value`
#[must_use]
pub fn const_int(dest: Local, value: i128) -> Statement {
    Statement::Const {
        dest: dest.into(),
        value: ConstValue::Integer(value),
        span: DUMMY_SPAN,
    }
}

/// `dest = left op right`
#[must_use]
pub fn binop(dest: Local, op: BinOp, left: Local, right: Local) -> Statement {
    Statement::BinaryOp {
        dest: dest.into(),
        op,
        left: left.into(),
        right: right.into(),
        span: DUMMY_SPAN,
    }
}

/// Build a `CallDispatch` terminator.
///
/// Function calls MUST be terminators, not statements, because a call
/// can unwind (panic/error) — which means control flow may diverge at
/// the call site. A basic block containing a call would not be a true
/// basic block.
#[must_use]
pub fn call_dispatch(
    callee: FunctionId,
    callee_name: &str,
    args: Vec<Local>,
    return_bb: BasicBlock,
    dest: Vec<Local>,
) -> Terminator {
    Terminator::CallDispatch {
        callee,
        callee_name: callee_name.to_string(),
        target: CallTarget::Jit,
        args,
        return_bb,
        dest,
        return_shape: ReturnShape::Scalar,
        span: DUMMY_SPAN,
    }
}

/// `Return(values)` terminator.
#[must_use]
pub fn return_(values: Vec<Local>) -> Terminator {
    Terminator::Return {
        values,
        span: DUMMY_SPAN,
    }
}

/// `Goto(target)` terminator.
#[must_use]
pub fn goto(target: BasicBlock) -> Terminator {
    Terminator::Goto {
        target,
        span: DUMMY_SPAN,
    }
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build MIR for `write_twice` with proper block-per-call structure.
    ///
    /// Each function call is a `CallDispatch` terminator — the basic block
    /// ENDS at the call and control resumes in the return block. This is
    /// the only correct way to represent calls in a CFG: calls can unwind
    /// (panic), so they MUST split the control flow.
    ///
    /// ```triet
    /// function write_twice(vga: &+ mutable VgaBuffer) -> Unit {
    ///     let b1: &0 mutable VgaBuffer = &0 mutable vga;
    ///     write_cell(b1, 0, 0, 'H', 0x0F);
    ///     let b2: &0 mutable VgaBuffer = &0 mutable vga;
    ///     write_cell(b2, 0, 1, 'i', 0x0F);
    ///     consume(vga);
    /// }
    /// ```
    #[test]
    fn mir_cfg_write_twice_multi_block() {
        let mut b = MirBuilder::new("write_twice", "Unit");

        let vga = b.add_param("vga", ParameterPassing::Move);
        let write_cell_id = b.new_func_id();
        let consume_id = b.new_func_id();

        let b1 = b.new_local();
        let b2 = b.new_local();
        let cell_h = b.new_local();
        let attr = b.new_local();
        let zero = b.new_local();
        let cell_i = b.new_local();
        let attr2 = b.new_local();
        let zero2 = b.new_local();
        let one = b.new_local();

        // ── bb0: set up borrow b1 + constants, call write_cell(b1) ──
        let bb0 = b.new_block();
        b.push(bb0, storage_live(b1));
        b.push(bb0, borrow(b1, ReferenceForm::BorrowExclusiveMutable, vga));
        b.push(bb0, storage_live(cell_h));
        b.push(bb0, const_int(cell_h, 72)); // 'H'
        b.push(bb0, storage_live(attr));
        b.push(bb0, const_int(attr, 15)); // 0x0F
        b.push(bb0, storage_live(zero));
        b.push(bb0, const_int(zero, 0));

        // ── bb1: return from write_cell(b1), clean up temps, set up b2 ──
        let bb1 = b.new_block();
        b.push(bb1, storage_dead(cell_h));
        b.push(bb1, storage_dead(attr));
        b.push(bb1, storage_dead(zero));
        // b1 last use was the call in bb0 → NLL: loan CAN end here
        b.push(bb1, storage_live(b2));
        b.push(bb1, borrow(b2, ReferenceForm::BorrowExclusiveMutable, vga));
        b.push(bb1, storage_live(cell_i));
        b.push(bb1, const_int(cell_i, 105)); // 'i'
        b.push(bb1, storage_live(attr2));
        b.push(bb1, const_int(attr2, 15));
        b.push(bb1, storage_live(zero2));
        b.push(bb1, const_int(zero2, 0));
        b.push(bb1, storage_live(one));
        b.push(bb1, const_int(one, 1));

        // ── bb2: return from write_cell(b2), clean up temps, call consume ──
        let bb2 = b.new_block();
        b.push(bb2, storage_dead(cell_i));
        b.push(bb2, storage_dead(attr2));
        b.push(bb2, storage_dead(zero2));
        b.push(bb2, storage_dead(one));
        // b2 last use was the call in bb1 → NLL: loan CAN end here

        // ── bb3: return from consume, final cleanup + return ──
        let bb3 = b.new_block();
        b.push(bb3, storage_dead(b1)); // lexical scope end
        b.push(bb3, storage_dead(b2)); // lexical scope end

        // ── Wire up terminators ──
        b.set_terminator(
            bb0,
            call_dispatch(
                write_cell_id,
                "write_cell",
                vec![b1, zero, zero, cell_h, attr],
                bb1,
                vec![],
            ),
        );
        b.set_terminator(
            bb1,
            call_dispatch(
                write_cell_id,
                "write_cell",
                vec![b2, zero2, one, cell_i, attr2],
                bb2,
                vec![],
            ),
        );
        b.set_terminator(
            bb2,
            call_dispatch(consume_id, "consume", vec![vga], bb3, vec![]),
        );
        b.set_terminator(bb3, return_(vec![]));

        let body = b.build(bb0);
        println!("=== MIR (write_twice, multi-block) ===\n{body}");

        let cfg = body.build_cfg();
        println!("=== CFG ===\n{cfg}");

        // 4 blocks: bb0 → bb1 → bb2 → bb3
        assert_eq!(cfg.blocks.len(), 4);
        assert_eq!(cfg.entry, bb0);
        assert_eq!(cfg.exits.len(), 1);
        assert_eq!(cfg.exits[0], bb3);

        // Verify predecessor/successor chain
        assert_eq!(cfg.blocks[bb0.0].predecessors, vec![]);
        assert_eq!(cfg.blocks[bb0.0].successors, vec![bb1]);
        assert_eq!(cfg.blocks[bb1.0].predecessors, vec![bb0]);
        assert_eq!(cfg.blocks[bb1.0].successors, vec![bb2]);
        assert_eq!(cfg.blocks[bb2.0].predecessors, vec![bb1]);
        assert_eq!(cfg.blocks[bb2.0].successors, vec![bb3]);
        assert_eq!(cfg.blocks[bb3.0].predecessors, vec![bb2]);
        assert_eq!(cfg.blocks[bb3.0].successors, vec![]);

        // No calls in statements — all calls are terminators
        for block in &cfg.blocks {
            for stmt in &block.data.statements {
                // Verify no statement is a call (calls must be terminators)
                // This is checked by construction — Statement::Call no longer exists
                let _ = stmt;
            }
        }
    }

    /// abs_diff with proper terminator-based CFG.
    #[test]
    fn mir_cfg_abs_diff_branching() {
        let mut b = MirBuilder::new("abs_diff", "Integer");

        let a = b.add_param("a", ParameterPassing::Borrow);
        let b_param = b.add_param("b", ParameterPassing::Borrow);
        let cond = b.new_local();
        let tmp1 = b.new_local();
        let tmp2 = b.new_local();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(cond));
        b.push(bb0, binop(cond, BinOp::Gt, a, b_param));

        let bb1 = b.new_block();
        b.push(bb1, storage_live(tmp1));
        b.push(bb1, binop(tmp1, BinOp::Sub, a, b_param));
        b.set_terminator(bb1, return_(vec![tmp1]));

        let bb2 = b.new_block();
        b.push(bb2, storage_live(tmp2));
        b.push(bb2, binop(tmp2, BinOp::Sub, b_param, a));
        b.set_terminator(bb2, return_(vec![tmp2]));

        b.set_terminator(
            bb0,
            Terminator::If {
                cond,
                positive_bb: bb1,
                zero_bb: None,
                negative_bb: bb2,
                span: DUMMY_SPAN,
            },
        );

        let body = b.build(bb0);
        let cfg = body.build_cfg();

        // bb0: 0 preds, 2 succs; bb1: 1 pred, 0 succs; bb2: 1 pred, 0 succs
        assert_eq!(cfg.blocks[bb0.0].successors.len(), 2);
        assert!(cfg.blocks[bb0.0].predecessors.is_empty());
        assert!(cfg.blocks[bb1.0].successors.is_empty());
        assert_eq!(cfg.blocks[bb1.0].predecessors, vec![bb0]);
        assert!(cfg.blocks[bb2.0].successors.is_empty());
        assert_eq!(cfg.blocks[bb2.0].predecessors, vec![bb0]);
        assert_eq!(cfg.exits.len(), 2);
    }
}

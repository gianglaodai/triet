//! Liveness analysis — backward dataflow on the CFG.
//!
//! Computes, for each basic block, which `Local` variables are live
//! at block entry (`live_in`) and block exit (`live_out`).
//!
//! A variable is **live** at a program point if there exists a path
//! from that point to a use of the variable. This is the key to NLL:
//! a borrow's loan ends not at `StorageDead`, but at the last use of
//! the borrowed reference — and liveness tells us where that last use is.
//!
//! ## Algorithm
//!
//! Standard backward dataflow with fixed-point iteration:
//!
//! ```text
//! for each block B:
//!     live_in[B]  = use[B]   // variables read before written in B
//!     live_out[B] = ∅
//!
//! repeat until stable:
//!     for each block B:
//!         live_out[B] = ⋃ { live_in[S] | S ∈ successors(B) }
//!         live_in[B]  = use[B] ∪ (live_out[B] − def[B])
//! ```
//!
//! - `use[B]`: variables read in B before being written
//! - `def[B]`: variables written in B before being read

use std::collections::BTreeSet;
use triet_mir::{BasicBlock, ControlFlowGraph, Local, Statement, Terminator};

/// Live variable sets for a single basic block.
#[derive(Clone, Debug, Default)]
pub struct BlockLiveness {
    /// Variables live at block entry.
    pub live_in: BTreeSet<Local>,
    /// Variables live at block exit.
    pub live_out: BTreeSet<Local>,
    /// Variables used (read) in this block before being defined.
    pub used: BTreeSet<Local>,
    /// Variables defined (written) in this block before being used.
    pub defined: BTreeSet<Local>,
    /// The program points where each variable is last used in this block.
    /// Index is statement position within the block (0-based).
    pub last_uses: Vec<(Local, usize)>,
}

/// Liveness analysis result for an entire function.
#[derive(Clone, Debug)]
pub struct LivenessResult {
    /// Per-block liveness data.
    pub blocks: Vec<BlockLiveness>,
}

impl LivenessResult {
    /// Compute liveness for a CFG using backward dataflow.
    #[must_use]
    pub fn compute(cfg: &ControlFlowGraph) -> Self {
        let mut blocks: Vec<BlockLiveness> = cfg.blocks.iter().map(compute_block_use_def).collect();

        // Fixed-point iteration
        loop {
            let mut changed = false;

            for (i, block) in cfg.blocks.iter().enumerate() {
                // live_out[B] = union of live_in of all successors
                let mut new_live_out: BTreeSet<Local> = BTreeSet::new();
                for &succ in &block.successors {
                    if succ.0 < blocks.len() {
                        new_live_out.extend(&blocks[succ.0].live_in);
                    }
                }

                if new_live_out != blocks[i].live_out {
                    blocks[i].live_out = new_live_out;
                    changed = true;
                }

                // live_in[B] = use[B] ∪ (live_out[B] − def[B])
                let mut new_live_in = blocks[i].used.clone();
                for l in blocks[i].live_out.difference(&blocks[i].defined) {
                    new_live_in.insert(*l);
                }

                if new_live_in != blocks[i].live_in {
                    blocks[i].live_in = new_live_in;
                    changed = true;
                }
            }

            if !changed {
                break;
            }
        }

        Self { blocks }
    }

    /// Check if a local is live at a specific statement index within a block.
    ///
    /// A local is "live after" a statement if it will be used later in this
    /// block OR is live-out of the block and the block has no subsequent
    /// definition of it.
    #[must_use]
    pub fn is_live_after(&self, block: BasicBlock, stmt_index: usize, local: Local) -> bool {
        if block.0 >= self.blocks.len() {
            return false;
        }
        let bl = &self.blocks[block.0];

        // Check if local is used in any later statement in this block
        for (l, pos) in &bl.last_uses {
            if *l == local && *pos > stmt_index {
                return true;
            }
        }

        // Check if local is live-out (used in a successor block)
        bl.live_out.contains(&local)
    }
}

/// Compute `use` and `def` sets for a single basic block.
///
/// `use[B]`: variables that are READ before being WRITTEN in B.
/// `def[B]`: variables that are WRITTEN before being READ in B.
///
/// Also records the last-use position for each variable (for NLL).
fn compute_block_use_def(block: &triet_mir::CfgBlock) -> BlockLiveness {
    let mut used: BTreeSet<Local> = BTreeSet::new();
    let mut defined: BTreeSet<Local> = BTreeSet::new();
    let mut last_uses: Vec<(Local, usize)> = Vec::new();

    for (i, stmt) in block.data.statements.iter().enumerate() {
        // Collect locals READ by this statement
        let reads = statement_reads(stmt);

        // Collect locals WRITTEN by this statement
        let writes = statement_writes(stmt);

        // Track uses: a variable is "used" in this block if it's read
        // before being defined
        for r in &reads {
            if !defined.contains(r) {
                used.insert(*r);
            }
            last_uses.push((*r, i));
        }

        // Track defs: a variable is "defined" in this block if it's
        // written before being read
        for w in &writes {
            if !used.contains(w) {
                defined.insert(*w);
            }
        }
    }

    // Also check terminator for reads
    let term_reads = terminator_reads(&block.data.terminator);
    for r in &term_reads {
        if !defined.contains(r) {
            used.insert(*r);
        }
        last_uses.push((*r, block.data.statements.len())); // at terminator position
    }

    BlockLiveness {
        live_in: BTreeSet::new(),
        live_out: BTreeSet::new(),
        used,
        defined,
        last_uses,
    }
}

/// Return the locals READ by a statement.
fn statement_reads(stmt: &Statement) -> Vec<Local> {
    // Liveness is tracked at whole-local granularity, so a projected place's
    // base local is what counts as read.
    match stmt {
        Statement::StorageLive(_, _) | Statement::StorageDead(_, _) => Vec::new(),
        Statement::Assign { source, .. } => vec![source.local],
        Statement::Borrow { source, .. } => vec![source.local],
        Statement::Const { .. } => Vec::new(),
        Statement::BinaryOp { left, right, .. } => vec![left.local, right.local],
        Statement::OutcomeDiscriminant { source, .. } => vec![source.local],
        Statement::OutcomeUnwrap { source, .. } => vec![source.local],
        Statement::OutcomeUnwrapError { source, .. } => vec![source.local],
        Statement::Drop(l, _) => vec![*l],
    }
}

/// Return the locals WRITTEN by a statement.
fn statement_writes(stmt: &Statement) -> Vec<Local> {
    match stmt {
        Statement::StorageLive(_, _) | Statement::StorageDead(_, _) => Vec::new(),
        Statement::Assign { dest, .. }
        | Statement::Borrow { dest, .. }
        | Statement::Const { dest, .. }
        | Statement::BinaryOp { dest, .. }
        | Statement::OutcomeDiscriminant { dest, .. }
        | Statement::OutcomeUnwrap { dest, .. }
        | Statement::OutcomeUnwrapError { dest, .. } => vec![dest.local],
        Statement::Drop(_, _) => Vec::new(),
    }
}

/// Return the locals READ by a terminator.
fn terminator_reads(term: &Terminator) -> Vec<Local> {
    match term {
        Terminator::Return { values, .. } => values.clone(),
        Terminator::Goto { .. } => Vec::new(),
        Terminator::If { cond, .. } => vec![*cond],
        Terminator::CallDispatch { args, .. } => args.clone(),
        Terminator::Unreachable { .. } => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MirBuilder, const_int, storage_live};
    use triet_mir::BinOp;

    /// A simple block: `x = 1; y = x + 2; return y`
    /// Live: x is live from its def until the add; y is live from its def until return
    #[test]
    fn liveness_simple_block() {
        let mut b = MirBuilder::new("test", "Integer");
        let x = b.new_local();
        let y = b.new_local();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(x));
        b.push(bb0, const_int(x, 1));
        b.push(bb0, storage_live(y));
        let two = b.new_local();
        b.push(bb0, crate::storage_live(two));
        b.push(bb0, crate::const_int(two, 2));
        b.push(bb0, crate::binop(y, BinOp::Add, x, two));
        b.set_terminator(bb0, crate::return_(vec![y]));

        let body = b.build(bb0);
        let cfg = body.build_cfg();
        let liveness = LivenessResult::compute(&cfg);

        // x is WRITTEN by const_int before being read by binop
        // → x is in `defined`, not `used`
        assert!(
            liveness.blocks[bb0.0].defined.contains(&x),
            "x is defined (written) first by const_int"
        );
        // x IS then read by binop — so it appears in last_uses
        let x_has_last_use = liveness.blocks[bb0.0]
            .last_uses
            .iter()
            .any(|(l, _)| *l == x);
        assert!(x_has_last_use, "x is read in binop → has a last_use entry");
        // y is defined in binop
        assert!(
            liveness.blocks[bb0.0].defined.contains(&y),
            "y is defined by binop"
        );
        // y is read in return (via terminator) — but since y was written first
        // by the binop, it's in `defined`, not `used`. It DOES have a last_use entry.
        let y_has_last_use = liveness.blocks[bb0.0]
            .last_uses
            .iter()
            .any(|(l, _)| *l == y);
        assert!(y_has_last_use, "y is read in return → has a last_use entry");

        // After the const (index 1), x should still be live (used later in binop at index 5)
        assert!(
            liveness.is_live_after(bb0, 1, x),
            "x is still alive after const_int because it's used in binop at index 5"
        );

        // After the binop (index 5), x should NOT be live (no more uses)
        assert!(
            !liveness.is_live_after(bb0, 5, x),
            "x is dead after the binop — last use was at index 5"
        );
    }
}

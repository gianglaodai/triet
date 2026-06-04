//! Borrow checker — conflict detection using liveness + loan tracking.
//!
//! Implements **forward dataflow analysis** over the CFG, propagating
//! variable states (Owned/Moved) and active loans across basic block
//! boundaries via fixed-point iteration — the same technique used for
//! liveness, but in the forward direction.
//!
//! ## Why forward dataflow is required
//!
//! Without cross-block propagation, a variable moved in `bb0` could be
//! used again in `bb1` because the checker naively resets state at each
//! block boundary. Forward dataflow ensures:
//!
//! - Variables moved in a predecessor stay moved at the successor's entry.
//! - Loans still live at a predecessor's exit stay live at the successor's entry.
//! - Merge at confluence points (if/else join) uses conservative rules.
//!
//! ## Merge rules (conservative — soundness over precision)
//!
//! - **VarState:** `Moved` if moved on ANY incoming path (avoids missing
//!   use-after-move errors across branches).
//! - **Active loans:** union of all incoming loans (avoids missing
//!   conflicting borrows).
//!
//! ## Error codes
//!
//! - **E2420 UseAfterMove**: using a variable whose ownership was transferred.
//! - **E2440 NllExclusivityViolation**: creating a conflicting borrow while
//!   another borrow is still active on the same variable.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use triet_mir::{BasicBlock, Local, Place, Projection, ReferenceForm, Span, Statement, Terminator};

use miette::Diagnostic;
use thiserror::Error;

use crate::liveness::LivenessResult;

// ── Place conflict ──────────────────────────────────────────

/// Whether two places may refer to overlapping memory.
///
/// Different base locals never alias (no pointer-aliasing analysis yet).
/// With the same base, projections are compared step-by-step: two distinct
/// **fields** (`obj.x` vs `obj.y`) are provably disjoint, so they do NOT
/// conflict — this is what enables field-level NLL. Anything we cannot prove
/// disjoint (a prefix relationship, an `Index`, or mismatched projection
/// kinds) is treated as overlapping (refuse over guess).
fn places_conflict(a: &Place, b: &Place) -> bool {
    if a.local != b.local {
        return false;
    }
    for (pa, pb) in a.projection.iter().zip(b.projection.iter()) {
        match (pa, pb) {
            (Projection::Field(x), Projection::Field(y)) => {
                if x != y {
                    // Distinct fields of the same base → disjoint.
                    return false;
                }
            }
            // Same field, both deref, or index (can't prove distinct) — keep
            // walking the common prefix.
            (Projection::Index(_), Projection::Index(_))
            | (Projection::Deref, Projection::Deref) => {}
            // Mismatched projection kinds at the same depth — be conservative.
            _ => return true,
        }
    }
    // One projection chain is a prefix of the other (or they are identical):
    // borrowing the whole and borrowing a part overlap.
    true
}

// ── Loan types ──────────────────────────────────────────────

/// An active loan on a place.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Loan {
    /// The place being borrowed (base local + field/deref projections).
    source: Place,
    /// The reference variable created by the borrow.
    dest: Local,
    /// Which reference form was used.
    form: ReferenceForm,
    /// Block where the borrow was created.
    issued_in: BasicBlock,
    /// Statement index within `issued_in` where the borrow was created.
    issued_at: usize,
}

impl Loan {
    /// Returns true if this loan conflicts with creating a new loan of `form`.
    fn conflicts_with(&self, new_form: ReferenceForm) -> bool {
        match new_form {
            ReferenceForm::BorrowReadOnly => {
                matches!(self.form, ReferenceForm::BorrowExclusiveMutable)
            }
            ReferenceForm::BorrowExclusiveMutable => true,
            ReferenceForm::WeakObserver => {
                matches!(self.form, ReferenceForm::BorrowExclusiveMutable)
            }
            ReferenceForm::StrongFrozen | ReferenceForm::StrongMutable => false,
        }
    }
}

// ── Variable state ──────────────────────────────────────────

/// Tracked state of a local variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VarState {
    /// Variable is owned and can be used.
    Owned,
    /// Variable was moved — any use is E2420.
    Moved,
}

// ── Block state (entry/exit) ────────────────────────────────

/// The borrow-checker state at a basic block boundary.
#[derive(Clone, Debug, Default)]
struct BlockState {
    /// Variable states (Owned or Moved).
    var_states: BTreeMap<Local, VarState>,
    /// Loans that are still active.
    active_loans: BTreeSet<Loan>,
}

impl BlockState {
    /// Create initial state with parameters marked Owned.
    fn initial(num_params: usize) -> Self {
        let mut var_states = BTreeMap::new();
        for i in 0..num_params {
            var_states.insert(Local(i), VarState::Owned);
        }
        Self {
            var_states,
            active_loans: BTreeSet::new(),
        }
    }

    /// Merge multiple predecessor states into one entry state.
    ///
    /// Conservative merge: if ANY predecessor has a variable Moved or a
    /// loan active, the merged state reflects that. This guarantees we
    /// never miss an error at the cost of possible false positives at
    /// confluence points (acceptable for safety).
    fn merge(predecessors: &[BlockState]) -> Self {
        if predecessors.is_empty() {
            return Self::default();
        }
        if predecessors.len() == 1 {
            return predecessors[0].clone();
        }

        let mut merged = Self::default();

        // var_states: Moved if Moved on ANY path
        let all_locals: BTreeSet<Local> = predecessors
            .iter()
            .flat_map(|s| s.var_states.keys().copied())
            .collect();
        for local in all_locals {
            let any_moved = predecessors
                .iter()
                .any(|s| s.var_states.get(&local) == Some(&VarState::Moved));
            if any_moved {
                merged.var_states.insert(local, VarState::Moved);
            } else {
                // Owned on all paths (or not present = assume Owned)
                merged.var_states.insert(local, VarState::Owned);
            }
        }

        // active_loans: union of all incoming loans
        for pred in predecessors {
            merged
                .active_loans
                .extend(pred.active_loans.iter().cloned());
        }

        merged
    }
}

// ── Borrow check result ─────────────────────────────────────

/// A borrow-check error with source-level diagnostics.
#[derive(Clone, Debug, Error, Diagnostic, PartialEq, Eq)]
pub enum BorrowError {
    /// E2420: Use after move — a variable was used after its ownership was transferred.
    #[error("E2420: use after move — `{name}` was used after its ownership was transferred")]
    #[diagnostic(
        code(triet::borrow::E2420),
        help("bind the moved value to a new variable before use, or borrow it instead of moving")
    )]
    UseAfterMove {
        /// The variable that was used after being moved.
        local: Local,
        /// Human-readable variable name.
        name: String,
        /// Source location of the use.
        #[label("used here after move")]
        span: Span,
    },

    /// E2440: NLL exclusivity violation — conflicting borrow.
    #[error(
        "E2440: cannot create {new_form} borrow on `{source_name}` — it is already exclusively borrowed"
    )]
    #[diagnostic(
        code(triet::borrow::E2440),
        help("end the earlier borrow before creating a new one, or use a read-only borrow (&0)")
    )]
    NllExclusivityViolation {
        /// The variable being borrowed.
        source_local: Local,
        /// Human-readable source name.
        source_name: String,
        /// The new borrow that conflicts.
        new_form: ReferenceForm,
        /// The existing loan that causes the conflict.
        existing_loan_dest: Local,
        /// Source location of the conflicting borrow.
        #[label("conflicting borrow created here")]
        span: Span,
    },
}

/// Result of borrow-checking a function.
#[derive(Clone, Debug)]
pub struct BorrowCheckResult {
    /// Errors found during checking.
    pub errors: Vec<BorrowError>,
}

impl BorrowCheckResult {
    /// Returns true if no errors were found.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

// ── Local name resolution ───────────────────────────────────

/// Maps locals to human-readable names for diagnostics.
type LocalNames = BTreeMap<Local, String>;

fn build_local_names(body: &triet_mir::Body) -> LocalNames {
    let mut names = LocalNames::new();
    for (i, (name, _)) in body.signature.params.iter().enumerate() {
        names.insert(Local(i), name.clone());
    }
    names
}

/// Render a place using variable names where known (e.g. `obj.x`).
fn place_name(place: &Place, names: &LocalNames) -> String {
    let mut s = names
        .get(&place.local)
        .cloned()
        .unwrap_or_else(|| place.local.to_string());
    for proj in &place.projection {
        s = match proj {
            Projection::Deref => format!("(*{s})"),
            Projection::Field(f) => format!("{s}.{f}"),
            Projection::Index(i) => format!("{s}[{i}]"),
        };
    }
    s
}

// ── Core checker — forward dataflow ─────────────────────────

/// Check a MIR body for borrow-check violations using forward dataflow
/// analysis over the CFG.
///
/// Propagates `var_states` and `active_loans` across basic block
/// boundaries. Uses fixed-point iteration: blocks are re-computed when
/// a predecessor's exit state changes.
#[must_use]
pub fn check_body(body: &triet_mir::Body) -> BorrowCheckResult {
    check_body_with(body, &BTreeMap::new())
}

/// Borrow-check a body with access to other functions' signatures, enabling
/// cross-call loan propagation: when a callee returns a reference borrowed
/// from a parameter, the loan is re-issued at the call site tied to the
/// return temporary's lifetime (`PropagatedLoan`).
#[must_use]
pub fn check_body_with(
    body: &triet_mir::Body,
    callee_sigs: &BTreeMap<String, triet_mir::FunctionSignature>,
) -> BorrowCheckResult {
    let cfg = body.build_cfg();
    let liveness = LivenessResult::compute(&cfg);
    let names = build_local_names(body);
    let mut errors = Vec::new();

    let num_blocks = cfg.blocks.len();

    // Per-block entry + exit states
    let mut entry_states: Vec<BlockState> = vec![BlockState::default(); num_blocks];
    let mut exit_states: Vec<BlockState> = vec![BlockState::default(); num_blocks];

    // Initialize entry block with parameters = Owned
    entry_states[cfg.entry.0] = BlockState::initial(body.signature.params.len());

    // Worklist: blocks whose successors need re-computation
    let mut worklist: VecDeque<BasicBlock> = VecDeque::new();
    let mut in_worklist: BTreeSet<BasicBlock> = BTreeSet::new();

    // Start with entry block's successors
    for &succ in &cfg.blocks[cfg.entry.0].successors {
        if !in_worklist.contains(&succ) {
            worklist.push_back(succ);
            in_worklist.insert(succ);
        }
    }

    // Process entry block first
    let (entry_exit, entry_errs) = process_block(
        cfg.entry,
        &entry_states[cfg.entry.0],
        &cfg,
        &liveness,
        &names,
        callee_sigs,
    );
    exit_states[cfg.entry.0] = entry_exit;
    errors.extend(entry_errs);

    // Fixed-point iteration
    while let Some(block) = worklist.pop_front() {
        in_worklist.remove(&block);

        // Compute entry state by merging predecessor exit states
        let preds: Vec<BlockState> = cfg.blocks[block.0]
            .predecessors
            .iter()
            .map(|&p| exit_states[p.0].clone())
            .collect();
        let new_entry = BlockState::merge(&preds);

        if new_entry.var_states == entry_states[block.0].var_states
            && new_entry.active_loans == entry_states[block.0].active_loans
        {
            // State unchanged — no need to re-process
            continue;
        }

        entry_states[block.0] = new_entry;

        // Process block with new entry state
        let (new_exit, block_errs) = process_block(
            block,
            &entry_states[block.0],
            &cfg,
            &liveness,
            &names,
            callee_sigs,
        );
        errors.extend(block_errs);

        if new_exit.var_states != exit_states[block.0].var_states
            || new_exit.active_loans != exit_states[block.0].active_loans
        {
            exit_states[block.0] = new_exit;
            // Propagate to successors
            for &succ in &cfg.blocks[block.0].successors {
                if !in_worklist.contains(&succ) {
                    worklist.push_back(succ);
                    in_worklist.insert(succ);
                }
            }
        }
    }

    BorrowCheckResult { errors }
}

/// Process a single block: walk statements and terminator, checking for
/// errors and updating state.
///
/// Returns (exit_state, errors_found).
fn process_block(
    block: BasicBlock,
    entry_state: &BlockState,
    cfg: &triet_mir::ControlFlowGraph,
    liveness: &LivenessResult,
    names: &LocalNames,
    callee_sigs: &BTreeMap<String, triet_mir::FunctionSignature>,
) -> (BlockState, Vec<BorrowError>) {
    let mut state = entry_state.clone();
    let mut errors = Vec::new();
    let block_data = &cfg.blocks[block.0].data;

    for (stmt_idx, stmt) in block_data.statements.iter().enumerate() {
        match stmt {
            Statement::StorageLive(l, _) => {
                state.var_states.insert(*l, VarState::Owned);
            }

            Statement::StorageDead(l, _) => {
                state.active_loans.retain(|loan| loan.source.local != *l);
                state.var_states.remove(l);
            }

            Statement::Borrow {
                dest,
                form,
                source,
                span,
            } => {
                // Check for conflicts with active loans (field-level: only
                // overlapping places conflict — `obj.x` vs `obj.y` do not).
                for loan in &state.active_loans {
                    if places_conflict(&loan.source, source) && loan.conflicts_with(*form) {
                        errors.push(BorrowError::NllExclusivityViolation {
                            source_local: source.local,
                            source_name: place_name(source, names),
                            new_form: *form,
                            existing_loan_dest: loan.dest,
                            span: span.clone(),
                        });
                    }
                }

                // Check that the borrowed base hasn't been moved.
                if state.var_states.get(&source.local) == Some(&VarState::Moved) {
                    errors.push(BorrowError::UseAfterMove {
                        local: source.local,
                        name: place_name(source, names),
                        span: span.clone(),
                    });
                }

                match form {
                    ReferenceForm::StrongFrozen | ReferenceForm::StrongMutable => {
                        state.var_states.insert(source.local, VarState::Moved);
                    }
                    _ => {
                        state.active_loans.insert(Loan {
                            source: source.clone(),
                            dest: dest.local,
                            form: *form,
                            issued_in: block,
                            issued_at: stmt_idx,
                        });
                    }
                }

                state.var_states.insert(dest.local, VarState::Owned);
            }

            Statement::Assign { dest, source, span } => {
                if state.var_states.get(&source.local) == Some(&VarState::Moved) {
                    errors.push(BorrowError::UseAfterMove {
                        local: source.local,
                        name: place_name(source, names),
                        span: span.clone(),
                    });
                }

                let conflicting = state
                    .active_loans
                    .iter()
                    .find(|l| places_conflict(&l.source, source));
                if let Some(loan) = conflicting {
                    errors.push(BorrowError::NllExclusivityViolation {
                        source_local: source.local,
                        source_name: place_name(source, names),
                        new_form: ReferenceForm::StrongMutable,
                        existing_loan_dest: loan.dest,
                        span: span.clone(),
                    });
                }
                state.var_states.insert(source.local, VarState::Moved);
                state.var_states.insert(dest.local, VarState::Owned);
            }

            Statement::Const { dest, .. } => {
                state.var_states.insert(dest.local, VarState::Owned);
            }

            Statement::BinaryOp {
                dest,
                left,
                right,
                span,
                ..
            } => {
                for op in [left, right] {
                    if state.var_states.get(&op.local) == Some(&VarState::Moved) {
                        errors.push(BorrowError::UseAfterMove {
                            local: op.local,
                            name: place_name(op, names),
                            span: span.clone(),
                        });
                    }
                }
                state.var_states.insert(dest.local, VarState::Owned);
            }

            Statement::OutcomeDiscriminant { dest, source, span }
            | Statement::OutcomeUnwrap { dest, source, span }
            | Statement::OutcomeUnwrapError { dest, source, span } => {
                if state.var_states.get(&source.local) == Some(&VarState::Moved) {
                    errors.push(BorrowError::UseAfterMove {
                        local: source.local,
                        name: place_name(source, names),
                        span: span.clone(),
                    });
                }
                state.var_states.insert(dest.local, VarState::Owned);
            }

            Statement::Drop(l, span) => {
                if state.var_states.get(l) == Some(&VarState::Moved) {
                    let l_name = names.get(l).cloned().unwrap_or_else(|| format!("{l}"));
                    errors.push(BorrowError::UseAfterMove {
                        local: *l,
                        name: l_name,
                        span: span.clone(),
                    });
                }
                state.var_states.insert(*l, VarState::Moved);
                state.active_loans.retain(|loan| loan.source.local != *l);
            }
        }

        // NLL: end loans whose dest is no longer live after this statement
        let block_id = block;
        let idx = stmt_idx;
        state
            .active_loans
            .retain(|loan| liveness.is_live_after(block_id, idx, loan.dest));
    }

    // Check terminator for use-after-move — use the terminator's own span
    // so diagnostics point to the actual source code the user wrote.
    let term_span = terminator_span(&block_data.terminator);
    let term_reads = terminator_reads(&block_data.terminator);
    for r in &term_reads {
        if state.var_states.get(r) == Some(&VarState::Moved) {
            let r_name = names.get(r).cloned().unwrap_or_else(|| format!("{r}"));
            errors.push(BorrowError::UseAfterMove {
                local: *r,
                name: r_name,
                span: term_span.clone(),
            });
        }
    }

    // Cross-call loan propagation (PropagatedLoan): if the callee returns a
    // reference borrowed from one of its parameters, re-issue that loan at the
    // call site — its source is the place the argument borrows, its dest is the
    // call's return temporary. The borrowed value therefore stays frozen for as
    // long as the returned reference lives (liveness of the return temp).
    if let Terminator::CallDispatch {
        callee_name,
        args,
        dest,
        ..
    } = &block_data.terminator
        && let Some(sig) = callee_sigs.get(callee_name)
    {
        let term_idx = block_data.statements.len();
        // NOTE: field paths in `return_borrow_map` distinguish which RETURN
        // field borrows which param. Multi-value struct returns (mapping each
        // field to a distinct return temp) are deferred — for single-value
        // returns the borrow lands in `dest[0]`.
        for param_indices in sig.return_borrow_map.values() {
            let Some(&ret_temp) = dest.first() else {
                continue;
            };
            for &pi in param_indices {
                let Some(&arg_local) = args.get(pi) else {
                    continue;
                };
                if let Some(orig) = state
                    .active_loans
                    .iter()
                    .find(|l| l.dest == arg_local)
                    .cloned()
                {
                    state.active_loans.insert(Loan {
                        source: orig.source.clone(),
                        dest: ret_temp,
                        form: orig.form,
                        issued_in: block,
                        issued_at: term_idx,
                    });
                }
            }
        }
    }

    (state, errors)
}

/// Return the source span of a terminator.
fn terminator_span(term: &Terminator) -> Span {
    match term {
        Terminator::Return { span, .. }
        | Terminator::Goto { span, .. }
        | Terminator::If { span, .. }
        | Terminator::CallDispatch { span, .. }
        | Terminator::Unreachable { span } => span.clone(),
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
    use crate::{
        MirBuilder, borrow, borrow_place, call_dispatch, const_int, return_, storage_dead,
        storage_live,
    };
    use triet_mir::{
        FieldPath, FunctionSignature, ParameterPassing, Place, Projection, ReferenceForm,
    };

    fn field(local: Local, name: &str) -> Place {
        Place {
            local,
            projection: vec![Projection::Field(name.to_string())],
        }
    }

    /// Cross-block use-after-move: move `vga` in `bb0`, then try to use it
    /// in `bb1`. Forward dataflow must propagate the Moved state.
    ///
    /// ```triet
    /// function cross_block_move(vga: &+ mutable VgaBuffer) -> Unit {
    ///     let other = vga;   // move vga in bb0
    ///     // goto bb1
    ///     consume(vga);      // ERROR: vga was moved in bb0
    /// }
    /// ```
    #[test]
    fn use_after_move_across_blocks_rejected() {
        let mut b = MirBuilder::new("cross_block_move", "Unit");
        let vga = b.add_param("vga", ParameterPassing::Move);
        let other = b.new_local();
        let consume_id = b.new_func_id();

        // bb0: move vga → other, then goto bb1
        let bb0 = b.new_block();
        b.push(bb0, storage_live(other));
        b.push(bb0, crate::assign(other, vga)); // move vga → _1

        // bb1: try to use vga — should be E2420
        let bb1 = b.new_block();
        let vga2 = b.new_local();
        b.push(bb1, storage_live(vga2));
        b.push(bb1, crate::assign(vga2, vga)); // vga was moved in bb0

        // bb2: clean up
        let bb2 = b.new_block();
        b.push(bb2, storage_dead(other));
        b.push(bb2, storage_dead(vga2));
        b.set_terminator(bb2, return_(vec![]));

        // Wire up: bb0 → bb1 (via consume call), bb1 → bb2
        b.set_terminator(
            bb0,
            call_dispatch(consume_id, "consume", vec![other], bb1, vec![]),
        );
        b.set_terminator(
            bb1,
            call_dispatch(consume_id, "consume", vec![vga2], bb2, vec![]),
        );

        let body = b.build(bb0);
        println!("=== MIR (cross_block_move) ===\n{body}");

        let result = check_body(&body);
        println!("=== BORROW CHECK (cross_block_move) ===");
        for err in &result.errors {
            println!("  {err}");
        }

        assert!(
            !result.is_ok(),
            "cross-block use-after-move MUST be rejected"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::UseAfterMove { .. })),
            "should have E2420 UseAfterMove across blocks, got: {:?}",
            result.errors
        );
    }

    /// NLL violation: two `&0 mutable` borrows where the first is still alive.
    #[test]
    fn nll_double_exclusive_borrow_rejected() {
        let mut b = MirBuilder::new("double_borrow", "Unit");
        let vga = b.add_param("vga", ParameterPassing::Move);
        let b1 = b.new_local();
        let b2 = b.new_local();
        let write_cell_id = b.new_func_id();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(b1));
        b.push(bb0, borrow(b1, ReferenceForm::BorrowExclusiveMutable, vga));
        b.push(bb0, storage_live(b2));
        b.push(bb0, borrow(b2, ReferenceForm::BorrowExclusiveMutable, vga));

        let cell_h = b.new_local();
        let attr = b.new_local();
        let zero = b.new_local();
        b.push(bb0, storage_live(cell_h));
        b.push(bb0, const_int(cell_h, 72));
        b.push(bb0, storage_live(attr));
        b.push(bb0, const_int(attr, 15));
        b.push(bb0, storage_live(zero));
        b.push(bb0, const_int(zero, 0));

        let bb1 = b.new_block();
        b.push(bb1, storage_dead(cell_h));
        b.push(bb1, storage_dead(attr));
        b.push(bb1, storage_dead(zero));
        b.push(bb1, storage_dead(b1));
        b.push(bb1, storage_dead(b2));
        b.set_terminator(bb1, return_(vec![]));

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

        let body = b.build(bb0);
        let result = check_body(&body);

        println!("=== BORROW CHECK (double borrow) ===");
        for err in &result.errors {
            println!("  {err}");
        }

        assert!(
            !result.is_ok(),
            "double exclusive borrow should be rejected"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::NllExclusivityViolation { .. })),
            "should have E2440"
        );
    }

    /// Correct NLL: sequential `&0 mutable` borrows in different blocks.
    #[test]
    fn nll_sequential_borrow_accepted() {
        let mut b = MirBuilder::new("sequential_borrow", "Unit");
        let vga = b.add_param("vga", ParameterPassing::Move);
        let b1 = b.new_local();
        let b2 = b.new_local();
        let write_cell_id = b.new_func_id();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(b1));
        b.push(bb0, borrow(b1, ReferenceForm::BorrowExclusiveMutable, vga));
        let cell_h = b.new_local();
        let attr = b.new_local();
        let zero = b.new_local();
        b.push(bb0, storage_live(cell_h));
        b.push(bb0, const_int(cell_h, 72));
        b.push(bb0, storage_live(attr));
        b.push(bb0, const_int(attr, 15));
        b.push(bb0, storage_live(zero));
        b.push(bb0, const_int(zero, 0));

        let bb1 = b.new_block();
        b.push(bb1, storage_dead(cell_h));
        b.push(bb1, storage_dead(attr));
        b.push(bb1, storage_dead(zero));
        b.push(bb1, storage_live(b2));
        b.push(bb1, borrow(b2, ReferenceForm::BorrowExclusiveMutable, vga));
        let cell_i = b.new_local();
        let attr2 = b.new_local();
        let zero2 = b.new_local();
        let one = b.new_local();
        b.push(bb1, storage_live(cell_i));
        b.push(bb1, const_int(cell_i, 105));
        b.push(bb1, storage_live(attr2));
        b.push(bb1, const_int(attr2, 15));
        b.push(bb1, storage_live(zero2));
        b.push(bb1, const_int(zero2, 0));
        b.push(bb1, storage_live(one));
        b.push(bb1, const_int(one, 1));

        let bb2 = b.new_block();
        b.push(bb2, storage_dead(cell_i));
        b.push(bb2, storage_dead(attr2));
        b.push(bb2, storage_dead(zero2));
        b.push(bb2, storage_dead(one));
        b.push(bb2, storage_dead(b1));
        b.push(bb2, storage_dead(b2));
        b.set_terminator(bb2, return_(vec![]));

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

        let body = b.build(bb0);
        let result = check_body(&body);

        println!("=== BORROW CHECK (sequential borrow) ===");
        for err in &result.errors {
            println!("  {err}");
        }

        assert!(
            result.is_ok(),
            "sequential borrow should be accepted, got errors: {:?}",
            result.errors
        );
    }

    /// Moving a borrowed variable within the same block.
    #[test]
    fn use_after_move_rejected() {
        let mut b = MirBuilder::new("use_after_move", "Unit");
        let vga = b.add_param("vga", ParameterPassing::Move);
        let b1 = b.new_local();
        let write_cell_id = b.new_func_id();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(b1));
        b.push(bb0, borrow(b1, ReferenceForm::BorrowExclusiveMutable, vga));
        let moved_vga = b.new_local();
        b.push(bb0, crate::assign(moved_vga, vga));

        let cell_h = b.new_local();
        let attr = b.new_local();
        let zero = b.new_local();
        b.push(bb0, storage_live(cell_h));
        b.push(bb0, const_int(cell_h, 72));
        b.push(bb0, storage_live(attr));
        b.push(bb0, const_int(attr, 15));
        b.push(bb0, storage_live(zero));
        b.push(bb0, const_int(zero, 0));

        let bb1 = b.new_block();
        b.push(bb1, storage_dead(cell_h));
        b.push(bb1, storage_dead(attr));
        b.push(bb1, storage_dead(zero));
        b.push(bb1, storage_dead(b1));
        b.set_terminator(bb1, return_(vec![]));

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

        let body = b.build(bb0);
        let result = check_body(&body);

        println!("=== BORROW CHECK (use after move) ===");
        for err in &result.errors {
            println!("  {err}");
        }

        assert!(
            !result.is_ok(),
            "moving a borrowed variable should be rejected"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::NllExclusivityViolation { .. })),
            "should have E2440 for moving a borrowed value"
        );
    }

    /// Field-level NLL: `&0 mutable obj.x` and `&0 mutable obj.y` are disjoint
    /// and may be held simultaneously. Both refs are kept live by passing them
    /// to a call, so both loans are active at the second borrow's creation.
    #[test]
    fn disjoint_field_borrows_accepted() {
        let mut b = MirBuilder::new("split", "Unit");
        let obj = b.add_param("obj", ParameterPassing::MutableBorrow);
        let use_id = b.new_func_id();
        let r_x = b.new_local();
        let r_y = b.new_local();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(r_x));
        b.push(
            bb0,
            borrow_place(r_x, ReferenceForm::BorrowExclusiveMutable, field(obj, "x")),
        );
        b.push(bb0, storage_live(r_y));
        b.push(
            bb0,
            borrow_place(r_y, ReferenceForm::BorrowExclusiveMutable, field(obj, "y")),
        );

        let bb1 = b.new_block();
        b.push(bb1, storage_dead(r_x));
        b.push(bb1, storage_dead(r_y));
        b.set_terminator(bb1, return_(vec![]));

        // Keep both references live at the same point → both loans active.
        b.set_terminator(
            bb0,
            call_dispatch(use_id, "use_both", vec![r_x, r_y], bb1, vec![]),
        );

        let body = b.build(bb0);
        println!("=== MIR (split) ===\n{body}");
        let result = check_body(&body);
        println!("=== BORROW CHECK (split) ===");
        for err in &result.errors {
            println!("  {err}");
        }
        assert!(
            result.is_ok(),
            "disjoint field borrows obj.x / obj.y must be accepted, got: {:?}",
            result.errors
        );
    }

    /// Negative control: two exclusive borrows of the SAME field `obj.x`,
    /// both live, must conflict (E2440). Proves the disjointness logic does
    /// not over-permit.
    #[test]
    fn same_field_borrows_rejected() {
        let mut b = MirBuilder::new("clash", "Unit");
        let obj = b.add_param("obj", ParameterPassing::MutableBorrow);
        let use_id = b.new_func_id();
        let r1 = b.new_local();
        let r2 = b.new_local();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(r1));
        b.push(
            bb0,
            borrow_place(r1, ReferenceForm::BorrowExclusiveMutable, field(obj, "x")),
        );
        b.push(bb0, storage_live(r2));
        b.push(
            bb0,
            borrow_place(r2, ReferenceForm::BorrowExclusiveMutable, field(obj, "x")),
        );

        let bb1 = b.new_block();
        b.push(bb1, storage_dead(r1));
        b.push(bb1, storage_dead(r2));
        b.set_terminator(bb1, return_(vec![]));

        b.set_terminator(
            bb0,
            call_dispatch(use_id, "use_both", vec![r1, r2], bb1, vec![]),
        );

        let body = b.build(bb0);
        let result = check_body(&body);
        println!("=== BORROW CHECK (clash) ===");
        for err in &result.errors {
            println!("  {err}");
        }
        assert!(
            !result.is_ok(),
            "two exclusive borrows of obj.x must conflict"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::NllExclusivityViolation { .. })),
            "should have E2440, got: {:?}",
            result.errors
        );
    }

    /// Cross-call loan propagation: a callee `get_cell(obj) -> &mut Cell`
    /// returns a reference borrowed from param 0. In the caller, the returned
    /// reference keeps `obj` frozen — so re-borrowing `obj` while that return
    /// value is still live MUST be rejected (E2440). Without the callee's
    /// signature (no propagation) the same code is wrongly accepted, proving
    /// the propagation is what extends the lifetime across the call.
    #[test]
    fn returned_reference_extends_source_lifetime() {
        // Callee signature: return value (Root) borrows from param 0.
        let mut cb = MirBuilder::new("get_cell", "Cell");
        cb.add_param("obj", ParameterPassing::MutableBorrow);
        cb.set_return_borrow(FieldPath::Root, vec![0]);
        let cbb = cb.new_block();
        cb.set_terminator(cbb, return_(vec![]));
        let callee = cb.build(cbb);
        let mut sigs: BTreeMap<String, FunctionSignature> = BTreeMap::new();
        sigs.insert("get_cell".to_string(), callee.signature.clone());

        // Caller: r1 = &mut obj; ret = get_cell(r1); r3 = &mut obj; use(ret, r3).
        let mut b = MirBuilder::new("caller", "Unit");
        let obj = b.add_param("obj", ParameterPassing::MutableBorrow);
        let get_id = b.new_func_id();
        let use_id = b.new_func_id();
        let r1 = b.new_local();
        let ret = b.new_local();
        let r3 = b.new_local();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(r1));
        b.push(bb0, borrow(r1, ReferenceForm::BorrowExclusiveMutable, obj));

        let bb1 = b.new_block();
        b.push(bb1, storage_live(r3));
        b.push(bb1, borrow(r3, ReferenceForm::BorrowExclusiveMutable, obj));

        let bb2 = b.new_block();
        b.push(bb2, storage_dead(r1));
        b.push(bb2, storage_dead(ret));
        b.push(bb2, storage_dead(r3));
        b.set_terminator(bb2, return_(vec![]));

        b.set_terminator(
            bb0,
            call_dispatch(get_id, "get_cell", vec![r1], bb1, vec![ret]),
        );
        b.set_terminator(
            bb1,
            call_dispatch(use_id, "use_both", vec![ret, r3], bb2, vec![]),
        );

        let body = b.build(bb0);
        println!("=== MIR (caller) ===\n{body}");

        // WITH propagation: obj stays frozen via `ret` → re-borrow rejected.
        let with_prop = check_body_with(&body, &sigs);
        println!("=== WITH propagation ===");
        for e in &with_prop.errors {
            println!("  {e}");
        }
        assert!(
            with_prop
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::NllExclusivityViolation { .. })),
            "returned ref must extend obj's loan → re-borrow is E2440, got: {:?}",
            with_prop.errors
        );

        // WITHOUT the callee signature: no propagation, lifetime not extended,
        // so the re-borrow is (wrongly) accepted — proving propagation matters.
        let without_prop = check_body(&body);
        assert!(
            without_prop.is_ok(),
            "without propagation the bridge is blind; expected no error, got: {:?}",
            without_prop.errors
        );
    }
}

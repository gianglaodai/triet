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
//! - **E2450 DropWhileBorrowed**: dropping a variable that still has active
//!   loans — the references would become dangling.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use triet_mir::{
    BasicBlock, Local, Place, Projection, ReferenceForm, Span, Statement, Terminator,
    builtin_shim_meta, is_copy,
};

use miette::Diagnostic;
use thiserror::Error;

use crate::liveness::LivenessResult;

// ── Place conflict ──────────────────────────────────────────

/// Whether two places may refer to overlapping memory.
///
/// When `conservative` is true, two places with **different base locals**
/// are assumed to conflict — this is necessary for `&0` (shared) and `&-`
/// (weak) borrows, where two distinct reference variables may point to
/// the same allocation. Without pointer-alias analysis we cannot prove
/// them disjoint, so we refuse over guess.
///
/// When `conservative` is false, different base locals are assumed
/// disjoint — safe for `&+ mutable` (exclusive) strong borrows where
/// the S6 system guarantees no aliasing, and for `BorrowExclusiveMutable`
/// where the exclusivity guarantees no other mutable access.
///
/// With the same base, projections are compared step-by-step: two distinct
/// **fields** (`obj.x` vs `obj.y`) are provably disjoint, so they do NOT
/// conflict — this is what enables field-level NLL. Anything we cannot prove
/// disjoint (a prefix relationship, an `Index`, or mismatched projection
/// kinds) is treated as overlapping (refuse over guess).
fn places_conflict(a: &Place, b: &Place, conservative: bool) -> bool {
    if a.local != b.local {
        return conservative;
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
            // A strong (moving) borrow conflicts with ANY active loan — moving
            // the value invalidates every reference that points into it.
            ReferenceForm::StrongFrozen | ReferenceForm::StrongMutable => true,
        }
    }
}

// ── Variable state ──────────────────────────────────────────

/// Tracked state of a local variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VarState {
    /// Variable is owned and can be used.
    Owned,
    /// Variable was moved (by Assign or strong reference) — any use is E2420.
    Moved,
    /// Variable storage was ended by Drop/StorageDead — Return can still
    /// consume it (returning a value after its storage ends is fine), but
    /// any other use is E2420. Distinguished from `Moved` so the E2450
    /// check at Return is not accompanied by a false-positive E2420.
    Ended,
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
            let any_ended = predecessors
                .iter()
                .any(|s| s.var_states.get(&local) == Some(&VarState::Ended));
            if any_moved {
                merged.var_states.insert(local, VarState::Moved);
            } else if any_ended {
                // Ended on some path, Owned on others — be conservative
                // and treat as Owned (value may still be available).
                merged.var_states.insert(local, VarState::Owned);
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
    /// E2423: Cannot copy Move type out of projection.
    #[error(
        "E2423: cannot extract type `{ty}` from `{place}` by value because it has Move semantics"
    )]
    #[diagnostic(
        code(triet::borrow::E2423),
        help("borrow the field instead of moving it out")
    )]
    CannotCopyMoveTypeOut {
        /// The place being extracted from.
        place: String,
        /// The type of the value being extracted.
        ty: String,
        /// Source location.
        #[label("cannot copy move type out here")]
        span: Span,
    },

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

    /// E2450: Drop while borrowed — dropping or returning a variable that
    /// still has active loans would leave dangling references.
    #[error("E2450: `{name}` still has active borrows — cannot end its storage here")]
    #[diagnostic(
        code(triet::borrow::E2450),
        help("ensure all references to `{name}` have ended before it goes out of scope")
    )]
    DropWhileBorrowed {
        /// The variable being dropped while borrowed.
        local: Local,
        /// Human-readable variable name.
        name: String,
        /// Source location of the drop.
        #[label("drop occurs here while borrows are still active")]
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
    // Merge let-bound local names from the lowerer.
    names.extend(body.local_names.clone());
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
            Projection::Payload(v) => format!("{s}.Payload({v})"),
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
        body,
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
            body,
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
    body: &triet_mir::Body,
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

            // ADR-0042 Δ4: Deinit = tombstone. Not a user re-init —
            // compiler-emitted zero after a Move-type arg is passed to
            // a Jit call. Sets the local to Moved so that subsequent
            // uses are E2420. Contrast with user Assign/Const which
            // revive to Owned (valid re-initialization).
            Statement::Deinit(l, _) => {
                state.var_states.insert(*l, VarState::Moved);
            }

            Statement::Borrow {
                dest,
                form,
                source,
                span,
            } => {
                // Check for conflicts with active loans (field-level: only
                // overlapping places conflict — `obj.x` vs `obj.y` do not).
                //
                // Shared borrows (&0) and weak observers (&-) can alias —
                // two different reference locals may point to the same
                // allocation. Without alias analysis, be conservative:
                // assume conflict when base locals differ.
                let may_alias = matches!(
                    *form,
                    ReferenceForm::BorrowReadOnly | ReferenceForm::WeakObserver
                );
                for loan in &state.active_loans {
                    if places_conflict(&loan.source, source, may_alias)
                        && loan.conflicts_with(*form)
                    {
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
                // Field access is a copy, not a move — reading `obj.field`
                // does not consume the whole object. Only a plain-local
                // source (no projections) is a genuine move of the base.
                let is_field_read = !source.projection.is_empty();

                // Δ3: Refuse to copy a Move type out of a projection
                if is_field_read {
                    let extracted_ty = triet_mir::place_type(source, body);
                    if !triet_mir::is_copy(&extracted_ty, body) {
                        errors.push(BorrowError::CannotCopyMoveTypeOut {
                            place: place_name(source, names),
                            ty: extracted_ty,
                            span: span.clone(),
                        });
                    }
                }

                if !is_field_read && state.var_states.get(&source.local) == Some(&VarState::Moved) {
                    errors.push(BorrowError::UseAfterMove {
                        local: source.local,
                        name: place_name(source, names),
                        span: span.clone(),
                    });
                }

                // A move must conflict with ANY active loan — even on
                // a different local, because a reference could alias
                // the moved value. Conservative: assume overlap.
                // Field reads do not conflict (they don't move the base).
                //
                // TODO(F-d): After Copy/Move type-awareness, a Copy-type
                // plain-source read should NOT conflict with a shared &0
                // borrow (only exclusive). Currently flagging ALL loans
                // for ALL plain-source assigns is conservative — safe but
                // false-positive on Copy reads under shared borrows.
                // SPEC §10.1: Copy types read without move; S6: shared
                // borrow plus read is valid. Fix when places_conflict is
                // refined to distinguish Copy-read vs Move-source.
                if !is_field_read {
                    let conflicting = state
                        .active_loans
                        .iter()
                        .find(|l| places_conflict(&l.source, source, true));
                    if let Some(loan) = conflicting {
                        errors.push(BorrowError::NllExclusivityViolation {
                            source_local: source.local,
                            source_name: place_name(source, names),
                            new_form: ReferenceForm::StrongMutable,
                            existing_loan_dest: loan.dest,
                            span: span.clone(),
                        });
                    }
                    // Δ1: Assign plain-source only marks Moved if type is Move
                    let source_ty = &body.local_decls[source.local.0].ty;
                    if !triet_mir::is_copy(source_ty, body) {
                        state.var_states.insert(source.local, VarState::Moved);
                    }
                }
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

            Statement::StructAlloc { .. } => {
                // No borrow-check impact — StorageLive already set the local
                // to Owned. StructAlloc just declares stack layout.
            }

            Statement::EnumAlloc { .. } => {
                // No borrow-check impact — same reasoning as StructAlloc.
                // StorageLive already owns the local; EnumAlloc just declares
                // stack layout.
            }

            Statement::SetDiscriminant { .. } => {
                // No borrow-check impact — unconditional write to metadata.
                // Discriminant is not a user-accessible field and can't be
                // borrowed separately.
            }

            Statement::GetDiscriminant { dest, source, span } => {
                // Reading the discriminant is a USE of the enum (not a move).
                // If the enum has been moved → E2420.
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
                // Check for active loans on the dropped variable — dropping
                // while borrows are still live would create dangling references.
                let has_active_loans = state
                    .active_loans
                    .iter()
                    .any(|loan| loan.source.local == *l);
                if has_active_loans {
                    let l_name = names.get(l).cloned().unwrap_or_else(|| format!("{l}"));
                    errors.push(BorrowError::DropWhileBorrowed {
                        local: *l,
                        name: l_name,
                        span: span.clone(),
                    });
                }

                // NOTE: Drop does NOT flag UseAfterMove for Moved/Ended locals.
                // A local moved by Assign (e.g., into a struct field) is legitimately
                // transferred; Dropping it afterwards is a no-op. UseAfterMove is
                // caught by subsequent reads/writes (and by the Return terminator
                // check below).
                //
                // Δ2: `Moved` is sticky. If it was `Moved`, it stays `Moved`.
                // For `Copy` types, they never become `Moved` on assign, so they safely
                // transition to `Ended`. Move types that are `Moved` will stay `Moved`,
                // correctly failing the Return check if F1 gap is triggered.
                if state.var_states.get(l) != Some(&VarState::Moved) {
                    state.var_states.insert(*l, VarState::Ended);
                }
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
    // `Ended` (set by Drop) is acceptable for Return — returning a value
    // after its storage logically ends is fine; the E2450 check below will
    // catch any dangling-reference issues.
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

    // Check Return terminator for E2450: returning a value that still has
    // active loans would create a dangling reference (the reference outlives
    // the borrowed value, whose storage ends at the function boundary).
    if let Terminator::Return { .. } = &block_data.terminator {
        for r in &term_reads {
            let has_active_loans = state
                .active_loans
                .iter()
                .any(|loan| loan.source.local == *r);
            if has_active_loans {
                let r_name = names.get(r).cloned().unwrap_or_else(|| format!("{r}"));
                errors.push(BorrowError::DropWhileBorrowed {
                    local: *r,
                    name: r_name,
                    span: term_span.clone(),
                });
            }
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

    // M3: builtin shim consume-arg tracking (ADR-0040 §3.6).
    // After a CallDispatch to a builtin shim, mark consume-args Moved
    // so that subsequent uses are E2420.
    if let Terminator::CallDispatch {
        callee_name, args, ..
    } = &block_data.terminator
        && let Some(meta) = builtin_shim_meta(callee_name)
    {
        for (i, arg) in args.iter().enumerate() {
            if i < meta.arg_consumes.len() && meta.arg_consumes[i] {
                let arg_ty = &body.local_decls[arg.0].ty;
                if !is_copy(arg_ty, body) {
                    state.var_states.insert(*arg, VarState::Moved);
                }
            }
        }
    }

    // M3+ (ADR-0042 Q4): user function move-marking.
    // Keyed by CallTarget::Jit — all Move-type args are consumed.
    // Check-then-mark: aliased double-move (e.g. foo(s, s)) → E2420
    // BEFORE marking, so that the callee never receives two params
    // pointing to the same heap allocation (would double-free inside
    // the callee, which caller zeroing cannot fix).
    // M3+ (ADR-0042 Q4): user function move-marking.
    // Keyed by CallTarget::Jit — all Move-type args are consumed.
    // Check-then-mark: aliased double-move (e.g. foo(s, s)) → E2420
    // BEFORE marking, so that the callee never receives two params
    // pointing to the same heap allocation (would double-free inside
    // the callee, which caller zeroing cannot fix).
    if let Terminator::CallDispatch {
        target: triet_mir::CallTarget::Jit,
        args,
        ..
    } = &block_data.terminator
    {
        for arg in args.iter() {
            let arg_ty = &body.local_decls[arg.0].ty;
            if !is_copy(arg_ty, body) {
                if matches!(state.var_states.get(arg), Some(VarState::Moved)) {
                    let name = body
                        .local_names
                        .get(arg)
                        .cloned()
                        .unwrap_or_else(|| format!("_{}", arg.0));
                    errors.push(BorrowError::UseAfterMove {
                        local: *arg,
                        name,
                        span: terminator_span(&block_data.terminator),
                    });
                } else {
                    state.var_states.insert(*arg, VarState::Moved);
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
        | Terminator::Unreachable { span }
        | Terminator::Trap { span }
        | Terminator::SwitchInt { span, .. } => span.clone(),
    }
}

/// Return the locals READ by a terminator.
fn terminator_reads(term: &Terminator) -> Vec<Local> {
    match term {
        Terminator::Return { values, .. } => values.clone(),
        Terminator::Goto { .. } => Vec::new(),
        Terminator::If { cond, .. } => vec![*cond],
        Terminator::CallDispatch { args, .. } => args.clone(),
        Terminator::Unreachable { .. } | Terminator::Trap { .. } => Vec::new(),
        Terminator::SwitchInt { discriminant, .. } => vec![*discriminant],
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
        CallTarget, DUMMY_SPAN, FieldPath, FunctionSignature, Local, ParameterPassing, Place,
        Projection, ReferenceForm, ReturnShape, Statement, Terminator,
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
        b.set_local_type(vga, "String");
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
    fn move_while_borrowed_rejected() {
        let mut b = MirBuilder::new("move_while_borrowed", "Unit");
        let vga = b.add_param("vga", ParameterPassing::Move);
        b.set_local_type(vga, "String");
        let b1 = b.new_local();
        let write_cell_id = b.new_func_id();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(b1));
        b.push(bb0, borrow(b1, ReferenceForm::BorrowExclusiveMutable, vga));
        let moved_vga = b.new_local();
        b.push(bb0, crate::assign(moved_vga, vga)); // move while &0 mutable borrow is active

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

        println!("=== BORROW CHECK (move while borrowed) ===");
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

    /// Δ1: plain-source assign of a Move type marks the source Moved,
    /// and subsequent use of the source is E2420 UseAfterMove.
    #[test]
    fn use_after_move_rejected() {
        let mut b = MirBuilder::new("use_after_move", "Unit");
        let s = b.add_param("s", ParameterPassing::Move);
        b.set_local_type(s, "String");
        let other = b.new_local();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(other));
        b.push(bb0, crate::assign(other, s)); // move s → other
        // try to use s after move → E2420
        let s2 = b.new_local();
        b.push(bb0, crate::assign(s2, s));

        b.push(bb0, storage_dead(other));
        b.push(bb0, storage_dead(s2));
        b.set_terminator(bb0, return_(vec![]));

        let body = b.build(bb0);
        println!("=== MIR (use_after_move) ===\n{body}");

        let result = check_body(&body);
        println!("=== BORROW CHECK (use_after_move) ===");
        for err in &result.errors {
            println!("  {err}");
        }

        assert!(
            !result.is_ok(),
            "use-after-move of a Move type MUST be rejected"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::UseAfterMove { .. })),
            "should have E2420 UseAfterMove, got: {:?}",
            result.errors
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

    /// E2450: returning a borrowed value while the reference is still alive.
    /// `r = &0 x; return x;` — E2450 must fire because x still has active loans.
    /// The lowerer emits Drop for owned locals in forward order before Return,
    /// so Drop(x) fires before Drop(r) can trigger NLL cleanup of the loan.
    #[test]
    fn e2450_return_borrowed_value() {
        let mut b = MirBuilder::new("demo", "Integer");
        let x = b.add_param("x", ParameterPassing::Move);
        let r = b.new_local();
        let bb0 = b.new_block();
        b.push(bb0, storage_live(r));
        b.push(bb0, borrow(r, ReferenceForm::BorrowReadOnly, x));
        // Simulate flush_all_for_return: Drop in forward order (source first).
        b.push(bb0, Statement::Drop(x, DUMMY_SPAN));
        b.push(bb0, Statement::Drop(r, DUMMY_SPAN));
        b.set_terminator(bb0, return_(vec![x]));
        let body = b.build(bb0);
        println!("=== MIR ===\n{body}");
        let result = check_body(&body);
        println!("=== ERRORS ===");
        for err in &result.errors {
            println!("  {err}");
        }
        assert!(
            !result.is_ok(),
            "E2450 must fire when returning borrowed value, got no errors"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::DropWhileBorrowed { .. })),
            "expected E2450 DropWhileBorrowed, got: {:?}",
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

    /// Δ3: copying a Move type out of a struct field is forbidden (E2423).
    #[test]
    fn cannot_copy_move_type_out_of_field() {
        let mut b = MirBuilder::new("extract_string_field", "Unit");
        // Add a struct layout with a String field
        b.add_struct_layout(triet_mir::StructLayout::compute(
            "HasString",
            &[("body".into(), "String".into(), 8, triet_mir::align::INTEGER)],
        ));
        let obj = b.add_param("obj", ParameterPassing::Move);
        b.set_local_type(obj, "HasString");

        let bb0 = b.new_block();
        let dest = b.new_local();
        b.push(bb0, storage_live(dest));
        // Assign from obj.body (projected field of Move type) → E2423
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(dest),
                source: field(obj, "body"),
                span: DUMMY_SPAN,
            },
        );
        b.push(bb0, storage_dead(dest));
        b.set_terminator(bb0, return_(vec![]));

        let body = b.build(bb0);
        println!("=== MIR (extract_string_field) ===\n{body}");

        let result = check_body(&body);
        println!("=== BORROW CHECK (extract_string_field) ===");
        for err in &result.errors {
            println!("  {err}");
        }

        assert!(
            !result.is_ok(),
            "copying a Move type out of a projection MUST be rejected"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::CannotCopyMoveTypeOut { .. })),
            "should have E2423 CannotCopyMoveTypeOut, got: {:?}",
            result.errors
        );
    }

    /// F1 chain: Move type assign → Drop → Return must trigger E2420.
    /// The Moved state must be sticky through Drop so that Return sees it.
    #[test]
    fn move_through_drop_to_return_rejected() {
        let mut b = MirBuilder::new("f1_chain", "Unit");
        let s = b.add_param("s", ParameterPassing::Move);
        b.set_local_type(s, "String");
        let other = b.new_local();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(other));
        b.push(bb0, crate::assign(other, s)); // move s → other — s now Moved
        // Drop(s) must NOT clear Moved → s stays Moved
        b.push(bb0, crate::Statement::Drop(s, triet_mir::DUMMY_SPAN));
        // Return(s) with s still Moved → E2420 (the F1 gap)
        b.set_terminator(bb0, return_(vec![s]));

        let body = b.build(bb0);
        println!("=== MIR (f1_chain) ===\n{body}");

        let result = check_body(&body);
        println!("=== BORROW CHECK (f1_chain) ===");
        for err in &result.errors {
            println!("  {err}");
        }

        assert!(
            !result.is_ok(),
            "F1: move → Drop → Return MUST be UseAfterMove"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::UseAfterMove { .. })),
            "should have E2420 UseAfterMove for returned Moved value, got: {:?}",
            result.errors
        );
    }

    /// F1 end-to-end with Move type via enum payload (ADR-0040 §7 fixture 37).
    /// Hand-built MIR: Move String into enum payload, then Return the original
    /// local → E2420. Tests that M1+M2 Marks Moved correctly through Payload assign.
    #[test]
    fn f1_enum_payload_move_type() {
        let mut b = MirBuilder::new("f1_enum_payload", "Unit");
        b.add_enum_layout(triet_mir::EnumLayout::compute(
            "OptionString",
            &[("Some".into(), 0, Some(("String".into(), 8, 8, vec![])))],
        ));
        let s = b.add_param("s", ParameterPassing::Move);
        b.set_local_type(s, "String");
        let a = b.new_local();
        b.set_local_type(a, "OptionString");

        let bb0 = b.new_block();
        b.push(bb0, storage_live(a));
        b.push(
            bb0,
            Statement::EnumAlloc {
                dest: a,
                enum_name: "OptionString".into(),
                span: DUMMY_SPAN,
            },
        );
        b.push(
            bb0,
            Statement::SetDiscriminant {
                dest: a,
                value: 0,
                span: DUMMY_SPAN,
            },
        );
        // Move String s into enum payload → M1 marks s Moved
        b.push(
            bb0,
            Statement::Assign {
                dest: Place {
                    local: a,
                    projection: vec![Projection::Payload("Some".into())],
                },
                source: Place::local(s),
                span: DUMMY_SPAN,
            },
        );
        // Return s after it was moved → E2420
        b.set_terminator(bb0, return_(vec![s]));

        let body = b.build(bb0);
        let result = check_body(&body);
        assert!(
            !result.is_ok(),
            "F1 enum payload: must reject use of moved s"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::UseAfterMove { .. })),
            "should have E2420, got: {:?}",
            result.errors
        );
    }

    /// E2450: dropping a heap value while a borrow is still active.
    #[test]
    fn e2450_heap_drop_while_borrowed() {
        let mut b = MirBuilder::new("e2450_heap", "Unit");
        let s = b.add_param("s", ParameterPassing::Move);
        b.set_local_type(s, "String");
        let r = b.new_local();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(r));
        b.push(bb0, borrow(r, ReferenceForm::BorrowReadOnly, s)); // &0 s
        // Drop s while r (the borrow) is still live → E2450
        b.push(bb0, Statement::Drop(s, DUMMY_SPAN));
        b.push(bb0, Statement::Drop(r, DUMMY_SPAN));
        b.set_terminator(bb0, return_(vec![]));

        let body = b.build(bb0);
        let result = check_body(&body);
        assert!(
            !result.is_ok(),
            "E2450: Drop with active borrow must be rejected"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::DropWhileBorrowed { .. })),
            "should have E2450, got: {:?}",
            result.errors
        );
    }

    /// ADR-0042 Q4: E2420 use-after-move qua user function call
    /// (CallTarget::Jit). M3+ marks Move-type args as Moved after
    /// a CallDispatch to a Jit target. Subsequent use → E2420.
    #[test]
    fn e2420_use_after_move_via_jit_call() {
        let mut b = MirBuilder::new("caller", "Unit");
        let s = b.add_param("s", ParameterPassing::Move);
        b.set_local_type(s, "String");
        let callee_id = b.new_func_id();

        let bb0 = b.new_block();
        let bb1 = b.new_block();
        let bb2 = b.new_block();

        // Call consume(my_string) — s is Move-type, must be marked Moved.
        b.set_terminator(
            bb0,
            Terminator::CallDispatch {
                callee: callee_id,
                callee_name: "consume".to_string(),
                target: CallTarget::Jit,
                args: vec![s],
                return_bb: bb1,
                dest: vec![],
                return_shape: ReturnShape::Scalar,
                span: DUMMY_SPAN,
            },
        );

        // bb1: use s (Assign reads it) after it was moved → E2420
        let t = b.new_local();
        b.push(bb1, storage_live(t));
        b.push(
            bb1,
            Statement::Assign {
                dest: Place::local(t),
                source: Place::local(s),
                span: DUMMY_SPAN,
            },
        );
        b.set_terminator(bb1, return_(vec![]));

        b.set_terminator(bb2, return_(vec![]));

        let body = b.build(bb0);
        let result = check_body(&body);
        assert!(
            !result.is_ok(),
            "ADR-0042 Q4: using s after consume(s) must be E2420"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::UseAfterMove { .. })),
            "should have E2420 after Jit call, got: {:?}",
            result.errors
        );
    }

    /// ADR-0042 Q4 teeth: gỡ move-marking cho CallTarget::Jit → test này đỏ
    /// (không có E2420, pipeline xanh oan).
    #[test]
    fn e2420_jit_call_teeth_requires_marking() {
        let mut b = MirBuilder::new("caller", "Unit");
        let s = b.add_param("s", ParameterPassing::Move);
        b.set_local_type(s, "String");
        let callee_id = b.new_func_id();

        let bb0 = b.new_block();
        let bb1 = b.new_block();
        b.set_terminator(
            bb0,
            Terminator::CallDispatch {
                callee: callee_id,
                callee_name: "consume".to_string(),
                target: CallTarget::Jit,
                args: vec![s],
                return_bb: bb1,
                dest: vec![],
                return_shape: ReturnShape::Scalar,
                span: DUMMY_SPAN,
            },
        );
        // Use s after call — marking code must fire E2420.
        let t = b.new_local();
        b.push(bb1, storage_live(t));
        b.push(
            bb1,
            Statement::Assign {
                dest: Place::local(t),
                source: Place::local(s),
                span: DUMMY_SPAN,
            },
        );
        b.set_terminator(bb1, return_(vec![]));

        let body = b.build(bb0);
        let result = check_body(&body);
        assert!(
            !result.is_ok(),
            "E2420 teeth: marking must fire; got Ok (no errors)"
        );
    }

    /// ADR-0042 Δ4: Deinit = tombstone (→ Moved), but user Assign
    /// revives to Owned (valid re-init after move).
    #[test]
    fn deinit_tombstone_user_assign_revives() {
        let mut b = MirBuilder::new("test", "Unit");
        let s = b.add_param("s", ParameterPassing::Move);
        b.set_local_type(s, "String");

        let bb0 = b.new_block();
        let bb1 = b.new_block();

        // Jit call: consume(s) — marks s Moved
        let callee_id = b.new_func_id();
        b.set_terminator(
            bb0,
            Terminator::CallDispatch {
                callee: callee_id,
                callee_name: "consume".to_string(),
                target: CallTarget::Jit,
                args: vec![s],
                return_bb: bb1,
                dest: vec![],
                return_shape: ReturnShape::Scalar,
                span: DUMMY_SPAN,
            },
        );

        // bb1: Deinit(s) — tombstone, keeps Moved
        b.push(bb1, Statement::Deinit(s, DUMMY_SPAN));

        // user Assign s = new_value — revives to Owned
        let new_val = b.new_local();
        b.set_local_type(new_val, "String");
        b.push(bb1, storage_live(new_val));
        b.push(
            bb1,
            Statement::Assign {
                dest: Place::local(s),
                source: Place::local(new_val),
                span: DUMMY_SPAN,
            },
        );
        // Use s after re-init → no error
        b.set_terminator(bb1, return_(vec![s]));

        let body = b.build(bb0);
        let result = check_body(&body);
        assert!(
            result.is_ok(),
            "Deinit→Assign: user re-init must revive Owned, got: {:?}",
            result.errors
        );
    }

    /// ADR-0042 Δ4: Deinit + use without re-init → E2420.
    #[test]
    fn deinit_without_reinit_is_e2420() {
        let mut b = MirBuilder::new("test", "Unit");
        let s = b.add_param("s", ParameterPassing::Move);
        b.set_local_type(s, "String");

        let bb0 = b.new_block();
        let bb1 = b.new_block();

        let callee_id = b.new_func_id();
        b.set_terminator(
            bb0,
            Terminator::CallDispatch {
                callee: callee_id,
                callee_name: "consume".to_string(),
                target: CallTarget::Jit,
                args: vec![s],
                return_bb: bb1,
                dest: vec![],
                return_shape: ReturnShape::Scalar,
                span: DUMMY_SPAN,
            },
        );
        b.push(bb1, Statement::Deinit(s, DUMMY_SPAN));
        // Use s after Deinit without re-init → E2420
        b.set_terminator(bb1, return_(vec![s]));

        let body = b.build(bb0);
        let result = check_body(&body);
        assert!(
            !result.is_ok(),
            "Deinit→use: must be E2420 (tombstone, no re-init)"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::UseAfterMove { .. })),
            "should have E2420, got: {:?}",
            result.errors
        );
    }
}

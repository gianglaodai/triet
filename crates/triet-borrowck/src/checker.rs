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
#[cfg(test)]
use triet_mir::MirType;
use triet_mir::{
    BasicBlock, Local, Place, Projection, ReferenceForm, Span, Statement, Terminator,
    builtin_shim_meta,
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
    /// ADR-0046: true for PropagatedLoan (return-borrow at call site).
    /// Propagated loans are bounded by the dest's liveness, not StorageDead
    /// — E2450 is skipped when the source drops because the dest is already
    /// dead. Direct loans (is_propagated=false) are bounded by scope exit.
    is_propagated: bool,
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
    /// WO-0075 (ADR-0070 §AMEND Phase 3): per-local set of moved-out projection
    /// PATHS. A single-level move (`h.f`) records `["f"]`; a multi-level move
    /// (`h.inner.x`) records `["inner", "x"]`. A path in this set is dead: reading
    /// an exact/ancestor/descendant path, or any whole-base use of `base`, is
    /// E2420 (`prefix_conflict`); a sibling path stays live. Union-merged across
    /// predecessors (monotone → fixpoint).
    partial_moves: BTreeMap<Local, BTreeSet<Vec<String>>>,
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
            partial_moves: BTreeMap::new(),
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

        // ADR-0070 / WO-0075: partial_moves = UNION of moved projection-PATH sets
        // across all predecessors. A path moved on ANY branch is moved at the join
        // (a value gone down one branch cannot be resurrected by another). Union
        // is monotone, so the dataflow fixpoint converges. Intersection would be
        // UNSOUND (it would forget a move on a sibling path) — tooth-F pins this.
        for pred in predecessors {
            for (local, paths) in &pred.partial_moves {
                merged
                    .partial_moves
                    .entry(*local)
                    .or_default()
                    .extend(paths.iter().cloned());
            }
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

    /// E2424: Sub-path reassignment unsupported (WO-0075, ADR-0070 §AMEND Phase 3).
    /// Reassigning an ancestor place (`h.inner = ...`) while a nested field of it
    /// has been moved out (`h.inner.x`) is NOT yet supported — the JIT cannot
    /// reconcile the moved leaf's tombstone with a whole-ancestor overwrite.
    /// Locked with a diagnostic rather than silently clearing stale move-state.
    #[error("E2424: cannot reassign `{place}` while a nested field of it has been moved out")]
    #[diagnostic(
        code(triet::borrow::E2424),
        help(
            "reassign the whole base instead, or avoid moving a nested field before overwriting an ancestor"
        )
    )]
    SubPathReassignUnsupported {
        /// The ancestor place being reassigned.
        place: String,
        /// Source location of the reassignment.
        #[label("ancestor reassigned here while a nested field is moved")]
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

    /// E2421: Use after storage end — a Move-type variable was used after
    /// its storage was ended by Drop/StorageDead (ADR-0054).
    /// Distinct from E2420: E2420 = "moved to someone else" (active transfer);
    /// E2421 = "lifetime ended by Drop, can't resurrect" (lifetime/deallocation).
    /// Return can still consume an Ended local (no false-positive at Return).
    #[error(
        "E2421: use after storage end — `{name}` was used after its storage was deallocated (Drop)"
    )]
    #[diagnostic(
        code(triet::borrow::E2421),
        help(
            "the value has been dropped and can no longer be used; move the use before the drop, or restructure the code to extend the value's lifetime"
        )
    )]
    UseAfterStorageEnd {
        /// The variable that was used after storage ended.
        local: Local,
        /// Human-readable variable name.
        name: String,
        /// Source location of the use.
        #[label("used here after storage was deallocated")]
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
    for (i, (name, _)) in body.signature.parameters.iter().enumerate() {
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
            Projection::OutcomeDiscriminant => format!("{s}.disc"),
            Projection::OutcomePayload => format!("{s}.payload"),
            Projection::OutcomePayloadLen => format!("{s}.payload_len"),
            Projection::OutcomePayloadCap => format!("{s}.payload_cap"),
        };
    }
    s
}

/// WO-0075 (ADR-0070 §AMEND Phase 3): the full Field-projection PATH of a place,
/// e.g. `h.inner.x` → `["inner", "x"]`. Returns `Some([])` for a bare local
/// (whole base) and `None` for ANY non-Field projection (Index/Deref/Payload/
/// OutcomeDiscriminant) — those are out of scope and callers treat them
/// conservatively (whole-base on read, refuse on extraction).
fn projection_path(place: &Place) -> Option<Vec<String>> {
    let mut path = Vec::new();
    for proj in &place.projection {
        match proj {
            Projection::Field(name) => path.push(name.clone()),
            _ => return None,
        }
    }
    Some(path)
}

/// WO-0075: do two projection paths conflict by prefix — i.e. is one a prefix of
/// the other (equality included)? A read-path `p` conflicts with a moved-path
/// `m` when `p` is `m` (exact-dead), an ancestor of `m` (`h.inner` after moving
/// `h.inner.x`), a descendant of `m`, or either is `[]` (whole-base touches any
/// moved field). Two SIBLING paths (`h.inner.x` vs `h.inner.y`) share the
/// `[inner]` prefix but diverge at the leaf → NOT a conflict → both stay live.
fn prefix_conflict(p: &[String], m: &[String]) -> bool {
    let n = p.len().min(m.len());
    p[..n] == m[..n]
}

/// WO-0075 (ADR-0070 §AMEND Phase 3): does reading `place` touch storage already
/// partially moved out? The read-path conflicts with a moved-path when one is a
/// prefix of the other (see `prefix_conflict`): exact field, ancestor, descendant,
/// or whole-base all conflict; a SIBLING field stays live. A non-Field projection
/// (`projection_path` → None) is treated as a whole-base use (path `[]`), which
/// prefixes every moved path → conservative. This is SEPARATE from the whole-
/// `Moved` var_state check each call site already performs.
fn partial_move_invalidates(state: &BlockState, place: &Place) -> bool {
    let Some(moved) = state.partial_moves.get(&place.local) else {
        return false;
    };
    if moved.is_empty() {
        return false;
    }
    // None (non-Field projection) → `[]` = whole-base conservative.
    let p = projection_path(place).unwrap_or_default();
    moved.iter().any(|m| prefix_conflict(&p, m))
}

/// Check if a Move-type local in `Ended` state is being used
/// and emit E2421 UseAfterStorageEnd (ADR-0054).  Copy types
/// are exempt — their Drop is a no-op on the stack.
fn check_use_after_end(
    state: &BlockState,
    body: &triet_mir::Body,
    local: Local,
    name: String,
    span: Span,
    errors: &mut Vec<BorrowError>,
) {
    if state.var_states.get(&local) == Some(&VarState::Ended)
        && !body.local_decls[local.0].ty.is_copy(Some(body))
    {
        errors.push(BorrowError::UseAfterStorageEnd { local, name, span });
    }
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
    entry_states[cfg.entry.0] = BlockState::initial(body.signature.parameters.len());

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

        // WO-0075 Commit 1 (§4 fixpoint-hole, pre-existing bug): a partial move
        // does NOT set the base local to `Moved` (it only adds a field to
        // `partial_moves`), so a partial-move delta is INVISIBLE to the
        // `var_states`/`active_loans` comparison. Without the third clause, a
        // field moved in a loop body never propagates back across the back-edge
        // → the fixpoint converges prematurely → a use-after-move on the next
        // iteration is SILENTLY MISSED (tooth-G). `partial_moves` is union-merged
        // (monotone), so adding it to the convergence test keeps termination.
        if new_entry.var_states == entry_states[block.0].var_states
            && new_entry.active_loans == entry_states[block.0].active_loans
            && new_entry.partial_moves == entry_states[block.0].partial_moves
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

        // WO-0075 Commit 1 (§4 fixpoint-hole): same reasoning as the entry
        // compare above — a partial-move delta must mark the exit state dirty so
        // it propagates to successors, else a back-edge never carries the move.
        if new_exit.var_states != exit_states[block.0].var_states
            || new_exit.active_loans != exit_states[block.0].active_loans
            || new_exit.partial_moves != exit_states[block.0].partial_moves
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
                // ADR-0070: a fresh storage slot has no moved-out fields.
                state.partial_moves.remove(l);
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
                // ADR-0084 Slice 1b: a sub-borrow reached THROUGH an existing
                // reference (`source` starts with `Projection::Deref` — e.g.
                // `h.name` where `h: &0 Holder`) cannot be tracked at field
                // granularity — WHOLE-OBJECT FALLBACK (refuse-over-guess, G
                // mandate): the loan covers the entire object reached via the
                // reference, not just the projected field. Cost (accepted):
                // two sub-borrows of DIFFERENT fields through the SAME
                // reference (`h.name` and `h.other`) conservatively conflict.
                //
                // A plain strip to `Place::local(source.local)` is NOT enough
                // for the combo form `(&0 h).name` (an inline re-borrow of a
                // LOCAL, not a reference param): that lowers to TWO Borrow
                // statements — an inner `tmp = &0 h` (loan A: source=h,
                // dest=tmp) then this outer sub-borrow (source={tmp,[Deref,
                // Field]}). `tmp` is a compiler temp whose only use IS this
                // statement, so loan A is pruned by NLL dest-liveness the
                // instant this statement finishes processing — well before a
                // later `Drop(h)`/move-of-`h` is checked, silently missing a
                // dangling reference. REBORROW CHASE: if `source.local` is
                // itself the `dest` of an existing active loan (i.e. it was
                // JUST created by borrowing something), inherit THAT loan's
                // `source` — anchoring the new loan on the reborrow's true
                // origin (`h`) instead of the short-lived intermediate. When
                // `source.local` is a reference obtained some other way (a
                // function PARAMETER already typed `&0 T`, no local borrow
                // statement created it within this function) there is no such
                // loan to chase — falls back to `Place::local(source.local)`.
                let loan_source = if source
                    .projection
                    .iter()
                    .any(|p| matches!(p, Projection::Deref))
                {
                    state
                        .active_loans
                        .iter()
                        .find(|l| l.dest == source.local)
                        .map_or_else(|| Place::local(source.local), |l| l.source.clone())
                } else {
                    source.clone()
                };

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
                    if places_conflict(&loan.source, &loan_source, may_alias)
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

                // Check that the borrowed base hasn't been moved/deallocated.
                // ADR-0070 (G's note): borrowing the whole base `&hw` after a
                // field was moved out, or `&hw.f` on a moved field, is a
                // use-after-move (a sibling field borrow stays valid).
                if state.var_states.get(&source.local) == Some(&VarState::Moved)
                    || partial_move_invalidates(&state, source)
                {
                    errors.push(BorrowError::UseAfterMove {
                        local: source.local,
                        name: place_name(source, names),
                        span: span.clone(),
                    });
                }
                check_use_after_end(
                    &state,
                    body,
                    source.local,
                    place_name(source, names),
                    span.clone(),
                    &mut errors,
                );

                match form {
                    ReferenceForm::StrongFrozen | ReferenceForm::StrongMutable => {
                        state.var_states.insert(source.local, VarState::Moved);
                    }
                    _ => {
                        state.active_loans.insert(Loan {
                            source: loan_source,
                            dest: dest.local,
                            form: *form,
                            issued_in: block,
                            issued_at: stmt_idx,
                            is_propagated: false,
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
                    // ADR-0070: re-reading a field that was already partially
                    // moved out (or any whole-base/multi-level read while a
                    // field is gone) is a use-after-move. Checked BEFORE this
                    // statement records its own move below.
                    if partial_move_invalidates(&state, source) {
                        errors.push(BorrowError::UseAfterMove {
                            local: source.local,
                            name: place_name(source, names),
                            span: span.clone(),
                        });
                    }
                    if !extracted_ty.is_copy(Some(body)) {
                        // A field MAY be moved out when it is a ZST capability OR
                        // a heap SCALAR (String/Vector/HashMap) OR a heap-STRUCT
                        // (Phase 2) OR a heap-carrying ENUM (WO-0074) — record the
                        // partial move. WO-0075 (ADR-0070 §AMEND Phase 3): the
                        // recorded key is now the full Field PATH (`projection_path`
                        // → ["inner", "x"] for multi-level), so multi-level
                        // extraction is OPENED (was refused). The JIT tombstones the
                        // moved leaf at its absolute offset in the base slot.
                        // WO-NullableFieldMoveOut (ADR-0070 §AMEND Phase 4 +
                        // ADR-0076 §AMEND): a heap-`T?` field (`String?`/`Vector?`/
                        // `HashMap?`) is now ALSO move-out-able. Its slot IS the
                        // drop-flag (static tombstone): the moved-out leaf ptr is
                        // zeroed in the base, and the free shim no-ops on
                        // 0/NULL_SENTINEL — no per-field dynamic drop-flag needed.
                        // `is_any_heap()` does NOT unwrap `Nullable`, so this arm
                        // matches `Nullable(inner) if inner.is_any_heap()`
                        // EXPLICITLY (not via `extracted_ty.is_any_heap()`).
                        // Still REFUSED (E2423): a NON-Field projection
                        // (`projection_path` → None: Index/Deref/Payload/
                        // OutcomeDiscriminant) and every other non-copy move-type
                        // (Outcome fields, Nullable(scalar/aggregate) are out of
                        // scope, defer).
                        match projection_path(source) {
                            Some(path)
                                if !path.is_empty()
                                    && (matches!(
                                        extracted_ty,
                                        triet_mir::MirType::Capability(_)
                                    ) || extracted_ty.is_any_heap()
                                        || matches!(
                                            extracted_ty,
                                            triet_mir::MirType::Struct(_)
                                        )
                                        || matches!(extracted_ty, triet_mir::MirType::Enum(_))
                                        || matches!(
                                            &extracted_ty,
                                            triet_mir::MirType::Nullable(inner)
                                                if inner.is_any_heap()
                                        )) =>
                            {
                                state
                                    .partial_moves
                                    .entry(source.local)
                                    .or_default()
                                    .insert(path);
                            }
                            _ => {
                                errors.push(BorrowError::CannotCopyMoveTypeOut {
                                    place: place_name(source, names),
                                    ty: extracted_ty.to_string(),
                                    span: span.clone(),
                                });
                            }
                        }
                    }
                    // ADR-0049: field-read on a moved base = use-after-move.
                    // Reading any part of a moved value is unsound even if
                    // the field type is Copy — the base has been zeroed/Deinit.
                    if state.var_states.get(&source.local) == Some(&VarState::Moved) {
                        errors.push(BorrowError::UseAfterMove {
                            local: source.local,
                            name: place_name(source, names),
                            span: span.clone(),
                        });
                    }
                    check_use_after_end(
                        &state,
                        body,
                        source.local,
                        place_name(source, names),
                        span.clone(),
                        &mut errors,
                    );
                }

                if !is_field_read
                    && (state.var_states.get(&source.local) == Some(&VarState::Moved)
                        // ADR-0070: moving the whole base after a field was
                        // partially moved out is a use-after-move.
                        || partial_move_invalidates(&state, source))
                {
                    errors.push(BorrowError::UseAfterMove {
                        local: source.local,
                        name: place_name(source, names),
                        span: span.clone(),
                    });
                }
                check_use_after_end(
                    &state,
                    body,
                    source.local,
                    place_name(source, names),
                    span.clone(),
                    &mut errors,
                );

                // A move must conflict with ANY active loan — even on
                // a different local, because a reference could alias
                // the moved value. Conservative: assume overlap.
                // Field reads do not conflict (they don't move the base).
                //
                // ADR-0079 (F-d resolved): a Copy-type plain-source read
                // does NOT move the value — it's a bitwise copy. A shared
                // &0 borrow plus a Copy read is valid (no exclusivity
                // violation). Skip the conservative conflict check when
                // the source type is Copy (e.g. `Nullable(&0 String)`
                // destructuring the returned reference via match ~+).
                if !is_field_read && !body.local_decls[source.local.0].ty.is_copy(Some(body)) {
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
                    if !source_ty.is_copy(Some(body)) {
                        state.var_states.insert(source.local, VarState::Moved);
                    }
                }
                // ADR-0070 / WO-0075 §F: re-initialization clears stale
                // partial-move state. A whole-local fresh assignment (`hw = ...`)
                // clears every moved path. An EXACT-path store (`hw.f = ...` or
                // `hw.a.b = ...` re-initialising exactly the moved path) re-inits
                // just that path. A NON-exact prefix conflict — reassigning an
                // ANCESTOR (`h.inner = ...` after moving `h.inner.x`) or descendant
                // of a moved path — is a sub-path reassign we do NOT support: emit
                // E2424 and leave the move-state intact (conservative; never clear
                // a path the JIT still has tombstoned). A non-Field dest projection
                // (`projection_path` → None) is left conservative (no clear).
                if dest.projection.is_empty() {
                    state.partial_moves.remove(&dest.local);
                } else if let Some(path) = projection_path(dest)
                    && let Some(s) = state.partial_moves.get_mut(&dest.local)
                {
                    let has_subpath_conflict =
                        s.iter().any(|m| *m != path && prefix_conflict(&path, m));
                    s.remove(&path);
                    if has_subpath_conflict {
                        errors.push(BorrowError::SubPathReassignUnsupported {
                            place: place_name(dest, names),
                            span: span.clone(),
                        });
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
                    if state.var_states.get(&op.local) == Some(&VarState::Moved)
                        || partial_move_invalidates(&state, op)
                    {
                        errors.push(BorrowError::UseAfterMove {
                            local: op.local,
                            name: place_name(op, names),
                            span: span.clone(),
                        });
                    }
                    check_use_after_end(
                        &state,
                        body,
                        op.local,
                        place_name(op, names),
                        span.clone(),
                        &mut errors,
                    );
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

            Statement::OutcomeAlloc { .. } => {
                // No borrow-check impact — same reasoning as StructAlloc.
            }

            Statement::SetDiscriminant { .. } => {
                // No borrow-check impact — unconditional write to metadata.
                // Discriminant is not a user-accessible field and can't be
                // borrowed separately.
            }

            Statement::GetDiscriminant { dest, source, span } => {
                // Reading the discriminant is a USE of the enum (not a move).
                // If the enum has been moved → E2420.
                if state.var_states.get(&source.local) == Some(&VarState::Moved)
                    || partial_move_invalidates(&state, source)
                {
                    errors.push(BorrowError::UseAfterMove {
                        local: source.local,
                        name: place_name(source, names),
                        span: span.clone(),
                    });
                }
                check_use_after_end(
                    &state,
                    body,
                    source.local,
                    place_name(source, names),
                    span.clone(),
                    &mut errors,
                );
                state.var_states.insert(dest.local, VarState::Owned);
            }

            Statement::Drop(l, span) => {
                // Check for active loans on the dropped variable — dropping
                // while borrows are still live would create dangling references.
                //
                // ADR-0046: propagated loans (return-borrow) are bounded by
                // the dest's liveness, not StorageDead. A propagated loan
                // is only safe to suppress if the dest reference is already
                // dead at this Drop point. If the dest is still live (e.g.,
                // nested scope where Drop(source) precedes a use of the
                // returned reference), fire E2450.
                // ADR-0063 §3: point-level READ-after-Drop liveness. A
                // propagated loan is also live if its dest reference is READ
                // (not Drop) by a later statement in THIS block — covers
                // same-block consumption (e.g. an If/match merge `_4 = move _3`
                // that consumes the loan-dest before it reaches live_out, so
                // live_out alone misses the UAF). Drop of the dest itself is NOT
                // a use — the dest is dying too, so there is no false-positive
                // by construction (no valid code reads a ref after its source
                // dies).
                let dest_used_after = |dest: triet_mir::Local| {
                    body.blocks[block.0].statements[stmt_idx + 1..]
                        .iter()
                        .any(|s| match s {
                            Statement::Assign { source, .. }
                            | Statement::Borrow { source, .. }
                            | Statement::GetDiscriminant { source, .. } => source.local == dest,
                            Statement::BinaryOp { left, right, .. } => {
                                left.local == dest || right.local == dest
                            }
                            _ => false,
                        })
                };
                let has_active_loans = state.active_loans.iter().any(|loan| {
                    loan.source.local == *l
                        && (!loan.is_propagated
                            || liveness.blocks[block.0].live_out.contains(&loan.dest)
                            || dest_used_after(loan.dest))
                });
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
                // ADR-0046: also remove loans where this local is the dest.
                // When a reference local dies, any loan it created (via
                // PropagatedLoan or direct borrow) is released.
                state.active_loans.retain(|loan| loan.dest != *l);
            }
            // ADR-0069: capability gate touches no place/local — no borrow effect.
            Statement::CapabilityCheck { .. } => {}
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
    // ADR-0054: non-Return terminators (If, CallDispatch, SwitchInt) MUST
    // enforce Ended → E2421 for Move types.
    let is_return = matches!(&block_data.terminator, Terminator::Return { .. });
    let term_span = terminator_span(&block_data.terminator);
    let term_reads = terminator_reads(&block_data.terminator);
    for r in &term_reads {
        // ADR-0070: a terminator read is a whole-base use of the local — moved
        // if fully Moved OR any field was partially moved out.
        let partially_moved = state.partial_moves.get(r).is_some_and(|s| !s.is_empty());
        if state.var_states.get(r) == Some(&VarState::Moved) || partially_moved {
            let r_name = names.get(r).cloned().unwrap_or_else(|| format!("{r}"));
            errors.push(BorrowError::UseAfterMove {
                local: *r,
                name: r_name,
                span: term_span.clone(),
            });
        }
        // Non-Return terminators: Ended → E2421 (Return stays lenient).
        if !is_return {
            let r_name = names.get(r).cloned().unwrap_or_else(|| format!("{r}"));
            check_use_after_end(&state, body, *r, r_name, term_span.clone(), &mut errors);
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
                        is_propagated: true,
                    });
                }
            }
        }
    }

    // ADR-0079 U2: builtin return-borrow loan propagation.
    // When a builtin shim returns a reference borrowed from one of its
    // arguments, trace through any intermediate borrow to find the REAL
    // source. For `get(&0 m, k)`, the lower emits `_tmp = &0 m` then
    // passes `_tmp` as arg[0]. The loan's source should be `m`, not the
    // temporary `_tmp` — otherwise match destructuring on the returned
    // reference conflicts with the temporary's loan (different locals,
    // conservative=true → E2440 false positive).
    if let Terminator::CallDispatch {
        callee_name,
        args,
        dest,
        ..
    } = &block_data.terminator
        && let Some(meta) = builtin_shim_meta(callee_name)
        && let Some(pi) = meta.returns_borrow_of
        && let Some(&ret_temp) = dest.first()
        && let Some(&arg_local) = args.get(pi)
    {
        let term_idx = block_data.statements.len();
        // Trace through intermediate borrow: if arg_local is the dest of
        // a direct borrow, use THAT loan's source (the real container).
        let real_source = state
            .active_loans
            .iter()
            .find(|l| l.dest == arg_local && !l.is_propagated)
            .map_or(Place::from(arg_local), |l| l.source.clone());
        state.active_loans.insert(Loan {
            source: real_source,
            dest: ret_temp,
            form: ReferenceForm::BorrowReadOnly,
            issued_in: block,
            issued_at: term_idx,
            is_propagated: true,
        });
    }

    // M3 pre-check (ADR-0079 U3): mutate-while-borrowed.
    // BEFORE mutating a container, verify no active loan conflicts with
    // the container's Place. "Mutate" covers two cases:
    //   1. Consume (arg_consumes[i]=true): insert/push — the arg handle is
    //      consumed → caller loses ownership → borrow invalidated.
    //   2. In-place mutate (mutates_arg=Some(i)): remove/pop — the handle
    //      survives but the container's contents are modified (tombstone,
    //      len--, value moved out) → any reference to internal slots is
    //      invalidated. G rules whole-container: even a different-key
    //      mutation is refused when ANY borrow is active.
    if let Terminator::CallDispatch {
        callee_name, args, ..
    } = &block_data.terminator
        && let Some(meta) = builtin_shim_meta(callee_name)
    {
        for (i, arg) in args.iter().enumerate() {
            let is_mutated = (i < meta.arg_consumes.len() && meta.arg_consumes[i])
                || meta.mutates_arg == Some(i);
            if is_mutated {
                let arg_place = Place::from(*arg);
                if let Some(conflicting) = state
                    .active_loans
                    .iter()
                    .find(|l| places_conflict(&l.source, &arg_place, true))
                {
                    let arg_name = names.get(arg).cloned().unwrap_or_else(|| format!("{arg}"));
                    errors.push(BorrowError::NllExclusivityViolation {
                        source_local: *arg,
                        source_name: arg_name,
                        new_form: ReferenceForm::BorrowExclusiveMutable,
                        existing_loan_dest: conflicting.dest,
                        span: term_span.clone(),
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
                if !arg_ty.is_copy(Some(body)) {
                    state.var_states.insert(*arg, VarState::Moved);
                }
            }
        }
    }

    // M3+ (ADR-0042 Q4): user function move-marking.
    // Keyed by CallTarget::Jit — all Move-type args are consumed.
    // Check-then-mark: aliased double-move (e.g. foo(s, s)) → E2420
    // BEFORE marking, so that the callee never receives two parameters
    // pointing to the same heap allocation (would double-free inside
    // the callee, which caller zeroing cannot fix).
    // M3+ (ADR-0042 Q4): user function move-marking.
    // Keyed by CallTarget::Jit — all Move-type args are consumed.
    // Check-then-mark: aliased double-move (e.g. foo(s, s)) → E2420
    // BEFORE marking, so that the callee never receives two parameters
    // pointing to the same heap allocation (would double-free inside
    // the callee, which caller zeroing cannot fix).
    if let Terminator::CallDispatch {
        target: triet_mir::CallTarget::Jit,
        args,
        return_shape,
        ..
    } = &block_data.terminator
    {
        // ADR-0049 Lát 6: sret arg[0] is write-only — caller keeps ownership.
        let skip_sret = matches!(return_shape, triet_mir::ReturnShape::Struct { .. });
        for (i, arg) in args.iter().enumerate() {
            if skip_sret && i == 0 {
                continue;
            }
            let arg_ty = &body.local_decls[arg.0].ty;
            if !arg_ty.is_copy(Some(body)) {
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
                } else if matches!(state.var_states.get(arg), Some(VarState::Ended)) {
                    let name = body
                        .local_names
                        .get(arg)
                        .cloned()
                        .unwrap_or_else(|| format!("_{}", arg.0));
                    errors.push(BorrowError::UseAfterStorageEnd {
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

    // NLL: end loans whose dest is no longer live after the terminator.
    // Without this, a borrow temporary passed to a call stays "alive"
    // across the terminator boundary and blocks Drop in the successor
    // block (E2450 false positive). The statement-level cleanup at line
    // 708 only handles intra-block statements; the terminator needs its
    // own pass because the dest's last use is the call itself.
    let term_idx = block_data.statements.len();
    state
        .active_loans
        .retain(|loan| liveness.is_live_after(block, term_idx, loan.dest));

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
        CallTarget, DUMMY_SPAN, FieldPath, FunctionId, FunctionSignature, Local, ParameterPassing,
        Place, Projection, ReferenceForm, ReturnShape, Statement, Terminator,
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
        let mut b = MirBuilder::new("cross_block_move", MirType::Unit);
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
        let mut b = MirBuilder::new("double_borrow", MirType::Unit);
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
        let mut b = MirBuilder::new("sequential_borrow", MirType::Unit);
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
        let mut b = MirBuilder::new("move_while_borrowed", MirType::Unit);
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
        let mut b = MirBuilder::new("use_after_move", MirType::Unit);
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

    /// ADR-0054 T1: Drop a Move-type local then use it → E2421 UseAfterStorageEnd.
    /// Hiện trạng trước fix: `got: []` (mù). Sau fix: E2421.
    #[test]
    fn drop_then_move_must_be_rejected() {
        let mut b = MirBuilder::new("drop_use", MirType::Unit);
        let s = b.add_param("s", ParameterPassing::Move);
        b.set_local_type(s, "String");
        let other = b.new_local();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(other));
        // Drop s (storage ends)
        b.push(bb0, Statement::Drop(s, DUMMY_SPAN));
        // Use s after Drop → phải bị E2421
        b.push(bb0, crate::assign(other, s));

        b.push(bb0, storage_dead(other));
        b.set_terminator(bb0, return_(vec![]));

        let body = b.build(bb0);
        println!("=== MIR (drop_then_move) ===\n{body}");

        let result = check_body(&body);
        println!("=== BORROW CHECK (drop_then_move) ===");
        for err in &result.errors {
            println!("  {err}");
        }

        assert!(
            !result.is_ok(),
            "use-after-Drop of a Move type MUST be rejected with E2421"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::UseAfterStorageEnd { .. })),
            "should have E2421 UseAfterStorageEnd, got: {:?}",
            result.errors
        );
    }

    /// ADR-0054 T2b: Drop a Copy-type local then use it → NO E2421 (Copy=no-op).
    /// Phạm vi Move-only — siết Copy là false-positive.
    #[test]
    fn drop_then_use_copy_must_be_allowed() {
        let mut b = MirBuilder::new("drop_copy", MirType::Unit);
        let n = b.add_param("n", ParameterPassing::Move);
        b.set_local_mir_type(n, MirType::Integer); // Copy type (must use MirType, not string)
        let other = b.new_local();

        let bb0 = b.new_block();
        b.push(bb0, storage_live(other));
        // Drop n (no-op for Copy — stack-safe)
        b.push(bb0, Statement::Drop(n, DUMMY_SPAN));
        // Use n after Drop → OK (Copy type, Drop=no-op)
        b.push(bb0, crate::assign(other, n));

        b.push(bb0, storage_dead(other));
        b.set_terminator(bb0, return_(vec![]));

        let body = b.build(bb0);
        println!("=== MIR (drop_copy) ===\n{body}");

        let result = check_body(&body);
        println!("=== BORROW CHECK (drop_copy) ===");
        for err in &result.errors {
            println!("  {err}");
        }

        assert!(
            result.is_ok(),
            "use-after-Drop of a Copy type MUST be allowed (Drop=no-op), got: {:?}",
            result.errors
        );
    }

    /// ADR-0054 T2: Return of a Move-type local after Drop is VALID (no E2421).
    /// Return-leniency carve-out: Ended set by Drop/StorageDead; Return can
    /// still consume it. This is why Ended was split from Moved originally.
    #[test]
    fn return_after_drop_must_be_allowed() {
        let mut b = MirBuilder::new("return_dropped", MirType::Unit);
        let s = b.add_param("s", ParameterPassing::Move);
        b.set_local_type(s, "String");

        let bb0 = b.new_block();
        // Drop s (Ended) then Return s — must be OK (no E2421, no E2420).
        b.push(bb0, Statement::Drop(s, DUMMY_SPAN));
        b.set_terminator(bb0, return_(vec![s]));

        let body = b.build(bb0);
        println!("=== MIR (return_after_drop) ===\n{body}");

        let result = check_body(&body);
        println!("=== BORROW CHECK (return_after_drop) ===");
        for err in &result.errors {
            println!("  {err}");
        }

        assert!(
            result.is_ok(),
            "Return of a Move-type local after Drop MUST be allowed (Return-leniency), got: {:?}",
            result.errors
        );
    }

    /// Field-level NLL: `&0 mutable obj.x` and `&0 mutable obj.y` are disjoint
    /// and may be held simultaneously. Both refs are kept live by passing them
    /// to a call, so both loans are active at the second borrow's creation.
    #[test]
    fn disjoint_field_borrows_accepted() {
        let mut b = MirBuilder::new("split", MirType::Unit);
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
        let mut b = MirBuilder::new("demo", MirType::Integer);
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
        let mut b = MirBuilder::new("clash", MirType::Unit);
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
        let mut cb = MirBuilder::new("get_cell", MirType::Struct("Cell".into()));
        cb.add_param("obj", ParameterPassing::MutableBorrow);
        cb.set_return_borrow(FieldPath::Root, vec![0]);
        let cbb = cb.new_block();
        cb.set_terminator(cbb, return_(vec![]));
        let callee = cb.build(cbb);
        let mut sigs: BTreeMap<String, FunctionSignature> = BTreeMap::new();
        sigs.insert("get_cell".to_string(), callee.signature.clone());

        // Caller: r1 = &mut obj; ret = get_cell(r1); r3 = &mut obj; use(ret, r3).
        let mut b = MirBuilder::new("caller", MirType::Unit);
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

    // ── WO-0075 (ADR-0070 §AMEND Phase 3): multi-level extraction ──
    // Shared layout: `struct Inner { x: String, y: String }`,
    // `struct Holder { inner: Inner, other: String }`. Used by teeth A-F.
    fn holder_layouts(b: &mut MirBuilder) {
        b.add_struct_layout(triet_mir::StructLayout::compute(
            "Inner",
            &[
                ("x".into(), MirType::String, 8, triet_mir::align::INTEGER),
                ("y".into(), MirType::String, 8, triet_mir::align::INTEGER),
            ],
        ));
        b.add_struct_layout(triet_mir::StructLayout::compute(
            "Holder",
            &[
                (
                    "inner".into(),
                    MirType::Struct("Inner".into()),
                    16,
                    triet_mir::align::POINTER,
                ),
                (
                    "other".into(),
                    MirType::String,
                    8,
                    triet_mir::align::INTEGER,
                ),
            ],
        ));
    }

    fn path_place(local: Local, parts: &[&str]) -> Place {
        Place {
            local,
            projection: parts
                .iter()
                .map(|p| Projection::Field((*p).to_string()))
                .collect(),
        }
    }

    fn has_uam(result: &BorrowCheckResult) -> bool {
        result
            .errors
            .iter()
            .any(|e| matches!(e, BorrowError::UseAfterMove { .. }))
    }

    /// Move `h.inner.x`, then run `read_stmts` and return whether a UAM fired.
    fn move_then(read_stmts: impl FnOnce(&mut MirBuilder, Local, BasicBlock)) -> BorrowCheckResult {
        let mut b = MirBuilder::new("ml", MirType::Unit);
        holder_layouts(&mut b);
        let h = b.add_param("h", ParameterPassing::Move);
        b.set_local_type(h, "Holder");
        let bb0 = b.new_block();
        let d1 = b.new_local();
        b.push(bb0, storage_live(d1));
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(d1),
                source: path_place(h, &["inner", "x"]),
                span: DUMMY_SPAN,
            },
        );
        read_stmts(&mut b, h, bb0);
        let body = b.build(bb0);
        check_body(&body)
    }

    /// Tooth A — sibling-live: move `h.inner.x`, read `h.inner.y` → NO UAM
    /// (sibling leaf stays live). Poison: base-only invalidate → false UAM.
    #[test]
    fn ml_tooth_a_sibling_live() {
        let r = move_then(|b, h, bb0| {
            let d2 = b.new_local();
            b.push(bb0, storage_live(d2));
            b.push(
                bb0,
                Statement::Assign {
                    dest: Place::local(d2),
                    source: path_place(h, &["inner", "y"]),
                    span: DUMMY_SPAN,
                },
            );
            b.set_terminator(bb0, return_(vec![]));
        });
        assert!(
            !has_uam(&r),
            "sibling field must stay live, got: {:?}",
            r.errors
        );
    }

    /// Tooth B — ancestor-dead: move `h.inner.x`, read `h.inner` → UAM.
    #[test]
    fn ml_tooth_b_ancestor_dead() {
        let r = move_then(|b, h, bb0| {
            let d2 = b.new_local();
            b.push(bb0, storage_live(d2));
            b.push(
                bb0,
                Statement::Assign {
                    dest: Place::local(d2),
                    source: path_place(h, &["inner"]),
                    span: DUMMY_SPAN,
                },
            );
            b.set_terminator(bb0, return_(vec![]));
        });
        assert!(
            has_uam(&r),
            "reading ancestor of a moved path is UAM, got: {:?}",
            r.errors
        );
    }

    /// Tooth C — exact-dead: move `h.inner.x`, read `h.inner.x` again → UAM.
    #[test]
    fn ml_tooth_c_exact_dead() {
        let r = move_then(|b, h, bb0| {
            let d2 = b.new_local();
            b.push(bb0, storage_live(d2));
            b.push(
                bb0,
                Statement::Assign {
                    dest: Place::local(d2),
                    source: path_place(h, &["inner", "x"]),
                    span: DUMMY_SPAN,
                },
            );
            b.set_terminator(bb0, return_(vec![]));
        });
        assert!(
            has_uam(&r),
            "re-reading the exact moved path is UAM, got: {:?}",
            r.errors
        );
    }

    /// Tooth D — whole-base-dead: move `h.inner.x`, borrow whole `h` → UAM.
    #[test]
    fn ml_tooth_d_whole_base_dead() {
        let r = move_then(|b, h, bb0| {
            let rref = b.new_local();
            b.push(bb0, storage_live(rref));
            b.push(bb0, borrow(rref, ReferenceForm::BorrowExclusiveMutable, h));
            b.set_terminator(bb0, return_(vec![]));
        });
        assert!(
            has_uam(&r),
            "whole-base use while a field is moved is UAM, got: {:?}",
            r.errors
        );
    }

    /// Tooth E — sibling-branch-live: move `h.inner.x`, read `h.other` → NO UAM.
    #[test]
    fn ml_tooth_e_sibling_branch_live() {
        let r = move_then(|b, h, bb0| {
            let d2 = b.new_local();
            b.push(bb0, storage_live(d2));
            b.push(
                bb0,
                Statement::Assign {
                    dest: Place::local(d2),
                    source: path_place(h, &["other"]),
                    span: DUMMY_SPAN,
                },
            );
            b.set_terminator(bb0, return_(vec![]));
        });
        assert!(
            !has_uam(&r),
            "a disjoint sibling field stays live, got: {:?}",
            r.errors
        );
    }

    /// Tooth F ⚔ — merge-union: move `h.inner.x` on ONE CFG branch, join, read
    /// `h.inner.x` at the confluence → UAM (union keeps the move). Poison the
    /// merge `union → intersection` → the move is forgotten at the join → no UAM.
    #[test]
    fn ml_tooth_f_merge_union() {
        let mut b = MirBuilder::new("ml_merge", MirType::Unit);
        holder_layouts(&mut b);
        let h = b.add_param("h", ParameterPassing::Move);
        b.set_local_type(h, "Holder");
        let cond = b.new_local();
        let d1 = b.new_local();
        let d2 = b.new_local();

        // entry: set cond, If → bb_move / bb_skip
        let entry = b.new_block();
        b.push(entry, storage_live(cond));
        b.push(entry, const_int(cond, 1));

        // bb_move: move h.inner.x; goto join
        let bb_move = b.new_block();
        b.push(bb_move, storage_live(d1));
        b.push(
            bb_move,
            Statement::Assign {
                dest: Place::local(d1),
                source: path_place(h, &["inner", "x"]),
                span: DUMMY_SPAN,
            },
        );

        // bb_skip: no move; goto join
        let bb_skip = b.new_block();

        // join: read h.inner.x → UAM (moved on one path)
        let join = b.new_block();
        b.push(join, storage_live(d2));
        b.push(
            join,
            Statement::Assign {
                dest: Place::local(d2),
                source: path_place(h, &["inner", "x"]),
                span: DUMMY_SPAN,
            },
        );
        b.set_terminator(join, return_(vec![]));

        b.set_terminator(bb_move, crate::goto(join));
        b.set_terminator(bb_skip, crate::goto(join));
        b.set_terminator(
            entry,
            Terminator::If {
                cond,
                positive_bb: bb_move,
                zero_bb: None,
                negative_bb: bb_skip,
                span: DUMMY_SPAN,
            },
        );

        let body = b.build(entry);
        let cfg = body.build_cfg();
        // Structural guard: the join MUST be a real confluence (≥2 preds).
        assert_eq!(
            cfg.blocks[join.0].predecessors.len(),
            2,
            "join must merge both branches (preds={:?})",
            cfg.blocks[join.0].predecessors
        );
        let r = check_body(&body);
        assert!(
            has_uam(&r),
            "a move on one branch must survive the union join, got: {:?}",
            r.errors
        );
    }

    /// E2423 tooth (preserved): a NON-Field projection extraction (an enum
    /// Payload) of a Move type is still REFUSED — `projection_path` returns None
    /// → the allow-arm falls through to E2423. Poison: widen the allow-arm to
    /// accept None → no error. (Replaces the obsolete `cannot_move_multilevel_
    /// field_out` test: multi-level Field extraction is now ALLOWED per ADR-0070
    /// §AMEND Phase 3; E2423 now guards non-Field projections.)
    #[test]
    fn cannot_move_non_field_projection_out() {
        let mut b = MirBuilder::new("extract_payload", MirType::Unit);
        b.add_enum_layout(triet_mir::EnumLayout::compute(
            "Box",
            &[(
                "S".into(),
                0,
                Some((MirType::String, 24, triet_mir::align::POINTER, vec![])),
            )],
        ));
        let e = b.add_param("e", ParameterPassing::Move);
        b.set_local_type(e, "Box");

        let bb0 = b.new_block();
        let dest = b.new_local();
        b.push(bb0, storage_live(dest));
        // Non-Field projection: enum Payload extraction of a Move (String) type.
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(dest),
                source: Place {
                    local: e,
                    projection: vec![Projection::Payload("S".to_string())],
                },
                span: DUMMY_SPAN,
            },
        );
        b.push(bb0, storage_dead(dest));
        b.set_terminator(bb0, return_(vec![]));

        let result = check_body(&b.build(bb0));
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::CannotCopyMoveTypeOut { .. })),
            "non-Field projection move-out must still be E2423, got: {:?}",
            result.errors
        );
    }

    /// neg tooth — sub-path reassign LOCKED (WO-0075 §F): move `h.inner.x`, then
    /// reassign the ANCESTOR `h.inner = ...` → E2424 (not a silent clear). Poison
    /// the §F guard (skip the diagnostic / clear anyway) → no E2424.
    #[test]
    fn ml_subpath_reassign_locked() {
        let r = move_then(|b, h, bb0| {
            let fresh = b.new_local();
            b.push(bb0, storage_live(fresh));
            // h.inner = fresh  (ancestor of the moved h.inner.x)
            b.push(
                bb0,
                Statement::Assign {
                    dest: path_place(h, &["inner"]),
                    source: Place::local(fresh),
                    span: DUMMY_SPAN,
                },
            );
            b.set_terminator(bb0, return_(vec![]));
        });
        assert!(
            r.errors
                .iter()
                .any(|e| matches!(e, BorrowError::SubPathReassignUnsupported { .. })),
            "ancestor reassign over a moved sub-path must be E2424, got: {:?}",
            r.errors
        );
    }

    /// ADR-0070 read-side: a SINGLE-level heap field move-out `dest = obj.body`
    /// is ALLOWED — it records a partial move instead of erroring. Poison the
    /// allow-arm (drop `is_any_heap()` from the guard) → this regresses to
    /// E2423 and the assertion fails = the allow-arm is load-bearing.
    #[test]
    fn single_level_heap_field_partial_moves_ok() {
        let mut b = MirBuilder::new("extract_string_field_ok", MirType::Unit);
        b.add_struct_layout(triet_mir::StructLayout::compute(
            "HasString",
            &[("body".into(), MirType::String, 8, triet_mir::align::INTEGER)],
        ));
        let obj = b.add_param("obj", ParameterPassing::Move);
        b.set_local_type(obj, "HasString");

        let bb0 = b.new_block();
        let dest = b.new_local();
        b.push(bb0, storage_live(dest));
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
        let result = check_body(&body);
        for err in &result.errors {
            println!("  {err}");
        }
        assert!(
            !result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::CannotCopyMoveTypeOut { .. })),
            "single-level heap field move-out must NOT emit E2423, got: {:?}",
            result.errors
        );
    }

    /// WO-0074 (Phase 3 — Nợ A): a SINGLE-level HEAP-CARRYING ENUM field
    /// move-out `dest = h.msg` is ALLOWED — it records a partial move instead
    /// of E2423 (the Site-2 allow-arm gained `matches!(extracted_ty,
    /// MirType::Enum(_))`). Poison: drop the Enum arm from the guard → this
    /// regresses to E2423 and the assertion fails = the enum allow-arm is
    /// load-bearing.
    #[test]
    fn single_level_enum_field_partial_moves_ok() {
        let mut b = MirBuilder::new("extract_enum_field_ok", MirType::Unit);
        // enum Msg { Text(String), Code(Integer) } — heap-carrying → non-copy.
        b.add_enum_layout(triet_mir::EnumLayout::compute(
            "Msg",
            &[
                (
                    "Text".into(),
                    0,
                    Some((MirType::String, 24, triet_mir::align::POINTER, vec![])),
                ),
                ("Code".into(), 1, None),
            ],
        ));
        // struct Holder { msg: Msg, n: Integer }
        b.add_struct_layout(triet_mir::StructLayout::compute(
            "Holder",
            &[
                (
                    "msg".into(),
                    MirType::Enum("Msg".into()),
                    32,
                    triet_mir::align::POINTER,
                ),
                ("n".into(), MirType::Integer, 8, triet_mir::align::INTEGER),
            ],
        ));
        let obj = b.add_param("obj", ParameterPassing::Move);
        b.set_local_type(obj, "Holder");

        let bb0 = b.new_block();
        let dest = b.new_local();
        b.push(bb0, storage_live(dest));
        b.push(
            bb0,
            Statement::Assign {
                dest: Place::local(dest),
                source: field(obj, "msg"),
                span: DUMMY_SPAN,
            },
        );
        b.push(bb0, storage_dead(dest));
        b.set_terminator(bb0, return_(vec![]));

        let body = b.build(bb0);
        let result = check_body(&body);
        for err in &result.errors {
            println!("  {err}");
        }
        assert!(
            !result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::CannotCopyMoveTypeOut { .. })),
            "single-level heap-carrying enum field move-out must NOT emit E2423, got: {:?}",
            result.errors
        );
    }

    /// WO-0075 Commit 1 tooth-G 🩸 (the deadliest): a field moved inside a LOOP
    /// body must propagate across the BACK-EDGE so the next iteration's re-read
    /// is a use-after-move. CFG:
    ///   entry → header
    ///   header: If cond → body / exit
    ///   body:  `s = move h.name`; StorageDead(s); goto header   (BACK-EDGE)
    ///   exit:  return
    /// `StorageDead(s)` neutralises the `var_states` delta so the ONLY thing
    /// that changes across the back-edge is `partial_moves[h] = {name}`. With the
    /// §4 fixpoint fix the header/body re-process and the move statement's own
    /// re-read on iteration 2 is caught (E2420). Poison: drop `partial_moves`
    /// from the fixpoint convergence test → the delta is invisible → the loop
    /// converges before re-processing → NO error → this assertion fails (RED).
    #[test]
    fn fixpoint_loop_partial_move_propagates_back_edge() {
        let mut b = MirBuilder::new("loop_move", MirType::Unit);
        b.add_struct_layout(triet_mir::StructLayout::compute(
            "HasString",
            &[("name".into(), MirType::String, 8, triet_mir::align::INTEGER)],
        ));
        let h = b.add_param("h", ParameterPassing::Move);
        b.set_local_type(h, "HasString");
        let cond = b.new_local();
        let s = b.new_local();

        // entry: set up cond, goto header
        let entry = b.new_block();
        b.push(entry, storage_live(cond));
        b.push(entry, const_int(cond, 1));

        // header: If cond → body / exit
        let header = b.new_block();

        // body: s = move h.name; StorageDead(s); goto header (BACK-EDGE)
        let body = b.new_block();
        b.push(body, storage_live(s));
        b.push(
            body,
            Statement::Assign {
                dest: Place::local(s),
                source: field(h, "name"),
                span: DUMMY_SPAN,
            },
        );
        b.push(body, storage_dead(s));
        b.set_terminator(body, crate::goto(header));

        // exit: return
        let exit = b.new_block();
        b.set_terminator(exit, return_(vec![]));

        b.set_terminator(entry, crate::goto(header));
        b.set_terminator(
            header,
            Terminator::If {
                cond,
                positive_bb: body,
                zero_bb: None,
                negative_bb: exit,
                span: DUMMY_SPAN,
            },
        );

        let body_mir = b.build(entry);
        let cfg = body_mir.build_cfg();
        // Structural guard: the header MUST have a real back-edge (≥2 preds:
        // entry + body) and the body's successor is the header — a straight-line
        // forgery would converge in one pass and prove nothing.
        assert!(
            cfg.blocks[header.0].predecessors.contains(&body),
            "test is not a loop: header has no back-edge from body (preds={:?})",
            cfg.blocks[header.0].predecessors
        );
        assert_eq!(
            cfg.blocks[body.0].successors,
            vec![header],
            "body must jump back to the header (back-edge)"
        );

        let result = check_body(&body_mir);
        for err in &result.errors {
            println!("  {err}");
        }
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::UseAfterMove { .. })),
            "loop-carried partial move must be a use-after-move on iteration 2; \
             got: {:?}",
            result.errors
        );
    }

    /// F1 chain: Move type assign → Drop → Return must trigger E2420.
    /// The Moved state must be sticky through Drop so that Return sees it.
    #[test]
    fn move_through_drop_to_return_rejected() {
        let mut b = MirBuilder::new("f1_chain", MirType::Unit);
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
        let mut b = MirBuilder::new("f1_enum_payload", MirType::Unit);
        b.add_enum_layout(triet_mir::EnumLayout::compute(
            "OptionString",
            &[("Some".into(), 0, Some((MirType::String, 8, 8, vec![])))],
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
        let mut b = MirBuilder::new("e2450_heap", MirType::Unit);
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
        let mut b = MirBuilder::new("caller", MirType::Unit);
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
        let mut b = MirBuilder::new("caller", MirType::Unit);
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
        let mut b = MirBuilder::new("test", MirType::Unit);
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
        let mut b = MirBuilder::new("test", MirType::Unit);
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

    // ═══════════════════════════════════════════════════════════════════
    // ADR-0079 Slice A — get-borrow borrowck teeth (hand-built MIR)
    // ═══════════════════════════════════════════════════════════════════
    //
    // These tests verify the borrowck machinery BEFORE typecheck surface
    // opens (mirroring HashMap P1a Slice A approach). The shim names used
    // here exist in `builtin_shim_meta` but the JIT shims are NOT yet wired
    // — that is Slice B. The borrowck only reads the metadata table.

    /// Helper: emit a `CallDispatch` terminator targeting a **builtin shim**
    /// (not a user function). Uses `CallTarget::Shim` so the JIT path
    /// distinguishes it, but `builtin_shim_meta` works on callee_name alone.
    fn shim_call(
        name: &str,
        args: Vec<Local>,
        return_bb: BasicBlock,
        dest: Vec<Local>,
    ) -> Terminator {
        Terminator::CallDispatch {
            callee: FunctionId(0),
            callee_name: name.to_string(),
            target: CallTarget::Shim,
            args,
            return_bb,
            dest,
            return_shape: ReturnShape::Scalar,
            span: DUMMY_SPAN,
        }
    }

    /// ADR-0079 U2: `get_ref(&0 m, k)` creates a PropagatedLoan on `m`.
    /// Drop `m` while `r` (the returned reference) is still live → E2450.
    /// The propagated loan dest (`r`) is live_out of bb1 into bb2 where it
    /// is used (the live_out check fires the DropWhileBorrowed).
    #[test]
    fn get_ref_borrow_then_drop_container_e2450() {
        let mut b = MirBuilder::new("get_ref_drop", MirType::Integer);
        let m = b.add_param("m", ParameterPassing::Move);
        b.set_local_type(m, "HashMap<Integer,String>");
        let r = b.new_local();
        let k = b.new_local();

        // bb0: r = get_ref(&0 m, k)  → loan created on m
        let bb0 = b.new_block();
        b.push(bb0, storage_live(r));
        b.push(bb0, storage_live(k));
        b.push(bb0, const_int(k, 1));
        let bb1 = b.new_block();
        b.set_terminator(
            bb0,
            shim_call("__triet_hashmap_get_ref", vec![m, k], bb1, vec![r]),
        );

        // bb1: Drop(m) — r is live_out of bb1 (used in bb2) → E2450
        b.push(bb1, Statement::Drop(m, DUMMY_SPAN));
        let bb2 = b.new_block();
        b.set_terminator(
            bb1,
            Terminator::Goto {
                target: bb2,
                span: DUMMY_SPAN,
            },
        );

        // bb2: use r (return it) → proves r was live after m dropped
        b.push(bb2, storage_dead(k));
        b.set_terminator(bb2, return_(vec![r]));

        let body = b.build(bb0);
        println!("=== get_ref → drop container → use ref ===");
        println!("{body}");
        let result = check_body(&body);
        for err in &result.errors {
            println!("  {err}");
        }
        assert!(
            !result.is_ok(),
            "E2450 must fire when dropping container while get_ref borrow is alive"
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

    /// ADR-0079 U3: `insert` CONSUMES the map handle (arg_consumes[0]=true).
    /// If `get_ref(&0 m, k)` created a loan on `m`, then `insert(m, …)` must
    /// fire E2440 — consuming the container while it is borrowed invalidates
    /// the reference.
    #[test]
    fn get_ref_borrow_then_insert_e2440() {
        let mut b = MirBuilder::new("get_ref_insert", MirType::Unit);
        let m = b.add_param("m", ParameterPassing::Move);
        b.set_local_type(m, "HashMap<Integer,String>");
        let r = b.new_local();
        let k1 = b.new_local();
        let k2 = b.new_local();
        let v = b.new_local();

        // bb0: r = get_ref(&0 m, k1)  → loan created on m
        let bb0 = b.new_block();
        b.push(bb0, storage_live(r));
        b.push(bb0, storage_live(k1));
        b.push(bb0, const_int(k1, 1));
        let bb1 = b.new_block();
        b.set_terminator(
            bb0,
            shim_call("__triet_hashmap_get_ref", vec![m, k1], bb1, vec![r]),
        );

        // bb1: insert(m, k2, v) — m is consumed (arg_consumes[0]=true)
        // while r (which borrows m) is still live → E2440
        let m2 = b.new_local();
        b.set_local_type(m2, "HashMap<Integer,String>");
        b.push(bb1, storage_live(m2));
        b.push(bb1, storage_live(k2));
        b.push(bb1, const_int(k2, 2));
        b.push(bb1, storage_live(v));
        b.push(bb1, const_int(v, 42));
        let bb2 = b.new_block();
        b.set_terminator(
            bb1,
            shim_call("__triet_hashmap_insert", vec![m, k2, v], bb2, vec![m2]),
        );

        // bb2: cleanup
        b.push(bb2, Statement::Drop(r, DUMMY_SPAN));
        b.push(bb2, Statement::Drop(m2, DUMMY_SPAN));
        b.push(bb2, storage_dead(k1));
        b.push(bb2, storage_dead(k2));
        b.set_terminator(bb2, return_(vec![]));

        let body = b.build(bb0);
        println!("=== get_ref → insert ===");
        println!("{body}");
        let result = check_body(&body);
        for err in &result.errors {
            println!("  {err}");
        }
        assert!(
            !result.is_ok(),
            "E2440 must fire when consuming (insert) container while get_ref borrow is alive"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::NllExclusivityViolation { .. })),
            "expected E2440 NllExclusivityViolation, got: {:?}",
            result.errors
        );
    }

    /// Negative control: `get_ref` then `Drop(r)` THEN `Drop(m)` —
    /// no error because the loan is dead before the container drops.
    #[test]
    fn get_ref_drop_ref_then_drop_container_no_error() {
        let mut b = MirBuilder::new("get_ref_clean", MirType::Unit);
        let m = b.add_param("m", ParameterPassing::Move);
        b.set_local_type(m, "HashMap<Integer,String>");
        let r = b.new_local();
        let k = b.new_local();

        // bb0: r = get_ref(&0 m, k)  → loan created on m
        let bb0 = b.new_block();
        b.push(bb0, storage_live(r));
        b.push(bb0, storage_live(k));
        b.push(bb0, const_int(k, 1));
        let bb1 = b.new_block();
        b.set_terminator(
            bb0,
            shim_call("__triet_hashmap_get_ref", vec![m, k], bb1, vec![r]),
        );

        // bb1: Drop(r) FIRST (loan ends) → THEN Drop(m) — no error
        b.push(bb1, Statement::Drop(r, DUMMY_SPAN));
        b.push(bb1, Statement::Drop(m, DUMMY_SPAN));
        b.push(bb1, storage_dead(k));
        b.set_terminator(bb1, return_(vec![]));

        let body = b.build(bb0);
        println!("=== get_ref → drop ref → drop container ===");
        println!("{body}");
        let result = check_body(&body);
        for err in &result.errors {
            println!("  {err}");
        }
        assert!(
            result.is_ok(),
            "no error when drop ref before container, got: {:?}",
            result.errors
        );
    }

    /// ADR-0079 U3: `remove` MUTATES the map in-place (mutates_arg=Some(0)).
    /// If `get_ref(&0 m, k)` created a loan on `m`, then `remove(m, k2)`
    /// must fire E2440 — mutating the container while borrowed invalidates
    /// the reference (even for a different key — whole-container rule).
    #[test]
    fn get_ref_borrow_then_remove_e2440() {
        let mut b = MirBuilder::new("get_ref_remove", MirType::Unit);
        let m = b.add_param("m", ParameterPassing::Move);
        b.set_local_type(m, "HashMap<Integer,String>");
        let r = b.new_local();
        let k1 = b.new_local();
        let k2 = b.new_local();

        // bb0: r = get_ref(&0 m, k1) → loan created on m
        let bb0 = b.new_block();
        b.push(bb0, storage_live(r));
        b.push(bb0, storage_live(k1));
        b.push(bb0, const_int(k1, 1));
        let bb1 = b.new_block();
        b.set_terminator(
            bb0,
            shim_call("__triet_hashmap_get_ref", vec![m, k1], bb1, vec![r]),
        );

        // bb1: remove(m, k2) — m is mutated in-place (mutates_arg[0])
        // while r (which borrows m) is still live → E2440
        let out = b.new_local();
        b.push(bb1, storage_live(out));
        b.push(bb1, storage_live(k2));
        b.push(bb1, const_int(k2, 2));
        let bb2 = b.new_block();
        b.set_terminator(
            bb1,
            shim_call("__triet_hashmap_remove", vec![m, k2], bb2, vec![out]),
        );

        // bb2: cleanup
        b.push(bb2, Statement::Drop(r, DUMMY_SPAN));
        b.push(bb2, Statement::Drop(out, DUMMY_SPAN));
        b.push(bb2, Statement::Drop(m, DUMMY_SPAN));
        b.push(bb2, storage_dead(k1));
        b.push(bb2, storage_dead(k2));
        b.set_terminator(bb2, return_(vec![]));

        let body = b.build(bb0);
        println!("=== get_ref → remove ===");
        println!("{body}");
        let result = check_body(&body);
        for err in &result.errors {
            println!("  {err}");
        }
        assert!(
            !result.is_ok(),
            "E2440 must fire when mutating (remove) container while get_ref borrow is alive"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::NllExclusivityViolation { .. })),
            "expected E2440 NllExclusivityViolation, got: {:?}",
            result.errors
        );
    }

    /// ADR-0079 U3: `pop` MUTATES the vector in-place (mutates_arg=Some(0)).
    /// If `get_ref(&0 v, i)` created a loan on `v`, then `pop(v)` must fire
    /// E2440 — mutating while borrowed invalidates the reference.
    #[test]
    fn get_ref_borrow_then_pop_e2440() {
        let mut b = MirBuilder::new("get_ref_pop", MirType::Unit);
        let v = b.add_param("v", ParameterPassing::Move);
        b.set_local_type(v, "Vector<String>");
        let r = b.new_local();
        let i = b.new_local();

        // bb0: r = get_ref(&0 v, i) → loan created on v
        let bb0 = b.new_block();
        b.push(bb0, storage_live(r));
        b.push(bb0, storage_live(i));
        b.push(bb0, const_int(i, 0));
        let bb1 = b.new_block();
        b.set_terminator(
            bb0,
            shim_call("__triet_vector_get_ref", vec![v, i], bb1, vec![r]),
        );

        // bb1: pop(v) — v is mutated in-place (mutates_arg[0])
        // while r is still live → E2440
        let out = b.new_local();
        b.push(bb1, storage_live(out));
        let bb2 = b.new_block();
        b.set_terminator(
            bb1,
            shim_call("__triet_vector_pop", vec![v], bb2, vec![out]),
        );

        // bb2: cleanup
        b.push(bb2, Statement::Drop(r, DUMMY_SPAN));
        b.push(bb2, Statement::Drop(out, DUMMY_SPAN));
        b.push(bb2, Statement::Drop(v, DUMMY_SPAN));
        b.push(bb2, storage_dead(i));
        b.set_terminator(bb2, return_(vec![]));

        let body = b.build(bb0);
        println!("=== get_ref → pop ===");
        println!("{body}");
        let result = check_body(&body);
        for err in &result.errors {
            println!("  {err}");
        }
        assert!(
            !result.is_ok(),
            "E2440 must fire when mutating (pop) vector while get_ref borrow is alive"
        );
        assert!(
            result
                .errors
                .iter()
                .any(|e| matches!(e, BorrowError::NllExclusivityViolation { .. })),
            "expected E2440 NllExclusivityViolation, got: {:?}",
            result.errors
        );
    }
}

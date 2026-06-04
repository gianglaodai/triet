//! v0.10.x.borrow.1 — NLL borrow-exclusivity enforcement per
//! [ADR-0025] §2 (E2440 `BorrowExclusivityViolation`).
//!
//! Implements Non-Lexical Lifetime live-range analysis on a linearized
//! CFG built from the function's AST. The algorithm runs three passes
//! per function:
//!
//! 1. **Collect** — walk the AST in execution order, emitting a flat
//!    sequence of [`Event`]s: `BorrowCreate` when a `let r = &FORM
//!    base` binding is introduced; `Use` when an identifier holding a
//!    tracked borrow is read; loop-boundary markers so the live-range
//!    extension semantics apply to bindings created outside a loop
//!    but used inside it.
//!
//! 2. **Live-range** — for each `BorrowCreate` event, scan forward for
//!    the maximum `Use` index for that binding. The live range is
//!    `[create_seq, last_use_seq]` (closed interval). Bindings whose
//!    last use sits inside a loop body that came AFTER their creation
//!    point extend the `last_use` to the loop-end marker (conservative
//!    correctness — see "loop conservatism" note below).
//!
//! 3. **Conflict detect** — for each pair of borrows on the same base
//!    identifier, check whether forms conflict (per [ADR-0025] §2.1
//!    table) AND live ranges overlap. Both true → emit E2440 with both
//!    creation spans labeled.
//!
//! **Branch isolation:** `if-else` and `match` arms are serialized in
//! the event stream (each arm contributes its own events sequentially).
//! Borrows local to one arm have their live-range bounded within that
//! arm; sibling-arm conflicts therefore never trigger E2440. Borrows
//! created BEFORE a branch and used inside it correctly extend the
//! live-range across the branch sites.
//!
//! **Conservative scopes (refuse-over-guess, defer v0.11):**
//! - **Base = root identifier:** `&FORM obj.field` collapses to base
//!   `obj`. Two borrows on different fields of the same object are
//!   treated as conflicting; field-granular tracking defers v0.11+.
//! - **Function-call arg passing:** an identifier appearing as a call
//!   argument is counted as a `Use`. Inter-procedural live-range
//!   analysis (callee may borrow further) defers v0.11+.
//! - **Closures:** captures inside lambdas are not currently traced;
//!   defer v0.11+.
//! - **Self-host port (Layer B):** `compiler/typecheck.tri` mirror
//!   defers per ADR-0029 §3 (Layer B internal compiler, defer-OK).
//!
//! [ADR-0025]: ../../../docs/decisions/0025-borrow-checker-rules.md
//! [ADR-0029 §3]: ../../../docs/decisions/0029-self-host-port-policy.md

use triet_syntax::{
    Arena, Expr, ExprId, FunctionBody, FunctionDef, ReferenceForm, Span, Spanned, Stmt, StmtId,
};

use crate::error::{BorrowError, TypeError};

/// One event in the linearized execution stream.
#[derive(Debug, Clone)]
struct Event {
    /// Monotonic sequence number assigned during collection.
    seq: usize,
    /// Source span covering the event location.
    span: Span,
    /// Event payload.
    kind: EventKind,
}

#[derive(Debug, Clone)]
enum EventKind {
    /// `let r = &FORM base ...` — binding `r` enters live state.
    BorrowCreate {
        /// Name of the let binding holding the borrow.
        binding: String,
        /// Root identifier the borrow is rooted at (operand grammar
        /// per ADR-0031 §2 — IDENT + field-access; we collapse field-
        /// access chains to the root identifier).
        base: String,
        /// 5-form reference form per ADR-0022 §2.
        form: ReferenceForm,
    },
    /// Identifier read of a tracked borrow binding.
    Use(String),
    /// Marker emitted at loop-body entry. Used by the live-range pass
    /// to know that any `Use` events between [`LoopEnter`] and the
    /// matching [`LoopExit`] should extend live-ranges of bindings
    /// created BEFORE the loop, to the `LoopExit` event (conservative
    /// approximation of fixed-point semantics — see Pass 2 logic).
    LoopEnter,
    /// Marker emitted at loop-body exit.
    LoopExit,
}

/// Collector state — accumulates events during AST walk.
struct Collector<'a> {
    arena: &'a Arena,
    events: Vec<Event>,
    /// Set of binding names introduced by `BorrowCreate` events so far.
    /// Used to filter `Use` events: only identifier reads of tracked
    /// borrow bindings matter; reads of plain values are ignored.
    tracked: std::collections::HashSet<String>,
}

impl<'a> Collector<'a> {
    fn new(arena: &'a Arena) -> Self {
        Self {
            arena,
            events: Vec::new(),
            tracked: std::collections::HashSet::new(),
        }
    }

    fn push(&mut self, span: Span, kind: EventKind) {
        let seq = self.events.len();
        self.events.push(Event { seq, span, kind });
    }

    /// Walk an expression for `Use` events. Identifiers reading a
    /// tracked borrow binding contribute one event. Nested expressions
    /// are walked left-to-right, depth-first.
    #[allow(clippy::too_many_lines)]
    fn walk_expr(&mut self, expr_id: ExprId) {
        let Spanned { node, span } = self.arena.expression(expr_id).clone();
        match node {
            // Identifier read — if it's a tracked borrow binding, log Use.
            Expr::Identifier { name } if self.tracked.contains(&name) => {
                self.push(span, EventKind::Use(name));
            }
            Expr::Identifier { .. } => {}
            // Walk child expressions.
            Expr::FieldAccess { object, .. } => self.walk_expr(object),
            Expr::TupleIndex { tuple, .. } => self.walk_expr(tuple),
            Expr::Call { callee, arguments } => {
                self.walk_expr(callee);
                for arg in arguments {
                    self.walk_expr(arg);
                }
            }
            Expr::MethodCall {
                receiver,
                arguments,
                ..
            } => {
                self.walk_expr(receiver);
                for arg in arguments {
                    self.walk_expr(arg);
                }
            }
            Expr::BinaryOp { left, right, .. } => {
                self.walk_expr(left);
                self.walk_expr(right);
            }
            Expr::UnaryOp { operand, .. } => self.walk_expr(operand),
            Expr::Borrow { operand, .. } => {
                // Operand of a borrow expression — its base identifier
                // becomes the "use" subject ONLY if the borrow is being
                // used in an rvalue position (e.g., as a function arg).
                // When this borrow is the RHS of a `let`, the let-stmt
                // walker handles the BorrowCreate event directly; the
                // operand walk just records the underlying identifier
                // as a candidate use (it's the borrow-source, not a
                // tracked borrow itself).
                self.walk_expr(operand);
            }
            Expr::SafeFieldAccess { object, .. } => self.walk_expr(object),
            Expr::SafeMethodCall {
                receiver,
                arguments,
                ..
            } => {
                self.walk_expr(receiver);
                for arg in arguments {
                    self.walk_expr(arg);
                }
            }
            Expr::ElvisOp { object, default } => {
                self.walk_expr(object);
                self.walk_expr(default);
            }
            Expr::ForceUnwrap { operand: inner } => self.walk_expr(inner),
            // ── Branches — serialized in event stream ────────────
            Expr::If {
                condition,
                then_branch,
                else_branch,
                ..
            } => {
                self.walk_expr(condition);
                self.walk_expr(then_branch);
                if let Some(else_block) = else_branch {
                    self.walk_expr(else_block);
                }
            }
            Expr::Match { scrutinee, arms } => {
                self.walk_expr(scrutinee);
                for arm in arms {
                    if let Some(guard) = arm.guard {
                        self.walk_expr(guard);
                    }
                    self.walk_expr(arm.body);
                }
            }
            Expr::Block {
                statements,
                final_expr,
            } => self.walk_block(&statements, final_expr),
            // Literals + Outcome constructors + other leaf forms don't
            // produce use events; skip.
            _ => {}
        }
    }

    fn walk_block(&mut self, statements: &[StmtId], final_expression: Option<ExprId>) {
        for stmt_id in statements {
            self.walk_stmt(*stmt_id);
        }
        if let Some(final_expr) = final_expression {
            self.walk_expr(final_expr);
        }
    }

    fn walk_stmt(&mut self, stmt_id: StmtId) {
        let stmt = self.arena.statement(stmt_id).clone();
        match stmt.node {
            Stmt::Let { name, init, .. } => {
                // First walk the RHS for use events (e.g., the operand
                // of a borrow might reference another tracked binding).
                self.walk_expr(init);
                // Then check whether the RHS is a borrow expression —
                // if so, the let-binding becomes a tracked borrow.
                if let Expr::Borrow { form, operand } = &self.arena.expression(init).node
                    && let Some(base) = extract_base_identifier(self.arena, *operand)
                {
                    self.tracked.insert(name.clone());
                    self.push(
                        stmt.span.clone(),
                        EventKind::BorrowCreate {
                            binding: name,
                            base,
                            form: *form,
                        },
                    );
                }
            }
            Stmt::Assignment { value, .. } => {
                // The target name is implicit; we don't track re-assign
                // semantics for borrow lifetimes at v0.10 scope.
                // Re-assignment to a `let mutable r: &0 X = ...` is
                // possible but rare; conservatively, the new RHS is
                // walked for use events.
                self.walk_expr(value);
            }
            Stmt::Const { value, .. } => self.walk_expr(value),
            Stmt::Return { value } => {
                if let Some(id) = value {
                    self.walk_expr(id);
                }
            }
            Stmt::Break => {
                // Unit variant — break-with-value is not modeled.
            }
            Stmt::Continue => {}
            Stmt::For { iterable, body, .. } => {
                self.walk_expr(iterable);
                // For-loop body — same loop semantics as while.
                self.push(stmt.span.clone(), EventKind::LoopEnter);
                self.walk_expr(body);
                self.push(stmt.span, EventKind::LoopExit);
            }
            Stmt::While {
                condition, body, ..
            } => {
                self.walk_expr(condition);
                self.push(stmt.span.clone(), EventKind::LoopEnter);
                self.walk_expr(body);
                self.push(stmt.span, EventKind::LoopExit);
            }
            Stmt::Loop { body } => {
                self.push(stmt.span.clone(), EventKind::LoopEnter);
                self.walk_expr(body);
                self.push(stmt.span, EventKind::LoopExit);
            }
            Stmt::Expression { expr } => self.walk_expr(expr),
        }
    }
}

/// Recursively collapse a borrow-operand expression (per ADR-0031 §2:
/// IDENT or field-access chain) to its root identifier. Returns `None`
/// for unsupported operand shapes (per refuse-over-guess; ADR-0031's
/// operand grammar restricts to these two cases at v0.9, so this
/// covers all parseable borrow operands).
fn extract_base_identifier(arena: &Arena, expr_id: ExprId) -> Option<String> {
    match &arena.expression(expr_id).node {
        Expr::Identifier { name } => Some(name.clone()),
        Expr::FieldAccess { object, .. } => extract_base_identifier(arena, *object),
        _ => None,
    }
}

/// Computed live-range info for one borrow.
///
/// `binding` is informational — kept for future error-message
/// refinement that names the binding (e.g., "cannot create `m2`
/// while `m1` still live") rather than just the base. v0.10 message
/// shape uses base + forms only; binding annotated `#[allow]` to
/// document the future-use intent.
#[derive(Debug, Clone)]
struct BorrowSummary {
    #[allow(dead_code)]
    binding: String,
    base: String,
    form: ReferenceForm,
    create_seq: usize,
    create_span: Span,
    last_use_seq: usize,
}

/// Pass 2 — compute live-range for each `BorrowCreate` event.
///
/// For each borrow, scan forward through the event stream looking for
/// `Use(binding_name)` events. Track loop-marker depth: if a use
/// occurs INSIDE a loop body that started AFTER the borrow's creation,
/// the `last_use` is extended to the matching `LoopExit` event seq
/// (conservative approximation of fixed-point reuse across iterations).
fn compute_live_ranges(events: &[Event]) -> Vec<BorrowSummary> {
    let mut summaries: Vec<BorrowSummary> = Vec::new();

    for (idx, event) in events.iter().enumerate() {
        if let EventKind::BorrowCreate {
            binding,
            base,
            form,
        } = &event.kind
        {
            // Find the last seq where this binding is used. Track loop
            // nesting that opened AFTER the create_seq — if a use is
            // inside such a loop, extend last_use to the loop's exit.
            let mut last_use_seq = event.seq;
            // Stack of LoopEnter seqs that opened AFTER our create.
            let mut active_loop_starts: Vec<usize> = Vec::new();
            for later in events.iter().skip(idx + 1) {
                match &later.kind {
                    EventKind::LoopEnter => active_loop_starts.push(later.seq),
                    EventKind::LoopExit => {
                        active_loop_starts.pop();
                    }
                    EventKind::Use(name) if name == binding => {
                        if active_loop_starts.is_empty() {
                            last_use_seq = last_use_seq.max(later.seq);
                        } else {
                            // Use inside a loop body that opened after
                            // our create. Extend last_use to the
                            // OUTERMOST such loop's exit — that's the
                            // soonest the borrow's logical liveness
                            // ends (its value may be re-read on later
                            // iterations).
                            let outermost_loop_start = active_loop_starts[0];
                            // Find that loop's matching LoopExit seq.
                            let exit_seq = find_matching_loop_exit(events, outermost_loop_start);
                            last_use_seq = last_use_seq.max(exit_seq);
                        }
                    }
                    EventKind::Use(_) | EventKind::BorrowCreate { .. } => {}
                }
            }
            summaries.push(BorrowSummary {
                binding: binding.clone(),
                base: base.clone(),
                form: *form,
                create_seq: event.seq,
                create_span: event.span.clone(),
                last_use_seq,
            });
        }
    }

    summaries
}

/// Given a `LoopEnter` at `start_seq`, find the matching `LoopExit`
/// (counting nested loops). Falls back to the highest seq in the
/// stream if no matching exit is found (defensive — should not happen
/// because the collector emits balanced markers).
fn find_matching_loop_exit(events: &[Event], start_seq: usize) -> usize {
    let mut depth = 0;
    for event in events {
        if event.seq <= start_seq {
            continue;
        }
        match event.kind {
            EventKind::LoopEnter => depth += 1,
            EventKind::LoopExit => {
                if depth == 0 {
                    return event.seq;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    events.last().map_or(start_seq, |e| e.seq)
}

/// Two borrows on the same base conflict per [ADR-0025] §2.1.
///
/// [ADR-0025]: ../../../docs/decisions/0025-borrow-checker-rules.md
const fn forms_conflict(a: ReferenceForm, b: ReferenceForm) -> bool {
    use ReferenceForm::{BorrowExclusiveMutable, BorrowReadOnly, WeakObserver};
    match (a, b) {
        // Weak observer never excludes — shared by convention.
        (WeakObserver, _) | (_, WeakObserver) => false,
        // Two read-only borrows coexist.
        (BorrowReadOnly, BorrowReadOnly) => false,
        // Anything else among scope borrows conflicts:
        //   &0 + &0 mutable        → exclusive vs shared
        //   &0 mutable + &0 mutable → two exclusive
        //   &0 mutable + &0        → exclusive vs shared
        (BorrowReadOnly | BorrowExclusiveMutable, BorrowReadOnly | BorrowExclusiveMutable) => true,
        // `&+` (owning forms) handled by E2420 UseAfterMove — not
        // reached here because owning-form bindings aren't `tracked`
        // in the collector (BorrowCreate only fires for borrow forms).
        _ => false,
    }
}

/// Pass 3 — pair-wise conflict detection. Returns one `E2440` per
/// conflicting pair; the second borrow's creation span carries the
/// primary label, with the first borrow's span included in the
/// diagnostic body via the `[Fix N]` block context.
fn detect_conflicts(summaries: &[BorrowSummary], errors: &mut Vec<TypeError>) {
    let mut reported_pairs: std::collections::HashSet<(usize, usize)> =
        std::collections::HashSet::new();
    for i in 0..summaries.len() {
        for j in (i + 1)..summaries.len() {
            let a = &summaries[i];
            let b = &summaries[j];
            if a.base != b.base {
                continue;
            }
            if !forms_conflict(a.form, b.form) {
                continue;
            }
            // Ranges overlap iff max(start) <= min(end).
            let overlap_start = a.create_seq.max(b.create_seq);
            let overlap_end = a.last_use_seq.min(b.last_use_seq);
            if overlap_start > overlap_end {
                continue;
            }
            // Avoid duplicate report (same pair detected twice).
            let key = (
                a.create_seq.min(b.create_seq),
                a.create_seq.max(b.create_seq),
            );
            if !reported_pairs.insert(key) {
                continue;
            }
            // The "second" borrow (later in execution order) gets the
            // primary span. ADR-0027 diagnostic format.
            let (first, second) = if a.create_seq <= b.create_seq {
                (a, b)
            } else {
                (b, a)
            };
            errors.push(TypeError::Borrow(BorrowError::BorrowExclusivityViolation {
                base: first.base.clone(),
                first_form: form_label(first.form),
                second_form: form_label(second.form),
                first_span: first.create_span.clone(),
                span: second.create_span.clone(),
            }));
        }
    }
}

fn form_label(form: ReferenceForm) -> String {
    match form {
        ReferenceForm::StrongFrozen => "&+".to_owned(),
        ReferenceForm::StrongMutable => "&+ mutable".to_owned(),
        ReferenceForm::BorrowReadOnly => "&0".to_owned(),
        ReferenceForm::BorrowExclusiveMutable => "&0 mutable".to_owned(),
        ReferenceForm::WeakObserver => "&-".to_owned(),
    }
}

/// v0.10.x.borrow.1 entry point — analyze a function body for NLL
/// borrow-exclusivity violations. Appends any detected `E2440`s onto
/// the caller's error list; never panics.
pub(crate) fn analyze_function(arena: &Arena, def: &FunctionDef, errors: &mut Vec<TypeError>) {
    // Parameter declarations are not borrow-create sites in
    // themselves; parameters typed as `&0 X` etc. ARE references but
    // they were borrowed at the caller's site, not here. The
    // parameter-vs-local-borrow case (caller-borrowed param +
    // locally-created conflicting borrow on the same base) is OUT
    // OF SCOPE for v0.10.x.borrow.1 — defer v0.11+ corpus-driven.
    let mut collector = Collector::new(arena);
    match &def.body {
        FunctionBody::Block { block } => collector.walk_expr(*block),
        FunctionBody::Expression { expr } => collector.walk_expr(*expr),
        FunctionBody::External { .. } => {}
    }

    let summaries = compute_live_ranges(&collector.events);
    detect_conflicts(&summaries, errors);
}

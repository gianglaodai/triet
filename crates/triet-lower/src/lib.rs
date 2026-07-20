//! Triết AST → MIR lowering.
//!
//! Takes a parsed `triet_syntax::Program` and produces MIR `triet_mir::Body`
//! for each function. The lowerer flattens the nested, typed AST into a flat
//! control-flow graph of basic blocks holding simple statements over
//! `Place`s (a base local refined by field/deref projections).
//!
//! Scope (v0 milestone): scalars, `let`, binary ops, `if`, `while`, calls,
//! borrows, and `obj.field` access (lowered to a `Field` projection).
//! Unsupported constructs return `Err(LowerError)` — the lowerer never panics
//! on user input.

#![warn(missing_docs)]

use std::collections::{BTreeMap, BTreeSet, HashMap};

use triet_mir::{
    BasicBlock, BinOp, Body, CallTarget, ConstValue, DUMMY_SPAN, EnumLayout, FieldLayout,
    FieldPath, FunctionSignature, Local, LocalDecl, MirType, ParameterPassing, Place, Projection,
    Span, Statement, StructLayout, Terminator,
};
use triet_syntax::{
    Arena, BinaryOperator, CapabilityLevel, Expr, ExprId, FunctionBody, FunctionDefinition, Item,
    MethodResolutions, PatternResolutions, Program, ReferenceForm, Stmt, TypeExpr, TypeId,
    UnaryOperator,
};

// ── Lowering error ───────────────────────────────────────────

/// An error produced when lowering cannot proceed because an AST construct
/// is not yet supported by the MIR backend.
#[derive(Debug, Clone)]
pub struct LowerError {
    /// Human-readable description of what was not supported.
    pub message: String,
    /// Source location of the unsupported construct.
    pub span: Span,
}

impl LowerError {
    fn unsupported_stmt(stmt: &Stmt, span: Span) -> Self {
        Self {
            message: format!("lowerer does not yet support this statement: {stmt:?}"),
            span,
        }
    }

    fn unsupported_expr(expr: &Expr, span: Span) -> Self {
        Self {
            message: format!("lowerer does not yet support this expression: {expr:?}"),
            span,
        }
    }

    fn unsupported_callee(expr: &Expr, span: Span) -> Self {
        Self {
            message: format!("unsupported callee expression: {expr:?}"),
            span,
        }
    }

    fn undefined_local(name: &str, span: Span) -> Self {
        Self {
            message: format!("undefined local variable: {name}"),
            span,
        }
    }

    fn heap_type_not_supported(what: &str, span: Span) -> Self {
        Self {
            message: format!(
                "heap types (String, Vector, HashMap) are not yet supported in this position: {what}. \
                 Only bare local variables may hold heap values in Bậc A."
            ),
            span,
        }
    }

    /// ADR-0065 pending: refuse a payload-bearing nullable enum `E?`. The
    /// disc-niche nullable repr (§12.7) is sound only for unit-only enums —
    /// the present value alone IS the whole 8-byte repr, so there is no room
    /// for a payload once the enum needs its own disc+payload (>8B) slot.
    ///
    /// `location`, when `Some`, names the field/variant + container that
    /// pins down WHERE the bad type appears — used by the declaration-site
    /// chokepoint (`lower_program`, WO-NullableEnumAggregate-Refuse PA-A,
    /// 2026-07-18) which scans every `Item::Struct`/`Item::Enum` up front.
    /// `None` is used by the older construction-site chokepoint
    /// (`Expr::OutcomeConstructor`, fixtures 374/375), which has no
    /// field/variant name to report at that point in lowering.
    fn nullable_enum_payload_unsupported(
        enum_name: &str,
        location: Option<&str>,
        span: Span,
    ) -> Self {
        let where_clause = location.map_or(String::new(), |loc| format!(" ({loc})"));
        Self {
            message: format!(
                "nullable enum `{enum_name}?`{where_clause}: payload-bearing nullable \
                 enums inside aggregates are currently unsupported (ADR-0065 pending).\n\
                 [Fix] Remove the `?` and model absence as an explicit no-payload \
                 variant (e.g. add a `None` variant to `{enum_name}`)."
            ),
            span,
        }
    }

    /// P0-sibling gap (WO-enum-return-sret Round 2, O recon 2026-07-17):
    /// `Nullable(Enum)` in RETURN position has no sret ABI yet. The P0 fix
    /// widened `MirType::Enum` return routing (and `INV-Enum-shape`) but
    /// `Nullable(Enum)` does not match `MirType::Enum` — it fell through to
    /// `ReturnShape::Scalar` untouched, silently miscompiling exactly like
    /// the enum-return bug this fixes. Unit-only `Enum?` as a LOCAL/param is
    /// unaffected (disc-niche PA-3c repr, sound) — only the RETURN position
    /// is refused here.
    fn nullable_enum_return_unsupported(enum_name: &str, span: Span) -> Self {
        Self {
            message: format!(
                "nullable enum return `{enum_name}?` is not yet supported: functions \
                 returning `{enum_name}?` have no sret ABI yet (return-shape decision \
                 only recognizes bare `{enum_name}`, not `{enum_name}?`) — returning it \
                 would silently miscompile as a Scalar 1-value, discarding the \
                 discriminant.\n[Fix] Return `{enum_name}` unwrapped and model absence \
                 as an explicit no-payload variant (e.g. add a `None`/`Nil` variant to \
                 `{enum_name}`), or write the nullable result into an out-parameter \
                 (`&0 mutable {enum_name}?`) instead of the return position."
            ),
            span,
        }
    }

    /// ADR-0065 §14 Amend (WO-2 Lát A, 2026-07-20 — narrowed after D found
    /// the unconditional predecessor (`nullable_struct_return_unsupported`,
    /// WO-StructReturnRefuse 2026-07-19) was the ONLY B8 (§4) enforcement at
    /// this position — the MIR verifier's `is_lowerable_nullable_payload`
    /// allows `MirType::Struct` unconditionally at return-type position, no
    /// Copy-ness/heap-content gate). Full-SRET now lands for Copy-only
    /// `Nullable(Struct)` return (tag-prepend, §3.2), but a struct with a
    /// HEAP-bearing field (String/Vector/HashMap, transitively through
    /// nested structs) still refuses: B8 (§4) restricts `Struct?`/`Enum?` to
    /// Copy-only fields/payload ("KHÔNG DROP GLUE. KHÔNG ALLOC. KHÔNG
    /// FREE.") — the `{tag,fields}` sret buffer carries no drop-glue, so a
    /// heap leaf living in it would leak or double-free. This is a DESIGN
    /// FENCE (ADR-locked), not a "not implemented yet" gap.
    fn nullable_struct_return_heap_field_unsupported(
        struct_name: &str,
        field_name: &str,
        span: Span,
    ) -> Self {
        Self {
            message: format!(
                "nullable struct return `{struct_name}?` has a heap-bearing field \
                 `{field_name}` and cannot be returned this way: ADR-0065 §4 (B8) \
                 restricts `Struct?`/`Enum?` to Copy-only fields/payload — the \
                 tag-prepend sret buffer carries no drop-glue, so a heap leaf in \
                 `{field_name}` would leak or double-free. This is a design fence, \
                 not a missing feature.\n\
                 [Fix 1] Change the return type to `{struct_name}` and model \
                 absence with a separate variant/flag.\n\
                 [Fix 2] Move the heap field out of the returned struct."
            ),
            span,
        }
    }

    fn null_literal_without_expected_type(span: Span) -> Self {
        Self {
            message: "Outcome/nullable constructor (`~+`/`~0`/`~-`) requires an expected \
                 type from context (annotate the binding or the return type, e.g. \
                 `let x: Integer? = ~0` or a function returning `T?` / `T~E`)."
                .to_string(),
            span,
        }
    }
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

// ── Lowering context ────────────────────────────────────────

/// Program-wide lowering input, shared by every function/method body.
///
/// Bundles the 8 invariant tables that `lower_program` computes once and
/// every `lower_function` / `Ctx::new` consumes. Passed by `&` and cloned
/// internally (each `Ctx` owns its own copies) — this is a structural
/// refactor of the former 11-/8-argument parameter lists, NOT an ownership
/// change: the clones are identical to the pre-refactor per-call `.clone()`s.
pub(crate) struct LoweringInput<'a> {
    /// AST arena, borrowed from the `Program` for the duration of lowering.
    pub arena: &'a Arena,
    /// Item symbol table (struct/enum names → kind) for `lower_type`.
    pub symbols: HashMap<String, TypeKind>,
    /// Struct layouts keyed by name (`struct_map` in `lower_program`).
    pub struct_layouts: HashMap<String, StructLayout>,
    /// Enum layouts keyed by name (`enum_map` in `lower_program`).
    pub enum_layouts: HashMap<String, EnumLayout>,
    /// Resolved enum variants from the type checker, keyed by pattern ID.
    pub pattern_resolutions: PatternResolutions,
    /// ADR-0061 T5: resolved trait-method calls (ExprId → mangled fn).
    pub method_resolutions: MethodResolutions,
    /// Map from (possibly mangled) function name to its return type.
    pub func_return_types: HashMap<String, MirType>,
    /// ADR-0069: capability name → Ł3 level. `lower_type_simple` (which keys
    /// off Ctx layout maps, not `symbols`) consults the keys to resolve a
    /// capability annotation to `MirType::Capability` (not `Unknown`, which
    /// would be Copy → a silent non-copy bypass). Lát 3: `Expr::Mint` reads the
    /// level so a `defer` mint emits a `CapabilityCheck` runtime gate.
    pub capabilities: std::collections::HashMap<String, CapabilityLevel>,
}

/// Per-function lowering state: local/block allocation, variable scope,
/// the basic blocks built so far, and the signature under construction.
struct Ctx {
    vars: HashMap<String, Local>,
    local_decls: Vec<LocalDecl>,
    cur: BasicBlock,
    next_bb: usize,
    mir_blocks: Vec<triet_mir::BlockData>,
    sig: FunctionSignature,

    /// Struct layouts keyed by name (cloned from lower_program's computation).
    struct_layouts: HashMap<String, StructLayout>,

    /// Enum layouts keyed by name (cloned from lower_program's computation).
    /// Still needed by the JIT for layout sizes/offsets. Variant resolution
    /// is handled by the type checker — these are NOT scanned by the lowerer
    /// for name→discriminant mapping.
    enum_layouts: HashMap<String, EnumLayout>,
    /// ADR-0069: capability name → Ł3 level. Keys drive `lower_type_simple`
    /// resolution; the level drives `Expr::Mint` (defer → `CapabilityCheck`).
    capabilities: std::collections::HashMap<String, CapabilityLevel>,
    /// Resolved enum variants from the type checker, keyed by pattern ID.
    pattern_resolutions: PatternResolutions,
    /// ADR-0061 T5: resolved trait-method calls (ExprId → mangled concrete
    /// function), from the type checker. Read at `Expr::MethodCall` to emit
    /// a direct `CallDispatch` to the impl method's `Body`.
    method_resolutions: MethodResolutions,
    /// Map from function name to its return type name.
    func_return_types: HashMap<String, MirType>,
    /// If this function returns a struct, the local holding the sret pointer.
    sret_ptr: Option<Local>,
    /// Locals introduced by `let` bindings that need `Drop` at scope exit.
    /// Flattened across all active scopes — `scope_snapshots` tracks
    /// where each scope started so we can drop in reverse order per scope.
    owned_locals: Vec<Local>,
    /// Stack of `owned_locals` lengths at each scope entry. `push_scope`
    /// pushes the current length; `pop_scope` drops everything from that
    /// snapshot to the end and truncates.
    scope_snapshots: Vec<usize>,
    /// Human-readable names for let-bound locals. Populated by `Stmt::Let`;
    /// passed through to `Body::local_names` for borrowck diagnostics.
    local_names: BTreeMap<Local, String>,
}

impl Ctx {
    fn new(
        name: &str,
        ret: &MirType,
        input: &LoweringInput,
        span: Span,
    ) -> Result<Self, LowerError> {
        // P0-sibling gap (WO-enum-return-sret Round 2, O recon): `Nullable(Enum)`
        // does NOT match `MirType::Enum` below, so it silently fell through to
        // `ReturnShape::Scalar` — the exact P0 bug, un-caught by `INV-Enum-shape`
        // (which only fires for a bare `MirType::Enum` return_type). Refuse it
        // HERE, at the same return-shape-decision layer as `is_enum_return`,
        // narrowly: only `Nullable(Enum)` in RETURN position, and only when the
        // enum is unit-only. Payload-bearing `Enum?` is refused ALREADY at
        // construction (`nullable_enum_payload_unsupported`, fixtures 374/375)
        // — do not re-refuse it here (would just be a second, redundant guard
        // on an already-dead path). Unit-only `Enum?` as a local/param is sound
        // (PA-3c disc-niche repr) and must NOT be touched — this check is
        // return-position-only by construction (it only runs once, in `Ctx::new`,
        // which is only ever called with the function's RETURN type).
        if let MirType::Nullable(inner) = ret
            && let MirType::Enum(enum_name) = inner.as_ref()
            && let Some(layout) = input.enum_layouts.get(enum_name)
            && layout.variants.iter().all(|v| v.payload.is_none())
        {
            return Err(LowerError::nullable_enum_return_unsupported(
                enum_name, span,
            ));
        }
        // ADR-0065 §14 Amend (WO-2 Lát A, 2026-07-20): the unconditional
        // refuse that used to sit here (`nullable_struct_return_unsupported`,
        // WO-StructReturnRefuse 2026-07-19) is replaced by full-SRET for
        // Copy-only `Nullable(Struct)` return. `is_struct_return` now unwraps
        // `Nullable` (idiom already used at `mir_lower.rs:2437,2472`) so the
        // return-shape decision recognizes `Struct?` the same as `Struct`.
        // The Copy-vs-heap-field split is checked further down, AFTER `ctx`
        // is constructed (`ctx_is_copy` needs `ctx.struct_layouts`, not yet
        // available at this point in the function) — see the
        // `nullable_struct_return_heap_field_unsupported` gate below.
        let is_struct_return = matches!(ret.nullable_payload().unwrap_or(ret), MirType::Struct(_));
        // P0 fix (2026-07-17): a user-defined enum return was falling into
        // the `_ => Scalar` catch-all below — no arm ever recognized
        // `MirType::Enum`. Register-based enum return has no static variant
        // at the return site, so it needs sret (like Struct) — but its own
        // ReturnShape::Enum (not Struct): the JIT copy is a raw byte-copy
        // by total_size, not a field-wise struct copy (PQ-2, WO-enum-return-sret).
        let is_enum_return = matches!(ret, MirType::Enum(_));
        // ADR-0058 Lát 1: heap binary Outcome uses JIT-sret (tái dùng String
        // machinery).  Scalar Outcome giữ 2-register; Ternary heap = deferred.
        let is_heap_outcome = matches!(
            ret,
            MirType::Outcome {
                allow_null_state: false,
                ..
            }
        ) && ret.has_heap_payload();
        // ADR-0049 L6 Lối d: String uses JIT-sret but keeps M4-escape Return[s].
        // ADR-0062: `String?` shares String's slot → same fat path (is_string_repr).
        let is_fat_return =
            is_struct_return || is_enum_return || ret.is_string_repr() || is_heap_outcome;
        let return_shape = match ret {
            _ if is_heap_outcome => triet_mir::ReturnShape::Struct {
                struct_name: ret.to_string(),
            },
            MirType::Outcome {
                allow_null_state: false,
                ..
            } => triet_mir::ReturnShape::BinaryOutcome,
            MirType::Outcome {
                allow_null_state: true,
                ..
            } => triet_mir::ReturnShape::TernaryOutcome,
            _ if is_enum_return => triet_mir::ReturnShape::Enum {
                enum_name: ret.to_string(),
            },
            _ if is_fat_return => triet_mir::ReturnShape::Struct {
                struct_name: ret.to_string(),
            },
            _ => triet_mir::ReturnShape::Scalar,
        };
        let mut ctx = Self {
            vars: HashMap::new(),
            local_decls: Vec::new(),
            cur: BasicBlock(0),
            next_bb: 1,
            mir_blocks: vec![triet_mir::BlockData {
                statements: Vec::new(),
                terminator: Terminator::Unreachable { span: DUMMY_SPAN },
            }],
            sig: FunctionSignature {
                name: name.to_string(),
                parameters: Vec::new(),
                return_type: ret.clone(),
                return_borrow_map: triet_mir::ReturnBorrowMap::new(),
                return_shape,
            },
            struct_layouts: input.struct_layouts.clone(),
            enum_layouts: input.enum_layouts.clone(),
            capabilities: input.capabilities.clone(),
            pattern_resolutions: input.pattern_resolutions.clone(),
            method_resolutions: input.method_resolutions.clone(),
            func_return_types: input.func_return_types.clone(),
            sret_ptr: None,
            owned_locals: Vec::new(),
            scope_snapshots: Vec::new(),
            local_names: BTreeMap::new(),
        };
        // ADR-0065 §14 Amend (WO-2 Lát A, 2026-07-20): B8 (§4) Copy-only gate
        // for `Nullable(Struct)` return, checked here (after `ctx` exists, so
        // `ctx_is_copy` can consult `ctx.struct_layouts`/`ctx.enum_layouts`).
        // Mirrors §12.2's `is_copy(Some(body))` gate for a nested nullable-
        // aggregate FIELD, one layer up: here it is the top-level RETURN
        // type. A heap-bearing struct (String/Vector/HashMap, transitively
        // through nested structs) refuses — B8 forbids drop-glue on an
        // aggregate-nullable slot, and the tag-prepend sret buffer has none.
        if let MirType::Nullable(inner) = ret
            && let MirType::Struct(struct_name) = inner.as_ref()
            && !ctx_is_copy(inner, &ctx)
        {
            let field_name = ctx
                .struct_layouts
                .get(struct_name.as_str())
                .and_then(|l| l.fields.iter().find(|f| !ctx_is_copy(&f.ty, &ctx)))
                .map_or_else(|| "?".to_string(), |f| f.name.clone());
            return Err(LowerError::nullable_struct_return_heap_field_unsupported(
                struct_name,
                &field_name,
                span,
            ));
        }
        if is_fat_return {
            ctx.sret_ptr = Some(ctx.alloc_local_ty(ret));
        }
        Ok(ctx)
    }

    /// Allocate a fresh local with a declared type.
    /// TECH-DEBT(B1a): &str bridge via From impl — migrate to MirType at S3.
    fn alloc_local_ty(&mut self, ty: impl Into<MirType>) -> Local {
        let l = Local(self.local_decls.len());
        self.local_decls.push(LocalDecl::new(ty));
        l
    }

    /// Allocate a temporary whose type is not tracked yet.
    fn alloc_local(&mut self) -> Local {
        self.alloc_local_ty(MirType::Unknown)
    }

    // ── Scope tracking (Drop emission) ─────────────────────────

    /// Push a new scope onto the stack. All subsequent `push_owned` calls
    /// will register locals in this scope. Returns the snapshot index.
    fn push_scope(&mut self) {
        self.scope_snapshots.push(self.owned_locals.len());
    }

    /// Pop the innermost scope and emit [`Statement::Drop`] for every
    /// owned local registered in it.
    ///
    /// ADR-0046: reference types (`&0 T` etc.) are sorted to drop BEFORE
    /// non-reference types, regardless of allocation order. This ensures
    /// borrowers die before their owners, preventing spurious E2450 when
    /// a PropagatedLoan ties a returned reference back to its source.
    fn pop_scope(&mut self) {
        let Some(snapshot) = self.scope_snapshots.pop() else {
            return;
        };
        // Collect indices with sort key, then emit Drop in sorted order.
        // Avoids borrowing self.owned_locals and self.local_decls
        // simultaneously (they're both fields of self).
        let mut locals: Vec<Local> = self.owned_locals.drain(snapshot..).collect();
        locals.sort_by_key(|&l| {
            let ty = &self.local_decls[l.0].ty;
            ty.is_reference()
        });
        for l in locals.into_iter().rev() {
            self.push(Statement::Drop(l, DUMMY_SPAN));
        }
    }

    /// Register a local as needing Drop at the end of the current scope.
    /// No-op if the local is already registered (e.g., `let b = a` reuses
    /// the same local as `a` without an intervening Assign).
    fn push_owned(&mut self, l: Local) {
        if !self.owned_locals.contains(&l) {
            self.owned_locals.push(l);
        }
    }

    // ── HP.4: heap Outcome payload {ptr,len,cap} move helpers ──────────
    //
    // A heap value (String/Vector/HashMap) is 24 bytes `{ptr,len,cap}`; the
    // Outcome slot holds it at offsets 8/16/24 (ADR-0053 §3.3). Moving such a
    // payload requires all three words, not just the `ptr` at OutcomePayload.

    /// Decompose a heap payload out of `src` Outcome's slot into a freshly
    /// typed struct local `dst` (bind capture). Mirror of the match-arm
    /// decompose (HP.3). Caller must `Deinit(src)` afterwards.
    fn bind_heap_outcome_payload(&mut self, dst: Local, src: Local, span: &Span) {
        for (proj, field) in [
            (Projection::OutcomePayload, "ptr"),
            (Projection::OutcomePayloadLen, "len"),
            (Projection::OutcomePayloadCap, "cap"),
        ] {
            let tmp = self.alloc_local_ty(MirType::Integer);
            self.push(Statement::StorageLive(tmp, span.clone()));
            self.push(Statement::Assign {
                dest: Place::local(tmp),
                source: Place::local(src).project(proj),
                span: span.clone(),
            });
            self.push(Statement::Assign {
                dest: Place::local(dst).project(Projection::Field(field.to_string())),
                source: Place::local(tmp),
                span: span.clone(),
            });
        }
    }

    /// Recompose a heap struct local `src` {ptr,len,cap} into `dst` Outcome
    /// slot's payload (inverse of [`bind_heap_outcome_payload`]). Caller must
    /// `Deinit(src)` afterwards so its scope-pop `Drop` is a no-op.
    fn write_heap_outcome_payload(&mut self, dst: Local, src: Local, span: &Span) {
        for (proj, field) in [
            (Projection::OutcomePayload, "ptr"),
            (Projection::OutcomePayloadLen, "len"),
            (Projection::OutcomePayloadCap, "cap"),
        ] {
            let tmp = self.alloc_local_ty(MirType::Integer);
            self.push(Statement::StorageLive(tmp, span.clone()));
            self.push(Statement::Assign {
                dest: Place::local(tmp),
                source: Place::local(src).project(Projection::Field(field.to_string())),
                span: span.clone(),
            });
            self.push(Statement::Assign {
                dest: Place::local(dst).project(proj),
                source: Place::local(tmp),
                span: span.clone(),
            });
        }
    }

    /// Copy a heap payload {ptr,len,cap} between two Outcome slots
    /// (passthrough arm). Caller must `Deinit(src)` afterwards.
    fn copy_heap_outcome_payload(&mut self, dst: Local, src: Local, span: &Span) {
        for proj in [
            Projection::OutcomePayload,
            Projection::OutcomePayloadLen,
            Projection::OutcomePayloadCap,
        ] {
            self.push(Statement::Assign {
                dest: Place::local(dst).project(proj.clone()),
                source: Place::local(src).project(proj),
                span: span.clone(),
            });
        }
    }

    /// Emit `Drop` for every currently-owned local BEFORE a Return
    /// terminator, in forward order so the *source* of a loan drops
    /// before its *dest*. This way E2450 fires before NLL cleanup on
    /// the dest can remove the loan.
    ///
    /// **Does NOT clear `owned_locals` or `scope_snapshots`.**  This is
    /// deliberate: a local live before a control-flow split must be
    /// dropped on EVERY exit path after it.  Clearing the global state
    /// on the first `return` would silently skip the drop on sibling
    /// paths (Case D soundness hole).  The scope bookkeeping is cleaned
    /// up by `pop_scope` at normal scope boundaries.
    fn flush_all_for_return(&mut self) {
        for i in 0..self.owned_locals.len() {
            self.push(Statement::Drop(self.owned_locals[i], DUMMY_SPAN));
        }
    }

    fn alloc_bb(&mut self) -> BasicBlock {
        let bb = BasicBlock(self.next_bb);
        self.next_bb += 1;
        self.mir_blocks.push(triet_mir::BlockData {
            statements: Vec::new(),
            terminator: Terminator::Unreachable { span: DUMMY_SPAN },
        });
        bb
    }

    fn push(&mut self, s: Statement) {
        self.mir_blocks[self.cur.0].statements.push(s);
    }

    fn term(&mut self, bb: BasicBlock, t: Terminator) {
        self.mir_blocks[bb.0].terminator = t;
    }

    fn is_open(&self, bb: BasicBlock) -> bool {
        matches!(
            self.mir_blocks[bb.0].terminator,
            Terminator::Unreachable { .. }
        )
    }

    /// True if any block's terminator has an edge into `bb` (i.e. `bb` has an
    /// incoming predecessor). Used to tell a reachable open block apart from a
    /// dead continuation block: `Stmt::Return` leaves `cur` pointing at a fresh
    /// block with no incoming edge (ADR-0055 Bug A). Such a block must NOT
    /// receive a synthetic tail `Return` — that arity-1 unit Return fails
    /// Outcome arity verification, and INV-4 happily leaves it `Unreachable`
    /// since nothing references it. Mirrors the verifier's reachability scan.
    fn block_has_incoming(&self, bb: BasicBlock) -> bool {
        self.mir_blocks.iter().any(|bd| match &bd.terminator {
            Terminator::Goto { target, .. } => *target == bb,
            Terminator::If {
                positive_bb,
                zero_bb,
                negative_bb,
                ..
            } => *positive_bb == bb || *negative_bb == bb || *zero_bb == Some(bb),
            Terminator::CallDispatch { return_bb, .. } => *return_bb == bb,
            Terminator::SwitchInt {
                cases, default_bb, ..
            } => *default_bb == bb || cases.iter().any(|&(_, t)| t == bb),
            Terminator::Return { .. }
            | Terminator::Unreachable { .. }
            | Terminator::Trap { .. } => false,
        })
    }

    fn build(self, entry: BasicBlock) -> Body {
        Body {
            signature: self.sig,
            blocks: self.mir_blocks,
            entry_block: entry,
            num_locals: self.local_decls.len(),
            local_decls: self.local_decls,
            struct_layouts: Vec::new(),
            enum_layouts: self.enum_layouts.values().cloned().collect(),
            local_names: self.local_names,
        }
    }
}

// ── Public API ──────────────────────────────────────────────

/// Kind of a user-defined type — struct or enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TypeKind {
    Struct,
    Enum,
    /// ADR-0069: a capability token type (`capability Cap grant`). Resolves to
    /// `MirType::Capability` — ZST, always non-copy.
    Capability,
}

/// ADR-0067 §AMEND (Enum-Payload-Aggregate Sizing) — resolve the byte width
/// of an aggregate-bearing field/payload type, consulting the current
/// `struct_map`/`enum_map` snapshot. Shared by the struct-field fixup pass
/// and the enum-payload fixup pass in [`lower_program`] so the two stay in
/// lockstep during the combined co-fixpoint (a struct field may be an enum
/// — ADR-0067 2b+ death-line #2 — and, as of this front, an enum payload may
/// be a struct or another enum). `fallback` is the size from the previous
/// iteration (or the initial seed); used only until the referenced layout
/// resolves — every struct/enum declared in the program already has an
/// entry in both maps by construction, so the fallback branch is a
/// defensive no-op in a well-formed program, never a permanent value.
fn resolve_aggregate_size(
    ty: &MirType,
    struct_map: &HashMap<String, StructLayout>,
    enum_map: &HashMap<String, EnumLayout>,
    fallback: usize,
) -> usize {
    match ty {
        MirType::Struct(name) => struct_map
            .get(name.as_str())
            .map_or(fallback, |l| l.total_size),
        // ADR-0067 2b+ (death-line #2): an enum's width comes from
        // `enum_map` (a heap-payload enum is 32B: {disc@0, ptr@8, len@16,
        // cap@24}), NOT struct_map — enums are never registered there.
        MirType::Enum(name) => enum_map
            .get(name.as_str())
            .map_or(fallback, |l| l.total_size),
        // ADR-0065 §12.1: nested nullable aggregate size.
        MirType::Nullable(inner) => match inner.as_ref() {
            // Struct? prepends an 8-byte tag word (Phương án A, §2.2) →
            // inner.total + 8.
            MirType::Struct(name) => struct_map
                .get(name.as_str())
                .map_or(fallback, |l| l.total_size + 8),
            // Enum? uses the disc-niche (0-byte overhead, §2.1); its size
            // resolves like a plain `Enum` field — look it up in `enum_map`,
            // NOT `struct_map` (an enum is never registered there, see the
            // `MirType::Enum` arm above). A payload-bearing `Enum?` is
            // refused at the declaration chokepoint before this size is
            // ever consumed by codegen (WO-NullableEnumAggregate-Refuse
            // PA-A, 2026-07-18).
            // TODO(ADR-0065): Layout blocked by nullable_enum_payload_unsupported at
            // frontend. This sizing is correct but representation is unimplemented.
            MirType::Enum(name) => enum_map
                .get(name.as_str())
                .map_or(fallback, |l| l.total_size),
            // ADR-0076: heap-`T?` leaf field. The ptr-sentinel rides the
            // inner's repr, so the slot at field-offset is the SAME width
            // as the plain heap field.
            MirType::String => 24,
            MirType::Vector(_) | MirType::HashMap(..) => 8,
            _ => fallback,
        },
        // ADR-0066 M-1: heap LEAF field width.
        MirType::String => 24,
        MirType::Vector(_) | MirType::HashMap(..) => 8,
        _ => fallback,
    }
}

/// Lower every function in a parsed program to its MIR body.
///
/// Returns `Err` if any function contains an AST construct the lowerer
/// does not yet support. The error carries a span for diagnostics.
///
/// Struct definitions in the program are lowered to `StructLayout` entries
/// and attached to every function body so that the JIT backend can compute
/// field offsets. In Bậc A every field is 8 bytes (i64), alignment 8.
#[allow(clippy::type_complexity)]
// ^-- TECH-DEBT(B1a S2): complex enum variant type — simplifies when
//     EnumLayout moves to dedicated type aliases at S3.
pub fn lower_program(
    prog: &Program,
    pattern_resolutions: &PatternResolutions,
    method_resolutions: &MethodResolutions,
) -> Result<Vec<Body>, LowerError> {
    // ── Build ItemSymbolTable FIRST (Pass-1, needed by lower_type) ──
    let symbols: std::collections::HashMap<String, TypeKind> = prog
        .items
        .iter()
        .filter_map(|item| match &item.node {
            Item::Struct { def } => Some((def.name.clone(), TypeKind::Struct)),
            Item::Enum { def } => Some((def.name.clone(), TypeKind::Enum)),
            Item::Capability { name, .. } => Some((name.clone(), TypeKind::Capability)),
            _ => None,
        })
        .collect();

    // ── Collect struct layouts from struct definitions ──────────
    // Bậc A: every field is 8 bytes, alignment 8 (single i64).
    // Bậc C will compute real sizes from type information.
    let mut struct_layouts: Vec<StructLayout> = prog
        .items
        .iter()
        .filter_map(|item| {
            if let Item::Struct { def } = &item.node {
                let fields: Vec<(String, MirType, usize, usize)> = def
                    .fields
                    .iter()
                    .map(|f| {
                        let ty = lower_type(&prog.arena, f.type_annotation, &symbols, None);
                        // ADR-0070: a capability field is a ZST — size 0, align 1
                        // (true zero-cost per ADR-0069 "0 byte at runtime"). The
                        // fixpoint pass below preserves this (Capability falls to
                        // its `_ => f.size` default). Every other field is 8B/i64.
                        let (sz, al) = if matches!(ty, MirType::Capability(_)) {
                            (0, 1)
                        } else {
                            (8, 8)
                        };
                        (f.name.clone(), ty, sz, al)
                    })
                    .collect();
                Some(StructLayout::compute(&def.name, &fields))
            } else {
                None
            }
        })
        .collect();
    let mut struct_map: HashMap<String, StructLayout> = struct_layouts
        .iter()
        .map(|l| (l.name.clone(), l.clone()))
        .collect();

    // ── Collect enum layouts from enum definitions ──────────────
    // Seed pass: every payload starts at 8 bytes (i64), except String/String?
    // which is the 24B fat pointer {ptr,len,cap} (ADR-0067 2b-0a / ADR-0076).
    // Unit variants have no payload (size 0). An aggregate (Struct/Enum)
    // payload's REAL width is resolved by the co-fixpoint below — it is
    // fixed up in lockstep with `struct_layouts`, not here.
    let mut enum_layouts: Vec<EnumLayout> = prog
        .items
        .iter()
        .filter_map(|item| {
            if let Item::Enum { def } = &item.node {
                let variants: Vec<(
                    String,
                    i64,
                    Option<(MirType, usize, usize, Vec<FieldLayout>)>,
                )> = def
                    .variants
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let disc = i as i64;
                        let payload = v.payload.map(|tid| {
                            let ty = lower_type(&prog.arena, tid, &symbols, None);
                            let size = if ty.is_string_repr() { 24 } else { 8 };
                            (ty, size, 8usize, Vec::new())
                        });
                        (v.name.clone(), disc, payload)
                    })
                    .collect();
                Some(EnumLayout::compute(&def.name, &variants))
            } else {
                None
            }
        })
        .collect();

    let mut enum_map: HashMap<String, EnumLayout> = enum_layouts
        .iter()
        .map(|l| (l.name.clone(), l.clone()))
        .collect();

    // ADR-0060 P2 / ADR-0067 §AMEND: CO-FIXPOINT over BOTH struct fields and
    // enum payloads. A struct field may be an enum (ADR-0067 2b+ death-line
    // #2) and — as of §AMEND — an enum payload may be a struct or another
    // enum, so the two tables must refine together until BOTH stabilize.
    // `resolve_aggregate_size` is the single sizing function shared by both
    // passes so they can never drift apart. Gauss-Seidel order (the enum
    // pass sees this iteration's fresh `struct_map`; the struct pass sees
    // this iteration's fresh `enum_map`) — the fixed point doesn't depend on
    // order since sizes only grow monotonically and the type graph is a
    // finite DAG (ADR-0068 bans recursive/Box types). Capped at
    // `FIXPOINT_ITERATION_LIMIT` to fail loudly (`Err`, never loop forever /
    // panic) if that DAG invariant is ever violated.
    const FIXPOINT_ITERATION_LIMIT: usize = 64;
    let mut fixpoint_iterations = 0usize;
    loop {
        fixpoint_iterations += 1;
        if fixpoint_iterations > FIXPOINT_ITERATION_LIMIT {
            return Err(LowerError {
                message: format!(
                    "struct/enum layout sizing did not converge after \
                     {FIXPOINT_ITERATION_LIMIT} iterations (a cyclic aggregate \
                     type without indirection? ADR-0068 bans Box/recursive \
                     types, so this should be unreachable for a well-formed \
                     program)"
                ),
                span: DUMMY_SPAN,
            });
        }
        let mut changed = false;

        // ── Enum payload pass ──
        let mut new_enum_layouts: Vec<EnumLayout> = Vec::with_capacity(enum_layouts.len());
        for layout in &enum_layouts {
            let variants: Vec<(
                String,
                i64,
                Option<(MirType, usize, usize, Vec<FieldLayout>)>,
            )> = layout
                .variants
                .iter()
                .map(|v| {
                    let payload = v.payload.as_ref().map(|p| {
                        let size = resolve_aggregate_size(&p.ty, &struct_map, &enum_map, p.size);
                        (p.ty.clone(), size, p.alignment, p.fields.clone())
                    });
                    (v.name.clone(), v.discriminant_value, payload)
                })
                .collect();
            let new_layout = EnumLayout::compute(&layout.name, &variants);
            if new_layout.total_size != layout.total_size {
                changed = true;
            }
            new_enum_layouts.push(new_layout);
        }
        enum_layouts = new_enum_layouts;
        enum_map = enum_layouts
            .iter()
            .map(|l| (l.name.clone(), l.clone()))
            .collect();

        // ── Struct field pass ──
        let mut new_struct_layouts: Vec<StructLayout> = Vec::with_capacity(struct_layouts.len());
        for layout in &struct_layouts {
            let new_fields: Vec<(String, MirType, usize, usize)> = layout
                .fields
                .iter()
                .map(|f| {
                    let size = resolve_aggregate_size(&f.ty, &struct_map, &enum_map, f.size);
                    (f.name.clone(), f.ty.clone(), size, f.alignment)
                })
                .collect();
            let new_layout = StructLayout::compute(&layout.name, &new_fields);
            if new_layout.total_size != layout.total_size {
                changed = true;
            }
            new_struct_layouts.push(new_layout);
        }
        struct_layouts = new_struct_layouts;
        struct_map = struct_layouts
            .iter()
            .map(|l| (l.name.clone(), l.clone()))
            .collect();

        if !changed {
            break;
        }
    }

    // ── ADR-0065 pending: refuse payload-bearing `Enum?` in aggregates ──
    // (WO-NullableEnumAggregate-Refuse PA-A, O recon 2026-07-18). The
    // co-fixpoint above now SIZES a payload-bearing enum correctly (ADR-0067
    // §AMEND, commit 9a1799c) but the disc-niche NULLABLE repr (ADR-0065
    // §12.7) is only sound for UNIT-ONLY enums — the "present value alone IS
    // the repr" trick has nowhere to put a tag once the payload needs its
    // own >8B disc+payload slot. Left unrefused, `struct S{e:E?,tail:...}`
    // silently overflows the 8B slot reserved for `e` into the next field
    // at construction time (verified: `Mid{m:5,e:E::V(42)}` reads back
    // `mid.m == 42`, exit 0 — see fixtures 414/415). `Expr::OutcomeConstructor`
    // already refuses this for values flowing through a nullable-typed
    // expression (`nullable_enum_payload_unsupported`, fixtures 374/375),
    // but that check runs per-function, per-expression — it does not cover
    // every path a bad TYPE can enter the program (e.g. a struct field is
    // never itself an `Expr`). Refusing here, at the DECLARATION chokepoint,
    // covers every value-construction path structurally, once, regardless
    // of how the value later gets built.
    //
    // Unit-only `Enum?` (PA-3c disc-niche, 0-byte overhead) is UNAFFECTED —
    // the `.any(|v| v.payload.is_some())` guard is load-bearing (fixtures
    // 417/418 pin this: a unit-only nullable enum field/local must keep
    // compiling and running).
    for item in &prog.items {
        match &item.node {
            Item::Struct { def } => {
                for field in &def.fields {
                    let ty = lower_type(&prog.arena, field.type_annotation, &symbols, None);
                    if let MirType::Nullable(inner) = &ty
                        && let MirType::Enum(enum_name) = inner.as_ref()
                        && let Some(layout) = enum_map.get(enum_name)
                        && layout.variants.iter().any(|v| v.payload.is_some())
                    {
                        return Err(LowerError::nullable_enum_payload_unsupported(
                            enum_name,
                            Some(&format!("field `{}` of struct `{}`", field.name, def.name)),
                            item.span.clone(),
                        ));
                    }
                }
            }
            Item::Enum { def } => {
                for variant in &def.variants {
                    let Some(payload_tid) = variant.payload else {
                        continue;
                    };
                    let ty = lower_type(&prog.arena, payload_tid, &symbols, None);
                    if let MirType::Nullable(inner) = &ty
                        && let MirType::Enum(enum_name) = inner.as_ref()
                        && let Some(layout) = enum_map.get(enum_name)
                        && layout.variants.iter().any(|v| v.payload.is_some())
                    {
                        return Err(LowerError::nullable_enum_payload_unsupported(
                            enum_name,
                            Some(&format!(
                                "variant `{}` of enum `{}`",
                                variant.name, def.name
                            )),
                            item.span.clone(),
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    // ADR-0049 Phase-1 Lát 1: synthetic String layout for fat-pointer StackSlot.
    // Heap still carries [Header 8B][len 8B][cap 8B][data…] in Lát 1-3;
    // the slot is a cache. Field order: ptr@0 (heap handle), len@8, cap@16.
    // Total 24 bytes, 8-byte aligned.
    // IMPORTANT: String is NOT added to struct_names — it has its own return ABI
    // (single i64, not sret) and Move semantics (not Copy). The layout exists
    // purely for JIT StackSlot allocation and field-offset computation.
    let string_layout = StructLayout::compute(
        "String",
        &[
            ("ptr".to_string(), MirType::Integer, 8, 8),
            ("len".to_string(), MirType::Integer, 8, 8),
            ("cap".to_string(), MirType::Integer, 8, 8),
        ],
    );
    struct_layouts.push(string_layout.clone());
    struct_map.insert("String".to_string(), string_layout);

    let mut func_return_types: HashMap<String, MirType> = prog
        .items
        .iter()
        .filter_map(|item| {
            if let Item::Function { def } = &item.node {
                let ret = def
                    .return_type
                    .map(|tid| lower_type(&prog.arena, tid, &symbols, None))
                    .unwrap_or(MirType::Integer);
                Some((def.name.clone(), ret))
            } else {
                None
            }
        })
        .collect();

    // ADR-0061 T5: register impl methods under their mangled names so a
    // resolved method call (`MethodResolution.concrete_fn`) can look up its
    // return type — the dispatch at the MethodCall site reads this map, the
    // same machinery user functions use. `Self` in the return type resolves
    // to the impl's `for_type`.
    for item in &prog.items {
        if let Item::Implementation { def } = &item.node {
            let self_ty = lower_type(&prog.arena, def.for_type, &symbols, None);
            let for_type_name = self_ty.to_string();
            for method in &def.methods {
                let mangled = triet_syntax::mangle_trait_method(
                    &for_type_name,
                    &def.trait_name,
                    &method.name,
                );
                let ret = method
                    .return_type
                    .map(|tid| lower_type(&prog.arena, tid, &symbols, Some(&self_ty)))
                    .unwrap_or(MirType::Integer);
                func_return_types.insert(mangled, ret);
            }
        }
    }

    // Bundle the 8 invariant tables once. `symbols`/`struct_map`/`enum_map`/
    // `func_return_types` are moved in (no longer used directly below — only
    // via `input.<field>`); the three resolution maps are `&` parameters so
    // they are cloned. The `struct_layouts`/`enum_layouts` Vecs are NOT moved
    // here — they stay live to fill `body.struct_layouts`/`enum_layouts`.
    // ADR-0069: capability name → Ł3 level, read straight from the decls (the
    // symbol table carries only the kind, not the level the mint gate needs).
    let capabilities: std::collections::HashMap<String, CapabilityLevel> = prog
        .items
        .iter()
        .filter_map(|item| match &item.node {
            Item::Capability { name, level } => Some((name.clone(), level.clone())),
            _ => None,
        })
        .collect();
    let input = LoweringInput {
        arena: &prog.arena,
        symbols,
        struct_layouts: struct_map,
        enum_layouts: enum_map,
        pattern_resolutions: pattern_resolutions.clone(),
        method_resolutions: method_resolutions.clone(),
        func_return_types,
        capabilities,
    };

    let mut bodies = Vec::new();
    for item in &prog.items {
        if let Item::Function { def } = &item.node {
            let mut body = lower_function(&input, def, item.span.clone(), None)?;
            body.struct_layouts = struct_layouts.clone();
            body.enum_layouts = enum_layouts.clone();
            bodies.push(body);
        }
    }

    // ADR-0061 T5.2: lower each `implement` method into an ordinary Body
    // named `Type$Trait$method` (the mangled name the dispatch calls). The
    // method is a normal FunctionDefinition; `impl_ctx` supplies the
    // mangled name (GAP-A) and the `for_type` that `self` resolves to
    // (GAP-B). `self` lives in params[0] like any local.
    for item in &prog.items {
        if let Item::Implementation { def } = &item.node {
            let self_ty = lower_type(input.arena, def.for_type, &input.symbols, None);
            let for_type_name = self_ty.to_string();
            for method in &def.methods {
                let mangled = triet_syntax::mangle_trait_method(
                    &for_type_name,
                    &def.trait_name,
                    &method.name,
                );
                let mut body = lower_function(
                    &input,
                    method,
                    item.span.clone(),
                    Some((&mangled, self_ty.clone())),
                )?;
                body.struct_layouts = struct_layouts.clone();
                body.enum_layouts = enum_layouts.clone();
                bodies.push(body);
            }
        }
    }
    Ok(bodies)
}

/// Lower one function definition to a MIR body.
///
/// `span` is the byte range of the function definition in the source file,
/// used as a fallback for body-level synthetic statements.
/// ADR-0061 T5: `impl_ctx` carries the `implement`-block context — the
/// mangled `Body` name (`Type$Trait$method`) and the `for_type` that
/// `Self` resolves to. `None` for ordinary top-level functions (name =
/// `func.name`, no `Self`). `Some` only for trait impl methods: both
/// halves are always present together, so a single tuple makes the two
/// illegal states (one-without-the-other) unrepresentable (O ruling).
pub(crate) fn lower_function(
    input: &LoweringInput,
    func: &FunctionDefinition,
    span: Span,
    impl_ctx: Option<(&str, MirType)>,
) -> Result<Body, LowerError> {
    let (body_name, self_type): (&str, Option<&MirType>) = match &impl_ctx {
        Some((name, ty)) => (name, Some(ty)),
        None => (func.name.as_str(), None),
    };
    let ret_ty = func
        .return_type
        .as_ref()
        .map(|tid| lower_type(input.arena, *tid, &input.symbols, self_type))
        .unwrap_or(MirType::Integer);
    let mut c = Ctx::new(body_name, &ret_ty, input, span.clone())?;
    let entry = c.cur;

    // Function scope: Drop all owned locals (parameters + let bindings) when
    // the function exits. Must start before pushing parameters.
    c.push_scope();

    for p in &func.parameters {
        let ty = lower_type(input.arena, p.type_annotation, &input.symbols, self_type);
        // B7-lift (ADR-0042): heap types now allowed as parameters.
        // Move semantics: callee owns + drops, caller zeroes slot after call.
        // ADR-0045 §2: reference types (&0 String etc.) are borrow parameters
        // — callee does NOT own, must NOT drop. Heap types with non-ref
        // annotations (e.g. s: String) remain Move.
        let l = c.alloc_local_ty(&ty);
        c.vars.insert(p.name.clone(), l);
        let passing = match p.passing_mode {
            triet_syntax::ParameterPassing::Borrow => ParameterPassing::Borrow,
            triet_syntax::ParameterPassing::Move => ParameterPassing::Move,
            triet_syntax::ParameterPassing::MutableBorrow => ParameterPassing::MutableBorrow,
        };
        // Only push_owned for Move parameters and non-reference types.
        // Reference types (&0 String) — callee borrows, no Drop.
        let is_ref_type = ty.is_reference();
        if matches!(passing, ParameterPassing::Move) || !is_ref_type {
            c.push_owned(l);
        }
        c.sig.parameters.push((p.name.clone(), passing));
    }

    // ADR-0046 §3: populate return_borrow_map for return-borrow elision.
    // If the return type is &0 T, tie it to the single ref-param.
    // Elision rule (check_lifetime_elision, check.rs:494) guarantees
    // exactly 0 or 1 non-owning ref-parameters — 0 = fn with no ref parameters
    // returning &0 T (unusual but valid: return a borrowed static/global);
    // 1 = tie to that param. 2+ is refused by E2400 (fatal at typecheck).
    // defense-in-depth: if typecheck leaks, Err — not panic — because
    // the harness (integration_tests.rs:64) runs through type errors and
    // panic would SIGABRT the entire corpus.
    if matches!(&c.sig.return_type, MirType::Reference { form, .. } if matches!(form, triet_mir::ReferenceForm::BorrowReadOnly | triet_mir::ReferenceForm::BorrowExclusiveMutable))
    {
        let ref_param_indices: Vec<usize> = c
            .sig
            .parameters
            .iter()
            .enumerate()
            .filter(|(_, (name, _))| {
                // ADR-0046 Blocker 2 fix: count by type-string & prefix
                // (Lối 1 — Move-vs-Borrow quyết theo type, không theo
                // ParameterPassing).  Mọi non-owning ref (&0/&0 mutable/&-)
                // bắt đầu bằng '&' nhưng KHÔNG bằng "&+".
                if let Some(&local) = c.vars.get(name) {
                    let ty = &c.local_decls[local.0].ty;
                    matches!(ty, MirType::Reference { form, .. } if !matches!(form, triet_mir::ReferenceForm::StrongFrozen | triet_mir::ReferenceForm::StrongMutable))
                } else {
                    false
                }
            })
            .map(|(i, _)| i)
            .collect();
        match ref_param_indices.len() {
            0 => {} // No ref-parameters to tie to — valid (static/global return).
            1 => {
                c.sig
                    .return_borrow_map
                    .insert(FieldPath::Root, BTreeSet::from([ref_param_indices[0]]));
            }
            _ => {
                return Err(LowerError {
                    message: format!(
                        "internal: return-borrow elision expects exactly 1 ref-param \
                         (found {}; typecheck E2400 should have rejected this)",
                        ref_param_indices.len()
                    ),
                    span,
                });
            }
        }
    }

    // ADR-0055: a block-form function body IS an expression; its return value
    // is its tail expression — identical to expr-body `= expr`. The distinction
    // between `FunctionBody::Block` and `FunctionBody::Expression` is artificial
    // and unified here: both lower through `lower_expr`. (`lower_block` survives
    // only for while-body, where discarding the tail is correct.)
    let body_expr = match &func.body {
        FunctionBody::Block { block } => Some(*block),
        FunctionBody::Expression { expr } => Some(*expr),
        FunctionBody::External { .. } => None,
    };
    if let Some(e) = body_expr {
        // ADR-0072 §2.3: the function body tail is a value-context SOURCE — the
        // function return type is the expected type. `~0`/`~+`/`null` consume it
        // in the leaf-consumer (§2.4); the is_null special-case is gone.
        let ret_ty = c.sig.return_type.clone();
        let val = lower_expr(e, Some(&ret_ty), input.arena, &mut c)?;
        // Emit the tail Return only into a REACHABLE open block. A block-body
        // ending in an explicit `return` leaves `cur` pointing at a dead
        // continuation block (created by Stmt::Return, no incoming edge);
        // injecting a synthetic unit Return there fails Outcome arity
        // verification (ADR-0055 Bug A) and is pointless dead code. `is_open`
        // filters blocks still holding the alloc_bb placeholder; the incoming
        // check filters dead ones. For expr-body `cur` is the entry (or a
        // reachable merge) so the guard always passes — no behavior change.
        if c.is_open(c.cur) && (c.cur == entry || c.block_has_incoming(c.cur)) {
            let span = input.arena.expression(e).span.clone();
            if emit_struct_sret_copy(&mut c, val, &span) {
                // Struct tail-return: fields copied into sret; emit Return(())
                // then leave `cur` at a fresh dead block so the pop_scope()
                // below drops owned locals into dead code, never after the
                // terminator. Mirrors the Stmt::Return struct branch.
                c.flush_all_for_return();
                let cur = c.cur;
                c.term(
                    cur,
                    Terminator::Return {
                        values: vec![],
                        span,
                    },
                );
                let dead = c.alloc_bb();
                c.cur = dead;
            } else {
                let values = lower_outcome_return_values(val, &mut c);
                let cur = c.cur;
                c.term(cur, Terminator::Return { values, span });
            }
        }
    }

    // Flush owned locals (parameters + let bindings) before building the body.
    // If a `return` already flushed everything, this is a no-op.
    c.pop_scope();

    // A block-form body that falls off the end returns unit.
    // This is a synthetic return — use DUMMY_SPAN since it has no source.
    // Same reachability guard as above: never inject into a dead continuation
    // block left behind by an explicit `return` (ADR-0055 Bug A).
    let cur = c.cur;
    if c.is_open(cur) && (cur == entry || c.block_has_incoming(cur)) {
        c.term(
            cur,
            Terminator::Return {
                values: vec![],
                span: DUMMY_SPAN,
            },
        );
    }

    Ok(c.build(entry))
}

/// Build a [`MirType`] directly from a type annotation.
///
/// Single producer of all MIR types — no String intermediate, no parse() round-trip.
/// Named types are classified as builtins, user structs, or user enums via
/// `TypeKind` symbol-table.
fn lower_type(
    arena: &Arena,
    id: TypeId,
    symbols: &std::collections::HashMap<String, TypeKind>,
    self_type: Option<&MirType>,
) -> MirType {
    match &arena.type_expression(id).node {
        // ADR-0061 GAP-B: `self` receiver / `Self` return type. The marker
        // carries no type of its own — the `implement` block injects its
        // `for_type` as `self_type` from above. Outside an impl context
        // (None) it is Unknown (a stray `self` the typechecker rejected).
        TypeExpr::SelfType => self_type.cloned().unwrap_or(MirType::Unknown),
        TypeExpr::Named(n) => match n.as_str() {
            "Integer" => MirType::Integer,
            "Trit" => MirType::Trit,
            "Tryte" => MirType::Tryte,
            "Long" => MirType::Long,
            "Trilean" => MirType::Trilean,
            "Unit" => MirType::Unit,
            "String" => MirType::String,
            // Vector/HashMap bare (ADR-0050 CORRECTION §3.1.1) — strip generic args
            // Bare `Vector` (no element) defaults to `Vector<Integer>` for
            // Bậc A byte-compat. A written `Vector<E>` parses to
            // `TypeExpr::Generic` (handled below, real element lowered), so the
            // `starts_with("Vector<")` string-form here is a legacy/edge path —
            // default its element to Integer.
            other if other == "Vector" || other.starts_with("Vector<") => {
                MirType::Vector(Box::new(MirType::Integer))
            }
            other if other == "HashMap" || other.starts_with("HashMap<") => {
                MirType::HashMap(Box::new(MirType::Integer), Box::new(MirType::Integer))
            }
            other if symbols.get(other) == Some(&TypeKind::Struct) => {
                MirType::Struct(other.to_string())
            }
            other if symbols.get(other) == Some(&TypeKind::Enum) => {
                MirType::Enum(other.to_string())
            }
            // ADR-0069: capability token type — ZST, always non-copy.
            other if symbols.get(other) == Some(&TypeKind::Capability) => {
                MirType::Capability(other.to_string())
            }
            _ => MirType::Unknown,
        },
        TypeExpr::Nullable(inner) => {
            MirType::Nullable(Box::new(lower_type(arena, *inner, symbols, self_type)))
        }
        TypeExpr::Reference { form, inner } => {
            let inner = lower_type(arena, *inner, symbols, self_type);
            let form = match form {
                ReferenceForm::StrongFrozen => triet_mir::ReferenceForm::StrongFrozen,
                ReferenceForm::StrongMutable => triet_mir::ReferenceForm::StrongMutable,
                ReferenceForm::BorrowReadOnly => triet_mir::ReferenceForm::BorrowReadOnly,
                ReferenceForm::BorrowExclusiveMutable => {
                    triet_mir::ReferenceForm::BorrowExclusiveMutable
                }
                ReferenceForm::WeakObserver => triet_mir::ReferenceForm::WeakObserver,
            };
            MirType::Reference {
                form,
                inner: Box::new(inner),
            }
        }
        TypeExpr::Outcome {
            value_type,
            error_type,
            allow_null_state,
        } => MirType::Outcome {
            value_type: Box::new(lower_type(arena, *value_type, symbols, self_type)),
            error_type: Box::new(lower_type(arena, *error_type, symbols, self_type)),
            allow_null_state: *allow_null_state,
        },
        TypeExpr::Generic { name, arguments } => match name.as_str() {
            // ADR-0077: carry the element type into the MIR Vector. A 0-arg
            // (malformed) Vector falls back to Integer for byte-compat.
            "Vector" => MirType::Vector(Box::new(
                arguments
                    .first()
                    .map(|a| lower_type(arena, *a, symbols, self_type))
                    .unwrap_or(MirType::Integer),
            )),
            // ADR-0080 KM-P1b: key from 1st type argument, value from 2nd.
            // Fallback to Integer for byte-compat (malformed 0/1-arg HashMap)
            // — was hardcoded `Integer` unconditionally pre-ADR-0080, silently
            // dropping an explicit `HashMap<String,V>` annotation's key type.
            "HashMap" => MirType::HashMap(
                Box::new(
                    arguments
                        .first()
                        .map(|a| lower_type(arena, *a, symbols, self_type))
                        .unwrap_or(MirType::Integer),
                ),
                Box::new(
                    arguments
                        .get(1)
                        .map(|a| lower_type(arena, *a, symbols, self_type))
                        .unwrap_or(MirType::Integer),
                ),
            ),
            _ => MirType::Unknown,
        },
        _ => MirType::Unknown,
    }
}

/// Simplified type builder using Ctx layout tables for Struct/Enum discrimination.
///
/// Used in `lower_stmt`/`lower_expr`. Unlike `lower_type`, this does not need
/// the `struct_names`/`enum_names` HashSets — it discriminates user types
/// directly from the layout maps on `Ctx`: if the name is in `enum_layouts`,
/// it's an `Enum`; if in `struct_layouts`, a `Struct`; otherwise `Unknown`
/// (refuse-over-guess).
///
/// TECH-DEBT(B1a S3): merge with `lower_type` when the helper chain is refactored
/// to carry `struct_names`/`enum_names` uniformly.
/// ADR-0066 M-2: Copy classification at construction time, using the lowering
/// `Ctx`'s layout maps (the `Body` is not built yet, so `MirType::is_copy(None)`
/// — which ASSUMES `Struct`/`Enum` are Copy — would leak transitive heap).
/// Recurses through `c.struct_layouts` / `c.enum_layouts`; refuse-over-guess on
/// unknown types (→ Move). Direct heap leaves (`String`/`Vector`/`HashMap`) are
/// Move; scalars/`Reference`/`Outcome`-of-Copy are Copy.
fn ctx_is_copy(ty: &MirType, c: &Ctx) -> bool {
    match ty {
        MirType::Nullable(inner) => ctx_is_copy(inner, c),
        MirType::String | MirType::Vector(_) | MirType::HashMap(..) => false,
        // ADR-0069: capability token — ALWAYS Move. This is the SECOND copy
        // classifier (the first is `MirType::is_copy` in triet-mir); BOTH must
        // short-circuit or the move/Deinit machinery here would treat a ZST
        // token as Copy (the `_ => true` fallthrough) → no tombstone → bypass.
        MirType::Capability(_) => false,
        MirType::Outcome {
            value_type,
            error_type,
            ..
        } => ctx_is_copy(value_type, c) && ctx_is_copy(error_type, c),
        MirType::Struct(name) | MirType::Enum(name) => {
            if let Some(s) = c.struct_layouts.get(name.as_str()) {
                return s.fields.iter().all(|f| ctx_is_copy(&f.ty, c));
            }
            if let Some(e) = c.enum_layouts.get(name.as_str()) {
                return e
                    .variants
                    .iter()
                    .all(|v| v.payload.as_ref().is_none_or(|p| ctx_is_copy(&p.ty, c)));
            }
            false // unknown type → Move (refuse-over-guess)
        }
        _ => true, // scalars, Reference — Copy
    }
}

fn lower_type_simple(arena: &Arena, id: TypeId, c: &Ctx) -> MirType {
    match &arena.type_expression(id).node {
        TypeExpr::Named(n) => match n.as_str() {
            "Integer" => MirType::Integer,
            "Trit" => MirType::Trit,
            "Tryte" => MirType::Tryte,
            "Long" => MirType::Long,
            "Trilean" => MirType::Trilean,
            "Unit" => MirType::Unit,
            "String" => MirType::String,
            // Bare `Vector` (no element) defaults to `Vector<Integer>` for
            // Bậc A byte-compat. A written `Vector<E>` parses to
            // `TypeExpr::Generic` (handled below, real element lowered), so the
            // `starts_with("Vector<")` string-form here is a legacy/edge path —
            // default its element to Integer.
            other if other == "Vector" || other.starts_with("Vector<") => {
                MirType::Vector(Box::new(MirType::Integer))
            }
            other if other == "HashMap" || other.starts_with("HashMap<") => {
                MirType::HashMap(Box::new(MirType::Integer), Box::new(MirType::Integer))
            }
            other if c.struct_layouts.contains_key(other) => MirType::Struct(other.to_string()),
            other if c.enum_layouts.contains_key(other) => MirType::Enum(other.to_string()),
            // ADR-0069: capability token type — ZST, non-copy.
            other if c.capabilities.contains_key(other) => MirType::Capability(other.to_string()),
            _ => MirType::Unknown, // refuse-over-guess
        },
        TypeExpr::Nullable(inner) => {
            MirType::Nullable(Box::new(lower_type_simple(arena, *inner, c)))
        }
        TypeExpr::Reference { form, inner } => {
            let inner = lower_type_simple(arena, *inner, c);
            let form = match form {
                ReferenceForm::StrongFrozen => triet_mir::ReferenceForm::StrongFrozen,
                ReferenceForm::StrongMutable => triet_mir::ReferenceForm::StrongMutable,
                ReferenceForm::BorrowReadOnly => triet_mir::ReferenceForm::BorrowReadOnly,
                ReferenceForm::BorrowExclusiveMutable => {
                    triet_mir::ReferenceForm::BorrowExclusiveMutable
                }
                ReferenceForm::WeakObserver => triet_mir::ReferenceForm::WeakObserver,
            };
            MirType::Reference {
                form,
                inner: Box::new(inner),
            }
        }
        TypeExpr::Outcome {
            value_type,
            error_type,
            allow_null_state,
        } => MirType::Outcome {
            value_type: Box::new(lower_type_simple(arena, *value_type, c)),
            error_type: Box::new(lower_type_simple(arena, *error_type, c)),
            allow_null_state: *allow_null_state,
        },
        TypeExpr::Generic { name, arguments } => match name.as_str() {
            // ADR-0077: carry the element type into the MIR Vector. A 0-arg
            // (malformed) Vector falls back to Integer for byte-compat.
            "Vector" => MirType::Vector(Box::new(
                arguments
                    .first()
                    .map(|a| lower_type_simple(arena, *a, c))
                    .unwrap_or(MirType::Integer),
            )),
            // ADR-0080 KM-P1b: key from 1st type argument, value from 2nd
            // (was hardcoded `Integer` unconditionally pre-ADR-0080).
            "HashMap" => MirType::HashMap(
                Box::new(
                    arguments
                        .first()
                        .map(|a| lower_type_simple(arena, *a, c))
                        .unwrap_or(MirType::Integer),
                ),
                Box::new(
                    arguments
                        .get(1)
                        .map(|a| lower_type_simple(arena, *a, c))
                        .unwrap_or(MirType::Integer),
                ),
            ),
            _ => MirType::Unknown,
        },
        _ => MirType::Unknown,
    }
}

/// Emit a `CallDispatch` terminator targeting a builtin shim, allocate a
/// return local of `dest_ty`, and advance `c.cur` to the return block.
/// Returns the destination local holding the shim's return value.
///
/// WO-ShimTempOwnership (2026-07-19): a shim-call ARGUMENT that the shim
/// only BORROWS (`builtin_shim_meta(shim_name).arg_consumes[i] == false`,
/// OR the shim has NO meta entry at all — e.g. `__triet_string_contains` —
/// treated identically to `false` per the WO mandate) must be registered
/// with `c.push_owned` so its scope-end `Drop` actually fires. Before this,
/// an argument that was a NAMED local (bound via `Stmt::Let`) was already
/// registered there and stayed correctly freed; an ANONYMOUS temp (e.g. the
/// destination of a bare field-access move-out, or a string-literal
/// constant) was never registered anywhere — nothing ever freed it
/// (measured: `concat`/`contains`/`eq` all leaked exactly the un-let-bound
/// args, `heap_shim_temp_leak_counting.rs`). `push_owned` is idempotent
/// (`Ctx::push_owned`, no-op if already registered) and `Drop` of a
/// Copy/Reference-typed local is already a no-op elsewhere in this lowerer
/// (`Stmt::Let` registers every local it binds unconditionally, regardless
/// of type) — so registering EVERY borrowed arg here, named or anonymous,
/// is safe by the same precedent, not a new assumption.
///
/// Args where `arg_consumes[i] == true` (the shim TAKES ownership — e.g.
/// `push`/`insert`) are deliberately SKIPPED: the shim itself transfers the
/// value into a live container, so nothing here should ever free it — doing
/// so would double-free IN THEORY (measured LÀNH/sound already without this,
/// `heap_shim_consuming_temp_counting.rs` — this is the control group this
/// fix must not touch).
///
/// ⚠️ HONESTY NOTE (O+G, 2026-07-19) — this `!consumed` branch is CURRENTLY
/// MASKED, not independently load-bearing: the JIT's own M3 zero-on-consume
/// pass (`crates/triet-jit/src/mir_lower.rs:4717-4718`) ALSO reads
/// `builtin_shim_meta(callee_name).arg_consumes` and unconditionally zeroes
/// every consumed arg's Cranelift variable/StackSlot right after the
/// `CallDispatch`, regardless of whether the lowerer scheduled a `Drop` for
/// it. Measured both ways: with M3 ON, deleting this `!consumed` check
/// entirely (registering `push_owned` for EVERY arg, consumed or not) still
/// reads FREE=1 (no double-free — M3 already zeroed the value, so the
/// wrongly-scheduled `Drop` is a silent no-op). With M3 turned OFF, keeping
/// this `!consumed` check exactly as written STILL double-frees
/// (`free(): double free detected in tcache 2`, SIGABRT) — because this
/// branch alone does not zero anything; it only decides whether to
/// SCHEDULE a `Drop`, and a correctly-skipped schedule is not what stops
/// the shim's OWN internal free from colliding with a stale caller-side
/// pointer once M3 is gone.
///
/// This is NOT textbook defense-in-depth (two independent checks, either
/// one sufficient alone) — it is ONE metadata table
/// (`builtin_shim_meta().arg_consumes`) read by TWO layers (this
/// `push_owned` decision here, M3's zero decision in the JIT) that make
/// DIFFERENT kinds of mistakes if the table lies: an entry that claims
/// BORROW when the shim actually CONSUMES leaks (this layer under-registers
/// `push_owned`, AND M3 under-zeroes — both miss the same way); an entry
/// that claims CONSUME when the shim actually BORROWS double-frees (this
/// layer under-registers correctly by skipping, but M3 zeroes a value the
/// caller still needs, corrupting later use — or in the caller-still-drops
/// direction, this layer's skip PLUS a real free-on-return collide). The
/// table is a single point of failure for BOTH layers, not two
/// independent locks. No test currently canaries `arg_consumes` itself for
/// correctness against the real shim signatures — see the `TODO.md` debt
/// entry (`builtin_shim_meta` SPOF) opened alongside this note. Keep this
/// branch (it is semantically correct — MIR-level ownership should track
/// reality even when a lower layer happens to also cover the failure mode
/// today, and it becomes the ONLY defense if M3 is ever refactored away),
/// but do not describe it in future comments as an independent safety net
/// until a dedicated test proves it fires with M3 disabled.
fn emit_shim_call(
    c: &mut Ctx,
    shim_name: &str,
    args: Vec<Local>,
    dest_ty: impl Into<MirType>,
    span: Span,
) -> Local {
    let meta = triet_mir::builtin_shim_meta(shim_name);
    for (i, &arg) in args.iter().enumerate() {
        let consumed = meta
            .as_ref()
            .is_some_and(|m| i < m.arg_consumes.len() && m.arg_consumes[i]);
        if !consumed {
            c.push_owned(arg);
        }
    }
    let dest = c.alloc_local_ty(dest_ty);
    c.push(Statement::StorageLive(dest, span.clone()));
    let ret_bb = c.alloc_bb();
    let call_bb = c.cur;
    c.term(
        call_bb,
        Terminator::CallDispatch {
            callee: triet_mir::FunctionId(0),
            callee_name: shim_name.into(),
            target: CallTarget::Shim,
            args,
            return_bb: ret_bb,
            dest: vec![dest],
            return_shape: triet_mir::ReturnShape::Scalar,
            span,
        },
    );
    c.cur = ret_bb;
    dest
}

// ── Outcome helpers ─────────────────────────────────────────

/// Expand an Outcome local into its constituent return values
/// `[disc, payload]` by loading from the 16-byte slot. Non-Outcome
/// values pass through as single-element vec.
fn lower_outcome_return_values(val: Local, c: &mut Ctx) -> Vec<Local> {
    let ty = &c.local_decls[val.0].ty;
    if !matches!(ty, MirType::Outcome { .. }) {
        return vec![val];
    }
    // ADR-0058 Lát 1: heap Outcome → sret, return the slot whole.
    if ty.has_heap_payload() {
        return vec![val];
    }
    // Load disc@0 into a temp.
    let disc_tmp = c.alloc_local_ty(MirType::Trit);
    c.push(Statement::StorageLive(disc_tmp, DUMMY_SPAN));
    c.push(Statement::Assign {
        dest: Place::local(disc_tmp),
        source: Place::local(val).project(Projection::OutcomeDiscriminant),
        span: DUMMY_SPAN,
    });
    // Load payload@8 into a temp.
    let payload_tmp = c.alloc_local_ty(MirType::Unknown);
    c.push(Statement::StorageLive(payload_tmp, DUMMY_SPAN));
    c.push(Statement::Assign {
        dest: Place::local(payload_tmp),
        source: Place::local(val).project(Projection::OutcomePayload),
        span: DUMMY_SPAN,
    });
    vec![disc_tmp, payload_tmp]
}

/// Route a struct return value through the sret buffer: copy each top-level
/// field of `val` into `*sret`. Returns `true` if the copy was emitted (caller
/// must emit `Return(())`), `false` if `val` is not a struct-sret return
/// (caller emits the by-value / Outcome `Return` unchanged).
///
/// SSOT for sret struct routing — shared by `Stmt::Return` and tail-Return.
/// Emits no terminator, no flush, and no dead block: each caller owns those
/// because their flush/dead-block sites differ.
fn emit_struct_sret_copy(c: &mut Ctx, val: Local, span: &Span) -> bool {
    let Some(sret) = c.sret_ptr else {
        return false;
    };
    // ADR-0065 §14 (WO-2 Lát A): `Nullable(Struct)` return — the sret buffer
    // is `{tag@0, fields@8+}` (Phương án A, §3.2), not the plain struct's
    // bare field layout the per-field loop below assumes. A single
    // whole-place Assign lets the JIT's Construction Taxonomy
    // (`nullable_struct_taxonomy`, `mir_lower.rs:1199-1237`) dispatch the tag
    // write / +8 shift itself: present arm (`val` a plain Struct, built by
    // the OutcomeConstructor Positive-arm leaf consumer) → case 2 Widen (set
    // tag=present, copy fields src+0 -> dest+8); null arm (`val` already
    // `Nullable(Struct)`, tag already written to ITS OWN slot by the Zero-arm
    // Const) → case 1 WholeCopy (tag-first verbatim, N+8 bytes). Both cases
    // reuse `copy_base_addr`'s existing pointer-fallback for `sret` (Local(0)
    // has no struct_slots entry — pointer-based, same mechanism already
    // proven for a `Struct?` PARAM, WO-StructParamABI). Kept as a SEPARATE
    // branch (not folded into the per-field loop below) so the existing
    // plain-`Struct` sret path — exercised by fixture 14 and siblings — is
    // byte-for-byte unchanged (implementer's choice, O 2026-07-20: "an toàn
    // hơn: giữ nguyên nhánh plain-Struct, thêm nhánh Nullable(Struct)").
    let sret_ty = c.local_decls[sret.0].ty.clone();
    if let MirType::Nullable(inner) = &sret_ty
        && matches!(inner.as_ref(), MirType::Struct(_))
    {
        c.push(Statement::Assign {
            dest: Place::local(sret),
            source: Place::local(val),
            span: span.clone(),
        });
        return true;
    }
    let source_ty = c.local_decls[val.0].ty.clone();
    if !matches!(source_ty, MirType::Struct(_)) {
        return false;
    }
    // Copy each field into the caller's sret buffer. A struct return with no
    // registered layout still routes through sret (Return(())), matching the
    // pre-refactor behavior — the absent layout simply emits no field copies.
    if let Some(layout) = c.struct_layouts.get(&source_ty.to_string()) {
        let field_names: Vec<String> = layout.fields.iter().map(|f| f.name.clone()).collect();
        for field_name in field_names {
            let dest_place = Place::local(sret).project(Projection::Field(field_name.clone()));
            let source_place = Place::local(val).project(Projection::Field(field_name));
            c.push(Statement::Assign {
                dest: dest_place,
                source: source_place,
                span: span.clone(),
            });
        }
    }
    true
}

// ── Block lowering ──────────────────────────────────────────

/// Lower a block expression (statements + optional final expression) into
/// the current block, discarding the block's value.
fn lower_block(block_expr: ExprId, arena: &Arena, c: &mut Ctx) -> Result<(), LowerError> {
    c.push_scope();
    match &arena.expression(block_expr).node {
        Expr::Block {
            statements,
            final_expr,
        } => {
            for &stmt_id in statements {
                let spanned_stmt = arena.statement(stmt_id);
                let stmt = spanned_stmt.node.clone();
                let stmt_span = spanned_stmt.span.clone();
                lower_stmt(&stmt, stmt_span, arena, c)?;
            }
            if let Some(e) = final_expr {
                lower_expr(*e, None, arena, c)?;
            }
        }
        _ => {
            lower_expr(block_expr, None, arena, c)?;
        }
    }
    c.pop_scope();
    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────

/// Returns `true` if the expression is a null literal (`null` keyword
/// or `~0` OutcomeConstructor with Zero arm and no payload).
fn is_null_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::NullLiteral)
        || matches!(
            expr,
            Expr::OutcomeConstructor {
                arm: triet_syntax::OutcomeArm::Zero,
                payload: None
            }
        )
}

// ── Statement lowering ──────────────────────────────────────

fn lower_stmt(stmt: &Stmt, stmt_span: Span, arena: &Arena, c: &mut Ctx) -> Result<(), LowerError> {
    match stmt {
        Stmt::Let {
            name,
            init,
            type_annotation,
            ..
        } => {
            // ── NullLiteral ~0 with expected type ──
            // Under PA-3c uniform, ~0 is always iconst(MIN) — type-agnostic.
            // The annotation tells us the local's type so is_copy/is_nullable_type
            // delegation works downstream.
            if is_null_expr(&arena.expression(*init).node) {
                let ann_ty = type_annotation
                    .as_ref()
                    .map(|tid| lower_type_simple(arena, *tid, c))
                    .ok_or_else(|| {
                        LowerError::null_literal_without_expected_type(stmt_span.clone())
                    })?;
                let d = c.alloc_local_ty(ann_ty.clone());
                c.push(Statement::StorageLive(d, stmt_span.clone()));
                c.push(Statement::Const {
                    dest: Place::local(d),
                    value: ConstValue::Integer(i128::from(triet_mir::NULL_SENTINEL)),
                    span: stmt_span.clone(),
                });
                c.vars.insert(name.clone(), d);
                c.local_names.insert(d, name.clone());
                c.push_owned(d);
            } else {
                // ADR-0072 §2.3/§2.4: let-init — the annotation is the expected
                // type. `~+ v` / `~0` now consume it in the leaf-consumer (no
                // bolt-on redirect); the widening block below still performs the
                // nullable-aggregate construct on the lowered payload local.
                let ann_ty_opt = type_annotation.map(|tid| lower_type_simple(arena, tid, c));
                let init_id = *init;
                let v = lower_expr(init_id, ann_ty_opt.as_ref(), arena, c)?;
                // ── Widening: override init local's type from annotation ──
                // `let x: Integer? = 5` → x should be typed "Integer?" not "?".
                // Under PA-3c widening is identity (same i64 value), so we only
                // need to fix the type string for downstream is_copy/is_nullable_type.
                // TODO(heap-nullable): when String?/Vector? arrive, overriding
                // the init local's type in-place mutates the source local when
                // init is an Identifier (x aliases the same local as y in
                // `let y = ...; let x: T? = y`). Safe today because widening
                // is a no-op and all nullable types are Copy. When heap-nullable
                // arrives, emit an Assign to a new typed local (M2 pattern)
                // instead of mutating.
                if let Some(tid) = type_annotation {
                    let ann_ty = lower_type_simple(arena, *tid, c);
                    // ADR-0065 Lát 2 / Delta 0: aggregate-widening repr-change.
                    // `let x: Struct? = y` (y a plain Struct) CANNOT retype
                    // in-place: the `Struct?` repr prepends a tag word (slot =
                    // total_size + 8, fields at +8), so widening is NOT the no-op
                    // that scalar / Enum?-niche / String?-sentinel widening is
                    // (those share the slot — keep them in-place; fixture 229
                    // etc. must stay green). Emit a genuine Assign to a fresh
                    // Nullable(Struct) local — the "M2 pattern" the TODO above
                    // prescribes — which fires the JIT's Delta 4a widening.
                    let is_struct_widening = matches!(&c.local_decls[v.0].ty, MirType::Struct(_))
                        && matches!(&ann_ty,
                            MirType::Nullable(inner) if matches!(**inner, MirType::Struct(_)));
                    if is_struct_widening {
                        let new_local = c.alloc_local_ty(ann_ty);
                        c.push(Statement::Assign {
                            dest: Place::local(new_local),
                            source: Place::local(v),
                            span: stmt_span.clone(),
                        });
                        c.vars.insert(name.clone(), new_local);
                        c.local_names.insert(new_local, name.clone());
                        c.push_owned(new_local);
                        return Ok(());
                    }
                    if ann_ty != MirType::Unknown {
                        c.local_decls[v.0].ty = ann_ty;
                    }
                }
                // M2 + ADR-0066 1c: If init is an Identifier of a Move type,
                // emit Assign + new local instead of aliasing. This creates a
                // genuine move-site so JIT's Zeroing-on-Move (M1) can zero the
                // source variable. Use `ctx_is_copy` (recurses the layout maps),
                // NOT `is_copy(None)` (assumes Struct/Enum Copy → a heap-struct
                // `let q = p` would alias = pseudo-copy, p usable after move).
                // Clone the type first to release the `c.local_decls` borrow
                // before the `&c` call.
                let is_move_binding =
                    if let Expr::Identifier { name: _ } = &arena.expression(*init).node {
                        let ty = c.local_decls[v.0].ty.clone();
                        !ctx_is_copy(&ty, c)
                    } else {
                        false
                    };
                if is_move_binding {
                    let new_local = c.alloc_local_ty(c.local_decls[v.0].ty.clone());
                    c.push(Statement::Assign {
                        dest: Place::local(new_local),
                        source: Place::local(v),
                        span: stmt_span.clone(),
                    });
                    // ADR-0066 1c (D2): tombstone the source RIGHT AFTER the
                    // move-Assign — ATOMIC, same basic block, no statement
                    // between. The JIT's Deinit struct-walk (1b/C) zeros the
                    // source's heap-field ptrs → its scope-end Drop is a no-op →
                    // no double-free. (For String, Deinit zeros the slot ptr;
                    // unchanged from the existing String move-binding path.)
                    c.push(Statement::Deinit(v, stmt_span.clone()));
                    c.vars.insert(name.clone(), new_local);
                    c.local_names.insert(new_local, name.clone());
                    c.push_owned(new_local);
                } else {
                    c.vars.insert(name.clone(), v);
                    c.local_names.insert(v, name.clone());
                    c.push_owned(v);
                }
            }
        }
        Stmt::Expression { expr } => {
            lower_expr(*expr, None, arena, c)?;
        }
        Stmt::Return { value } => {
            if let (Some(_), Some(v)) = (c.sret_ptr, value) {
                // Fat return via sret. Struct → copy fields into the caller's
                // buffer (emit_struct_sret_copy); String/heap-Outcome → M4
                // escape Return[s] (JIT writes {ptr,len,cap} from slot to sret).
                // ADR-0072 §2.3: sret return position is a value-context SOURCE —
                // forward the function return type as expected (fixture 156:
                // `return ~+ 5` from an `Integer~String` heap-Outcome fn).
                let ret_ty = c.sig.return_type.clone();
                let struct_local = lower_expr(*v, Some(&ret_ty), arena, c)?;
                if emit_struct_sret_copy(c, struct_local, &stmt_span) {
                    c.flush_all_for_return();
                    let cur = c.cur;
                    c.term(
                        cur,
                        Terminator::Return {
                            values: Vec::new(),
                            span: stmt_span.clone(),
                        },
                    );
                    let dead = c.alloc_bb();
                    c.cur = dead;
                } else {
                    // ADR-0049 Lát 6 Lối d: String sret — emit Return[s]
                    // (M4 escape). JIT Return handler writes {ptr,len,cap}
                    // from slot to sret buffer.
                    c.flush_all_for_return();
                    let cur = c.cur;
                    c.term(
                        cur,
                        Terminator::Return {
                            values: vec![struct_local],
                            span: stmt_span.clone(),
                        },
                    );
                    let dead = c.alloc_bb();
                    c.cur = dead;
                }
            } else {
                let mut values = Vec::new();
                if let Some(v) = value {
                    // ADR-0072 §2.3: return position — the function return type is
                    // the expected type. `~0`/`~+`/`null` consume it in the
                    // leaf-consumer (§2.4); the is_null special-case is gone.
                    let ret_ty = c.sig.return_type.clone();
                    let val = lower_expr(*v, Some(&ret_ty), arena, c)?;
                    values.extend(lower_outcome_return_values(val, c));
                }
                // Drop owned locals before Return, so the borrowck can
                // flag dangling references (E2450) on the return value.
                c.flush_all_for_return();
                let cur = c.cur;
                c.term(
                    cur,
                    Terminator::Return {
                        values,
                        span: stmt_span.clone(),
                    },
                );
                let dead = c.alloc_bb();
                c.cur = dead;
            }
        }
        Stmt::Assignment { target, value } => {
            let v = lower_expr(*value, None, arena, c)?;
            match &arena.expression(*target).node {
                // Simple identifier target: emit an Assign to update the
                // original local in-place, so loop headers reading from the
                // same local see the updated value. Without this, `x = x - 1`
                // inside a while loop would rebind x to a new local while the
                // loop header still reads the old one → infinite loop.
                Expr::Identifier { name } => {
                    if let Some(&orig) = c.vars.get(name) {
                        c.push(Statement::Assign {
                            dest: Place::local(orig),
                            source: Place::local(v),
                            span: stmt_span.clone(),
                        });
                        // Keep the var map pointing to orig — the Assign
                        // updated it in-place, and the loop header reads
                        // from the same local.
                    } else {
                        // Variable not yet defined — shouldn't happen after
                        // typecheck, but treat as a new binding.
                        c.vars.insert(name.clone(), v);
                    }
                }
                // Field/projection target: store through the place.
                _ => {
                    let dest = lower_place(*target, arena, c)?;
                    c.push(Statement::Assign {
                        dest,
                        source: Place::local(v),
                        span: stmt_span.clone(),
                    });
                }
            }
        }
        Stmt::While {
            condition, body, ..
        } => {
            let hdr = c.alloc_bb();
            let bdy = c.alloc_bb();
            let ext = c.alloc_bb();

            let cur = c.cur;
            c.term(
                cur,
                Terminator::Goto {
                    target: hdr,
                    span: DUMMY_SPAN,
                },
            );

            c.cur = hdr;
            let cond = lower_expr(*condition, None, arena, c)?;
            let hdr_end = c.cur;
            c.term(
                hdr_end,
                Terminator::If {
                    cond,
                    positive_bb: bdy,
                    zero_bb: None,
                    negative_bb: ext,
                    span: arena.expression(*condition).span.clone(),
                },
            );

            c.cur = bdy;
            lower_block(*body, arena, c)?;
            let bdy_end = c.cur;
            c.term(
                bdy_end,
                Terminator::Goto {
                    target: hdr,
                    span: DUMMY_SPAN,
                },
            );

            c.cur = ext;
        }
        // Const / Break / Continue / For / Loop not yet lowered.
        other => return Err(LowerError::unsupported_stmt(other, stmt_span)),
    }
    Ok(())
}

// ── Place lowering ──────────────────────────────────────────

/// Lower an lvalue expression to a [`Place`]. Identifiers map to their local;
/// `obj.field` appends a `Field` projection. Non-place expressions are
/// materialized into a temporary and referred to as a bare local.
fn lower_place(expr_id: ExprId, arena: &Arena, c: &mut Ctx) -> Result<Place, LowerError> {
    match &arena.expression(expr_id).node {
        Expr::Identifier { name } => {
            let &local = c.vars.get(name).ok_or_else(|| {
                LowerError::undefined_local(name, arena.expression(expr_id).span.clone())
            })?;
            Ok(Place::local(local))
        }
        Expr::FieldAccess { object, field } => {
            let base = lower_place(*object, arena, c)?;
            // ADR-0084 Slice 1a: auto-deref a reference exactly one layer
            // before projecting the field, so `r.x` (r: `&0 T`/`&0 mutable
            // T`) reads through the pointer instead of treating the
            // reference's own slot as `T`. `place_result_type` resolves
            // `base`'s type through any Field/Deref projections already
            // applied by an outer recursive call (e.g. the nested
            // `(&0 outer).inner.x` chain — only the FIRST step ever sees a
            // `Reference`, since a plain struct field is never itself
            // stored as a pointer).
            let base = if matches!(place_result_type(&base, c), MirType::Reference { .. }) {
                base.project(Projection::Deref)
            } else {
                base
            };
            Ok(base.project(Projection::Field(field.clone())))
        }
        _ => {
            let temp = lower_expr(expr_id, None, arena, c)?;
            Ok(Place::local(temp))
        }
    }
}

/// ADR-0065 §12: resolve the result type of a projected `Place` by walking its
/// `Field` projections through the struct layouts. Unwraps a nullable aggregate
/// (`Struct?`/`Enum?`) before descending so a field of a `Point?` resolves
/// against `Point`. Returns `MirType::Unknown` for any projection it cannot
/// resolve (non-aggregate base, unknown struct/field, non-`Field` projection) —
/// callers fall back to the legacy untyped temp in that case.
fn place_result_type(place: &Place, c: &Ctx) -> MirType {
    let mut ty = c.local_decls[place.local.0].ty.clone();
    for proj in &place.projection {
        match proj {
            // ADR-0084 Slice 1a: a reference dereferences to its pointee
            // type. Only ever the first projection of a chain (a plain
            // struct field is never itself stored as a pointer).
            Projection::Deref => {
                let MirType::Reference { inner, .. } = &ty else {
                    return MirType::Unknown;
                };
                ty = (**inner).clone();
            }
            Projection::Field(name) => {
                // Unwrap a nullable aggregate to reach the inner struct's fields.
                let inner = ty.nullable_payload().unwrap_or(&ty).clone();
                let MirType::Struct(sname) = &inner else {
                    return MirType::Unknown;
                };
                match c
                    .struct_layouts
                    .get(sname)
                    .and_then(|layout| layout.fields.iter().find(|f| &f.name == name))
                {
                    Some(field) => ty = field.ty.clone(),
                    None => return MirType::Unknown,
                }
            }
            _ => return MirType::Unknown,
        }
    }
    ty
}

/// ADR-0072 §2.4 / ADR-0052: emit a zero Outcome — a 2-slot StackSlot
/// `{disc: Trit(0), payload: 0}`. Shared SSOT for `~0` in an Outcome context
/// (`OutcomeConstructor` Zero arm) and the deprecated `null` keyword
/// (`NullLiteral`) when the expected type is an Outcome. Emits exactly the
/// statement sequence both call-sites used inline (byte-identical).
fn emit_outcome_zero(c: &mut Ctx, outcome_ty: MirType, span: &Span) -> Local {
    let outcome = c.alloc_local_ty(outcome_ty);
    c.push(Statement::StorageLive(outcome, span.clone()));
    c.push(Statement::OutcomeAlloc {
        dest: outcome,
        span: span.clone(),
    });
    // disc = Trit(0).
    let disc_tmp = c.alloc_local_ty(MirType::Trit);
    c.push(Statement::StorageLive(disc_tmp, span.clone()));
    c.push(Statement::Const {
        dest: Place::local(disc_tmp),
        value: ConstValue::Trit(0),
        span: span.clone(),
    });
    c.push(Statement::Assign {
        dest: Place::local(outcome).project(Projection::OutcomeDiscriminant),
        source: Place::local(disc_tmp),
        span: span.clone(),
    });
    // payload = 0 (don't-care — ~0 has no associated data).
    let payload_tmp = c.alloc_local_ty(MirType::Integer);
    c.push(Statement::StorageLive(payload_tmp, span.clone()));
    c.push(Statement::Const {
        dest: Place::local(payload_tmp),
        value: ConstValue::Integer(0),
        span: span.clone(),
    });
    c.push(Statement::Assign {
        dest: Place::local(outcome).project(Projection::OutcomePayload),
        source: Place::local(payload_tmp),
        span: span.clone(),
    });
    outcome
}

// ── Expression lowering ─────────────────────────────────────

fn lower_expr(
    expr_id: ExprId,
    expected: Option<&MirType>, // ADR-0072 Slice 1: plumbing only, not yet consumed
    arena: &Arena,
    c: &mut Ctx,
) -> Result<Local, LowerError> {
    let expr_span = arena.expression(expr_id).span.clone();
    match &arena.expression(expr_id).node {
        Expr::IntegerLiteral { value, suffix } => {
            // Latent type-inference: stamp the local with the suffix-implied
            // scalar type so a `match` scrutinee bound to this literal reaches
            // the correct value-keyed dispatch (Trit/Integer) instead of Unknown.
            let ty = match suffix {
                None | Some(triet_syntax::NumericSuffix::Integer) => MirType::Integer,
                Some(triet_syntax::NumericSuffix::Trit) => MirType::Trit,
                Some(triet_syntax::NumericSuffix::Tryte) => MirType::Tryte,
                Some(triet_syntax::NumericSuffix::Long) => MirType::Long,
            };
            let d = c.alloc_local_ty(ty);
            c.push(Statement::StorageLive(d, expr_span.clone()));
            c.push(Statement::Const {
                dest: Place::local(d),
                value: ConstValue::Integer(*value),
                span: expr_span,
            });
            Ok(d)
        }
        Expr::TernaryLiteral { value } => {
            let d = c.alloc_local_ty(MirType::Integer);
            c.push(Statement::StorageLive(d, expr_span.clone()));
            c.push(Statement::Const {
                dest: Place::local(d),
                value: ConstValue::Integer(*value),
                span: expr_span,
            });
            Ok(d)
        }
        Expr::TritLiteral { value } => {
            let d = c.alloc_local_ty(MirType::Trit);
            c.push(Statement::StorageLive(d, expr_span.clone()));
            c.push(Statement::Const {
                dest: Place::local(d),
                value: ConstValue::Trit(*value),
                span: expr_span,
            });
            Ok(d)
        }
        Expr::TrileanLiteral { value } => {
            let d = c.alloc_local_ty(MirType::Trilean);
            c.push(Statement::StorageLive(d, expr_span.clone()));
            let trit: i8 = match value {
                triet_syntax::TrileanValue::True => 1,
                triet_syntax::TrileanValue::False => -1,
                triet_syntax::TrileanValue::Unknown => 0,
            };
            c.push(Statement::Const {
                dest: Place::local(d),
                value: ConstValue::Trit(trit),
                span: expr_span,
            });
            Ok(d)
        }
        Expr::NullLiteral => {
            // ADR-0072 §2.4: the deprecated `null` keyword is a LEAF consumer of
            // the expected type. STRICT (Slice 3 — no fallback): no expected type
            // from context → error (typecheck should have caught it).
            let Some(ctx_ty) = expected else {
                return Err(LowerError::null_literal_without_expected_type(expr_span));
            };
            match ctx_ty {
                MirType::Nullable(_) => {
                    // PA-3c null repr → NULL_SENTINEL of the expected type.
                    let d = c.alloc_local_ty(ctx_ty.clone());
                    c.push(Statement::StorageLive(d, expr_span.clone()));
                    c.push(Statement::Const {
                        dest: Place::local(d),
                        value: ConstValue::Integer(i128::from(triet_mir::NULL_SENTINEL)),
                        span: expr_span,
                    });
                    Ok(d)
                }
                MirType::Outcome { .. } => {
                    // Outcome-zero: 2-slot StackSlot {disc: Trit(0), payload: 0}.
                    Ok(emit_outcome_zero(c, ctx_ty.clone(), &expr_span))
                }
                _ => Err(LowerError::null_literal_without_expected_type(expr_span)),
            }
        }
        // ADR-0069: `mint Cap` → a ZST capability token. 0 byte at runtime —
        // no heap, no shim, no stack slot (the JIT keeps it in a plain i64
        // Variable holding a dummy 0; the value is never read meaningfully).
        // The local is typed `Capability(name)` so it is non-copy: passing it
        // to a function moves it (Deinit tombstone), and reuse-after-move is
        // E2420 (borrowck, keyed off `is_copy`). Typecheck (`check_mint`) has
        // already verified the capability exists and is mintable (Grant/Defer).
        Expr::Mint { capability_name } => {
            // Lát 3: a `defer` mint emits a runtime capability gate BEFORE the
            // ZST init (§5 LOCK — check at the mint site, fail-closed). Grant
            // mints emit NOTHING extra (Lát 0 zero-cost path, unchanged).
            if matches!(
                c.capabilities.get(capability_name.as_str()),
                Some(CapabilityLevel::Defer)
            ) {
                c.push(Statement::CapabilityCheck {
                    capability_name: capability_name.clone(),
                    span: expr_span.clone(),
                });
            }
            let d = c.alloc_local_ty(MirType::Capability(capability_name.clone()));
            c.push(Statement::StorageLive(d, expr_span.clone()));
            c.push(Statement::Const {
                dest: Place::local(d),
                value: ConstValue::Integer(0),
                span: expr_span,
            });
            Ok(d)
        }
        Expr::OutcomeConstructor { arm, payload } => {
            // ── ADR-0052 §3.2 + OP.3.5: StackSlot 16-byte constructor ──
            // Outcome = 1 local → StackSlot {disc@0, payload@8}.
            // disc encoding: Positive=1, Negative=−1 (Trit).
            // Zero arm (T?~E) deferred to later OP.
            // Payload = value/error scalar (Bậc A — heap deferred).

            // ADR-0072 §2.4: decide the lowering path by the EXPECTED type, NOT
            // the function return type. STRICT (Slice 3 — no fallback):
            // `~+`/`~0`/`~-` in a position with no expected type (e.g. an operand)
            // → error (typecheck should have caught it).
            let Some(ctx_ty) = expected.cloned() else {
                return Err(LowerError::null_literal_without_expected_type(expr_span));
            };

            // Nullable context → PA-3c nullable repr (NO OutcomeAlloc).
            if let MirType::Nullable(inner) = &ctx_ty {
                // HOTFIX (2026-07-15, O recon): disc-niche nullable repr
                // ("present value IS the repr" below) is sound ONLY for
                // unit-only enums (disc@0 IS the whole 8-byte value). An
                // enum with a payload-bearing variant is >8B (disc@0 +
                // payload@8) and this repr silently truncates/misreads it
                // → SIGILL 132 at runtime. Refuse before it reaches MIR.
                if let MirType::Enum(enum_name) = inner.as_ref()
                    && let Some(layout) = c.enum_layouts.get(enum_name)
                    && layout.variants.iter().any(|v| v.payload.is_some())
                {
                    return Err(LowerError::nullable_enum_payload_unsupported(
                        enum_name, None, expr_span,
                    ));
                }
                return match arm {
                    triet_syntax::OutcomeArm::Positive => {
                        let Some(p) = payload else {
                            return Err(LowerError::unsupported_expr(
                                &arena.expression(expr_id).node,
                                expr_span,
                            ));
                        };
                        // Payload-plain: present scalar value IS the repr (PA-3c);
                        // the parent value-context performs any nullable widening.
                        let inner_ty = inner.as_ref().clone();
                        lower_expr(*p, Some(&inner_ty), arena, c)
                    }
                    triet_syntax::OutcomeArm::Zero => {
                        let d = c.alloc_local_ty(ctx_ty.clone());
                        c.push(Statement::StorageLive(d, expr_span.clone()));
                        c.push(Statement::Const {
                            dest: Place::local(d),
                            value: ConstValue::Integer(i128::from(triet_mir::NULL_SENTINEL)),
                            span: expr_span,
                        });
                        Ok(d)
                    }
                    triet_syntax::OutcomeArm::Negative => {
                        // `~-` on a `T?` — typecheck should have rejected this.
                        Err(LowerError::null_literal_without_expected_type(expr_span))
                    }
                };
            }

            // Non-wrapper context → cannot build an Outcome/nullable value here.
            if !matches!(ctx_ty, MirType::Outcome { .. }) {
                return Err(LowerError::null_literal_without_expected_type(expr_span));
            }

            // ~0 (Zero arm) — constructor for T?~E ternary Outcome.
            // Has no payload — disc = Trit(0), payload = 0 (don't-care).
            if matches!(arm, triet_syntax::OutcomeArm::Zero) {
                return Ok(emit_outcome_zero(c, ctx_ty.clone(), &expr_span));
            }

            let payload_ty = if let MirType::Outcome {
                ref value_type,
                ref error_type,
                ..
            } = ctx_ty
            {
                match arm {
                    triet_syntax::OutcomeArm::Positive => value_type.as_ref().clone(),
                    triet_syntax::OutcomeArm::Negative => error_type.as_ref().clone(),
                    triet_syntax::OutcomeArm::Zero => MirType::Unknown,
                }
            } else {
                MirType::Unknown
            };

            let disc_value: i8 = match arm {
                triet_syntax::OutcomeArm::Positive => 1,
                triet_syntax::OutcomeArm::Negative => -1,
                triet_syntax::OutcomeArm::Zero => 0, // unreachable (caught above)
            };

            // Allocate single Outcome local with 16-byte StackSlot.
            let outcome_ty = ctx_ty.clone();
            let outcome = c.alloc_local_ty(outcome_ty);
            c.push(Statement::StorageLive(outcome, expr_span.clone()));
            c.push(Statement::OutcomeAlloc {
                dest: outcome,
                span: expr_span.clone(),
            });

            // Store disc at offset 0.
            let disc_tmp = c.alloc_local_ty(MirType::Trit);
            c.push(Statement::StorageLive(disc_tmp, expr_span.clone()));
            c.push(Statement::Const {
                dest: Place::local(disc_tmp),
                value: ConstValue::Trit(disc_value),
                span: expr_span.clone(),
            });
            c.push(Statement::Assign {
                dest: Place::local(outcome).project(Projection::OutcomeDiscriminant),
                source: Place::local(disc_tmp),
                span: expr_span.clone(),
            });

            // Lower payload into the Outcome slot.
            if let Some(payload_expr) = payload {
                let val = lower_expr(*payload_expr, None, arena, c)?;
                let val_ty = &c.local_decls[val.0].ty;
                if val_ty.is_any_heap() {
                    // HP.1: heap payload → store {ptr@8, len@16, cap@24}.
                    let ptr_tmp = c.alloc_local_ty(MirType::Integer);
                    c.push(Statement::StorageLive(ptr_tmp, expr_span.clone()));
                    c.push(Statement::Assign {
                        dest: Place::local(ptr_tmp),
                        source: Place::local(val).project(Projection::Field("ptr".to_string())),
                        span: expr_span.clone(),
                    });
                    c.push(Statement::Assign {
                        dest: Place::local(outcome).project(Projection::OutcomePayload),
                        source: Place::local(ptr_tmp),
                        span: expr_span.clone(),
                    });

                    let len_tmp = c.alloc_local_ty(MirType::Integer);
                    c.push(Statement::StorageLive(len_tmp, expr_span.clone()));
                    c.push(Statement::Assign {
                        dest: Place::local(len_tmp),
                        source: Place::local(val).project(Projection::Field("len".to_string())),
                        span: expr_span.clone(),
                    });
                    c.push(Statement::Assign {
                        dest: Place::local(outcome).project(Projection::OutcomePayloadLen),
                        source: Place::local(len_tmp),
                        span: expr_span.clone(),
                    });

                    let cap_tmp = c.alloc_local_ty(MirType::Integer);
                    c.push(Statement::StorageLive(cap_tmp, expr_span.clone()));
                    c.push(Statement::Assign {
                        dest: Place::local(cap_tmp),
                        source: Place::local(val).project(Projection::Field("cap".to_string())),
                        span: expr_span.clone(),
                    });
                    c.push(Statement::Assign {
                        dest: Place::local(outcome).project(Projection::OutcomePayloadCap),
                        source: Place::local(cap_tmp),
                        span: expr_span.clone(),
                    });
                    // WO-OutcomeEarlyReturnHeapPayload root cause (Mentor O +
                    // G, 2026-07-02/03): this site is shared by EVERY
                    // `~+ v`/`~- e` with a heap payload in the language, not
                    // just `~->` early-return. It copied {ptr,len,cap} out of
                    // `val` but never tombstoned `val` — harmless for a
                    // literal/temp payload (no drop obligation), but when
                    // `val` is a named-local with a drop obligation (e.g. the
                    // Site-B `e_local` bind), its later scope-exit Drop frees
                    // the same buffer `outcome` now owns → double-free.
                    c.push(Statement::Deinit(val, expr_span.clone()));
                } else {
                    // Scalar payload → store single i64 at offset 8.
                    let payload_tmp = c.alloc_local_ty(payload_ty);
                    c.push(Statement::StorageLive(payload_tmp, expr_span.clone()));
                    c.push(Statement::Assign {
                        dest: Place::local(payload_tmp),
                        source: Place::local(val),
                        span: expr_span.clone(),
                    });
                    c.push(Statement::Assign {
                        dest: Place::local(outcome).project(Projection::OutcomePayload),
                        source: Place::local(payload_tmp),
                        span: expr_span.clone(),
                    });
                }
            } else {
                return Err(LowerError::unsupported_expr(
                    &arena.expression(expr_id).node,
                    expr_span,
                ));
            }

            Ok(outcome)
        }
        Expr::StringLiteral { value } => {
            let d = c.alloc_local_ty(MirType::String);
            c.push(Statement::StorageLive(d, expr_span.clone()));
            c.push(Statement::Const {
                dest: Place::local(d),
                value: ConstValue::String(value.clone()),
                span: expr_span,
            });
            Ok(d)
        }
        Expr::Identifier { name } => {
            // ADR-0071 Lát 2: a bare identifier is ONLY a local binding. A
            // unit enum variant must be qualified (`Enum::Variant`, lowered as
            // `Expr::EnumLiteral`); a bare variant name is undefined here.
            if let Some(&local) = c.vars.get(name) {
                return Ok(local);
            }
            Err(LowerError::undefined_local(name, expr_span))
        }
        Expr::UnaryOp { operator, operand } => {
            let val = lower_expr(*operand, None, arena, c)?;
            let d = c.alloc_local();
            c.push(Statement::StorageLive(d, expr_span.clone()));
            match operator {
                UnaryOperator::Negate => {
                    // -x → 0 - x
                    let zero = c.alloc_local();
                    c.push(Statement::Const {
                        dest: Place::local(zero),
                        value: ConstValue::Integer(0),
                        span: expr_span.clone(),
                    });
                    c.push(Statement::BinaryOp {
                        dest: Place::local(d),
                        op: BinOp::Sub,
                        left: Place::local(zero),
                        right: Place::local(val),
                        span: expr_span.clone(),
                    });
                }
                UnaryOperator::Not | UnaryOperator::KleeneNot => {
                    return Err(LowerError::unsupported_expr(
                        &arena.expression(expr_id).node,
                        expr_span,
                    ));
                }
            }
            Ok(d)
        }
        Expr::BinaryOp {
            operator,
            left,
            right,
        } => {
            let lhs = lower_expr(*left, None, arena, c)?;
            let rhs = lower_expr(*right, None, arena, c)?;
            let ty = binop_result_type(operator);
            let d = c.alloc_local_ty(ty);

            // `Pow` is not an ALU instruction — it must go through the
            // `__triet_pow` shim via CallDispatch (extern "C" SystemV).
            if matches!(operator, BinaryOperator::Pow) {
                c.push(Statement::StorageLive(d, expr_span.clone()));
                let ret_bb = c.alloc_bb();
                let call_bb = c.cur;
                c.term(
                    call_bb,
                    Terminator::CallDispatch {
                        callee: triet_mir::FunctionId(0),
                        callee_name: "__triet_pow".into(),
                        target: CallTarget::Shim,
                        args: vec![lhs, rhs],
                        return_bb: ret_bb,
                        dest: vec![d],
                        return_shape: triet_mir::ReturnShape::Scalar,
                        span: expr_span,
                    },
                );
                c.cur = ret_bb;
                return Ok(d);
            }

            let op = lower_binop(operator);
            c.push(Statement::StorageLive(d, expr_span.clone()));
            match op {
                Some(op) => c.push(Statement::BinaryOp {
                    dest: Place::local(d),
                    op,
                    left: Place::local(lhs),
                    right: Place::local(rhs),
                    span: expr_span,
                }),
                None => c.push(Statement::Assign {
                    dest: Place::local(d),
                    source: Place::local(lhs),
                    span: expr_span,
                }),
            }
            Ok(d)
        }
        Expr::Call { callee, arguments } => {
            // ADR-0071 Lát 2: enum-variant construction (unit + payload) is the
            // qualified `Enum::Variant[(payload)]` form, parsed as
            // `Expr::EnumLiteral` (handled above) — NOT a Call. The old
            // bare-`Variant(args)` resolution and `TypeName.Variant(payload)`
            // dot-form construction are gone; a Call here is a plain
            // function/builtin call.
            let callee_name = match &arena.expression(*callee).node {
                Expr::Identifier { name } => name.clone(),
                other => return Err(LowerError::unsupported_callee(other, expr_span)),
            };

            // ── Builtin shim dispatch (ADR-0040 §3.1 + §5) ──
            // String: concat, eq. String+Vector: len (type-aware).
            // Vector: vector_new, push.
            match callee_name.as_str() {
                "concat" => {
                    let args: Vec<Local> = arguments
                        .iter()
                        .map(|a| lower_expr(*a, None, arena, c))
                        .collect::<Result<Vec<_>, _>>()?;
                    let dest = emit_shim_call(
                        c,
                        "__triet_string_concat",
                        args,
                        MirType::String,
                        expr_span,
                    );
                    return Ok(dest);
                }
                "eq" => {
                    let args: Vec<Local> = arguments
                        .iter()
                        .map(|a| lower_expr(*a, None, arena, c))
                        .collect::<Result<Vec<_>, _>>()?;
                    let dest =
                        emit_shim_call(c, "__triet_string_eq", args, MirType::Integer, expr_span);
                    return Ok(dest);
                }
                "len" | "length" => {
                    // Type-aware dispatch: String → string_len, Vector → vector_len.
                    // ADR-0045 §8: strip reference prefix to accept &0 String etc.
                    // TECH-DEBT(ADR-0045): MIR-type-as-string, xem §3.
                    if arguments.len() != 1 {
                        return Err(LowerError::unsupported_expr(
                            &arena.expression(*callee).node,
                            expr_span,
                        ));
                    }
                    let arg = lower_expr(arguments[0], None, arena, c)?;
                    let arg_ty = &c.local_decls[arg.0].ty;
                    // ADR-0049 Phase-1 Lát 1 B4: length on owned String reads
                    // field-1 (len) from the StackSlot, not from the heap shim.
                    // Borrow (&0 String etc.) keeps the shim — the handle still
                    // points to the heap where len@body+0.
                    if matches!(arg_ty, MirType::String) {
                        // WO-ShimTempOwnership (2026-07-19): this fast path
                        // BYPASSES `emit_shim_call` entirely (no CallDispatch
                        // is ever emitted for the owned-String case), so the
                        // chokepoint fix there does not reach `arg` here — it
                        // needs its own `push_owned`. `length()` only READS
                        // `arg`'s `len` field, never frees/consumes it, so
                        // this is unconditionally a BORROW position — mirrors
                        // `emit_shim_call`'s `arg_consumes[i] == false` arm.
                        // Idempotent for a named (`let`-bound) `arg` (already
                        // registered by `Stmt::Let`); load-bearing for an
                        // anonymous temp (a bare field-access move-out or a
                        // string literal used directly as this argument) —
                        // measured leaking (FREE=0) before this fix.
                        c.push_owned(arg);
                        let d = c.alloc_local_ty(MirType::Integer);
                        c.push(Statement::Assign {
                            dest: Place::local(d),
                            source: Place::local(arg).project(Projection::Field("len".to_string())),
                            span: expr_span.clone(),
                        });
                        return Ok(d);
                    }
                    let base_ty = if let MirType::Reference { inner, .. } = arg_ty {
                        inner.as_ref()
                    } else {
                        arg_ty
                    };
                    let shim_name = match base_ty {
                        MirType::String => "__triet_string_len",
                        ty if ty.is_vec() => "__triet_vector_len",
                        ty if ty.is_hashmap() => "__triet_hashmap_len",
                        other => {
                            return Err(LowerError::heap_type_not_supported(
                                &format!(
                                    "len() on type `{other}` — expected String, Vector, or HashMap"
                                ),
                                expr_span,
                            ));
                        }
                    };
                    let dest = emit_shim_call(c, shim_name, vec![arg], MirType::Integer, expr_span);
                    return Ok(dest);
                }
                "contains" => {
                    // ADR-0047 §1: type-aware dispatch — String/Vector/HashMap.
                    // Lối 1: strip reference prefix, dispatch by TYPE-STRING.
                    if arguments.len() != 2 {
                        return Err(LowerError::unsupported_expr(
                            &arena.expression(*callee).node,
                            expr_span,
                        ));
                    }
                    let args: Vec<Local> = arguments
                        .iter()
                        .map(|a| lower_expr(*a, None, arena, c))
                        .collect::<Result<Vec<_>, _>>()?;
                    let arg0_ty = &c.local_decls[args[0].0].ty;
                    // Lối 1: strip &0 /&+ /&- prefix to get base type.
                    let base_ty = if let MirType::Reference { inner, .. } = arg0_ty {
                        inner.as_ref()
                    } else {
                        arg0_ty
                    };
                    let shim_name = match base_ty {
                        MirType::String => "__triet_string_contains",
                        ty if ty.is_vec() => "__triet_vector_contains",
                        ty if ty.is_hashmap() => "__triet_hashmap_contains",
                        other => {
                            return Err(LowerError::heap_type_not_supported(
                                &format!(
                                    "contains() on type `{other}` — expected String, Vector, or HashMap"
                                ),
                                expr_span,
                            ));
                        }
                    };
                    let dest = emit_shim_call(c, shim_name, args, MirType::Trilean, expr_span);
                    return Ok(dest);
                }
                "is_empty" => {
                    // ADR-0047 §2: derive len(x) == 0 via BinOp::Eq.
                    // Eq returns select(cmp, 1, -1) — correct Trilean encoding.
                    if arguments.len() != 1 {
                        return Err(LowerError::unsupported_expr(
                            &arena.expression(*callee).node,
                            expr_span,
                        ));
                    }
                    let arg = lower_expr(arguments[0], None, arena, c)?;
                    let arg_ty = &c.local_decls[arg.0].ty;

                    // ADR-0049 Lát 6.3: for owned String, read len from
                    // StackSlot (field projection), not heap shim.
                    // Same pattern as `length`/`len`.
                    let len_dest = if matches!(arg_ty, MirType::String) {
                        // WO-1 (2026-07-20): this owned-String fast path
                        // BYPASSES `emit_shim_call` entirely (like
                        // `length`'s fast path above), so the
                        // `emit_shim_call` chokepoint fix does not reach
                        // `arg` here — it needs its own `push_owned`.
                        // `is_empty()` only READS `arg`'s `len` field via
                        // projection (below), never frees/consumes it, so
                        // this is unconditionally a BORROW position.
                        // Idempotent for a named (`let`-bound) `arg`
                        // (already registered by `Stmt::Let`);
                        // load-bearing for an anonymous temp (a bare
                        // field-access move-out or a string literal used
                        // directly as this argument) — measured leaking
                        // (FREE=0) before this fix.
                        c.push_owned(arg);
                        let d = c.alloc_local_ty(MirType::Integer);
                        c.push(Statement::Assign {
                            dest: Place::local(d),
                            source: Place::local(arg).project(Projection::Field("len".to_string())),
                            span: expr_span.clone(),
                        });
                        d
                    } else {
                        let base_ty = if let MirType::Reference { inner, .. } = arg_ty {
                            inner.as_ref()
                        } else {
                            arg_ty
                        };
                        let len_shim = match base_ty {
                            MirType::String => "__triet_string_len",
                            ty if ty.is_vec() => "__triet_vector_len",
                            ty if ty.is_hashmap() => "__triet_hashmap_len",
                            other => {
                                return Err(LowerError::heap_type_not_supported(
                                    &format!(
                                        "is_empty() on type `{other}` — expected String, Vector, or HashMap"
                                    ),
                                    expr_span,
                                ));
                            }
                        };
                        emit_shim_call(c, len_shim, vec![arg], MirType::Integer, expr_span.clone())
                    };
                    // Compare len == 0 → Trilean encoding (Eq returns 1 or -1).
                    let result = c.alloc_local_ty(MirType::Trilean);
                    let zero = c.alloc_local_ty(MirType::Integer);
                    c.push(Statement::StorageLive(result, expr_span.clone()));
                    c.push(Statement::StorageLive(zero, expr_span.clone()));
                    c.push(Statement::Const {
                        dest: Place::local(zero),
                        value: ConstValue::Integer(0),
                        span: expr_span.clone(),
                    });
                    c.push(Statement::BinaryOp {
                        dest: Place::local(result),
                        op: BinOp::Eq,
                        left: Place::local(len_dest),
                        right: Place::local(zero),
                        span: expr_span,
                    });
                    return Ok(result);
                }
                "clear" => {
                    // ADR-0048: in-place clear(&0 mutable String).
                    // Lối 1: strip ref prefix, dispatch by type-string.
                    if arguments.len() != 1 {
                        return Err(LowerError::unsupported_expr(
                            &arena.expression(*callee).node,
                            expr_span,
                        ));
                    }
                    let arg = lower_expr(arguments[0], None, arena, c)?;
                    let arg_ty = &c.local_decls[arg.0].ty;
                    let base_ty = if let MirType::Reference { inner, .. } = arg_ty {
                        inner.as_ref()
                    } else {
                        arg_ty
                    };
                    let shim_name = match base_ty {
                        MirType::String => "__triet_string_clear",
                        other => {
                            return Err(LowerError::heap_type_not_supported(
                                &format!("clear() on type `{other}` — expected String"),
                                expr_span,
                            ));
                        }
                    };
                    let dest = emit_shim_call(c, shim_name, vec![arg], MirType::Integer, expr_span);
                    return Ok(dest);
                }
                "append" => {
                    // ADR-0049 Lát 5: append(&0 mutable String, byte).
                    if arguments.len() != 2 {
                        return Err(LowerError::unsupported_expr(
                            &arena.expression(*callee).node,
                            expr_span,
                        ));
                    }
                    let arg = lower_expr(arguments[0], None, arena, c)?;
                    let byte_arg = lower_expr(arguments[1], None, arena, c)?;
                    let arg_ty = &c.local_decls[arg.0].ty;
                    let base_ty = if let MirType::Reference { inner, .. } = arg_ty {
                        inner.as_ref()
                    } else {
                        arg_ty
                    };
                    let shim_name = match base_ty {
                        MirType::String => "__triet_string_append",
                        other => {
                            return Err(LowerError::heap_type_not_supported(
                                &format!("append() on type `{other}` — expected String"),
                                expr_span,
                            ));
                        }
                    };
                    let dest = emit_shim_call(
                        c,
                        shim_name,
                        vec![arg, byte_arg],
                        MirType::Integer,
                        expr_span,
                    );
                    return Ok(dest);
                }
                "get" => {
                    // Type-aware dispatch with borrow-get (ADR-0079 Slice B).
                    // - Owned container + heap element → value-get (which E1047
                    //   refuses at typecheck level for heap; this is the defense
                    //   path in case typecheck is bypassed).
                    // - &0 container + heap element → get_ref (zero-copy borrow).
                    // - &0 container + Copy element → value-get (returns value).
                    if arguments.len() != 2 {
                        return Err(LowerError::unsupported_expr(
                            &arena.expression(*callee).node,
                            expr_span,
                        ));
                    }
                    let args: Vec<Local> = arguments
                        .iter()
                        .map(|a| lower_expr(*a, None, arena, c))
                        .collect::<Result<Vec<_>, _>>()?;
                    let arg0_ty = &c.local_decls[args[0].0].ty;
                    let is_borrow = matches!(arg0_ty, MirType::Reference { .. });
                    let base_ty = if let MirType::Reference { inner, .. } = arg0_ty {
                        inner.as_ref()
                    } else {
                        arg0_ty
                    };
                    // Extract element/value type.
                    let elem_ty = if let MirType::Vector(inner) = base_ty {
                        (**inner).clone()
                    } else if let MirType::HashMap(_, v) = base_ty {
                        (**v).clone()
                    } else {
                        MirType::Integer
                    };
                    // Borrow-get: &0 container with heap element → get_ref shim
                    // returning a nullable slot pointer (zero-copy). The pointer
                    // is represented as Nullable(Integer) at the MIR level — it's
                    // an i64 slot-ptr or NULL_SENTINEL. The borrow semantics are
                    // enforced by the borrowck (U2 PropagatedLoan), not the MIR
                    // type (which would need Nullable(Reference(...)) — unsupported
                    // by the MIR verifier/JIT repr campaign).
                    let is_heap_elem = elem_ty.is_any_heap();
                    // ADR-0082 §AMEND-3: a Struct/Enum element is a FAT
                    // aggregate — `is_any_heap()` doesn't cover it (that
                    // predicate is String/Vector/HashMap only), so without
                    // this branch it would fall to the generic `else` below
                    // and dispatch to the plain i64-return `__triet_vector_get`/
                    // `__triet_hashmap_get` shim, which cannot marshal >8B.
                    let is_aggregate_elem =
                        matches!(elem_ty, MirType::Struct(_) | MirType::Enum(_));
                    // ADR-0079 §AMEND (Slice 2, composes ADR-0084): an
                    // aggregate element now reaches here via TWO distinct
                    // typecheck arms — the owned-container get-by-value arm
                    // (`get(v,i)`, Copy-only, E1049 refuses heap-bearing) AND
                    // the NEW `&0`-borrow get_ref arm (`get(&0 v,i)`, heap-
                    // bearing fully supported). `is_borrow` genuinely
                    // distinguishes the two now — route to the dedicated
                    // `_get_ref_agg` shim (returns cell_ptr, zero-copy,
                    // borrows the container) instead of `_get_copy` (returns
                    // a bitwise copy) when the call is the borrow form. Do
                    // NOT collapse these: `_get_copy` on a `&0` aggregate
                    // call would silently drop the loan (returns_borrow_of:
                    // None) and let the container be mutated/freed out from
                    // under the "borrowed" element (dangling), plus alias the
                    // container's heap pointer for a heap-bearing field
                    // (double-free on drop) — exactly the MINE-1 hazard this
                    // slice closes.
                    let (shim_name, dest_ty) = if is_aggregate_elem && is_borrow {
                        let name = if base_ty.is_hashmap() {
                            "__triet_hashmap_get_ref_agg"
                        } else if base_ty.is_vec() {
                            "__triet_vector_get_ref_agg"
                        } else {
                            return Err(LowerError::heap_type_not_supported(
                                &format!("get() on type `{arg0_ty}` — expected Vector or HashMap"),
                                expr_span,
                            ));
                        };
                        let ref_ty = MirType::Reference {
                            form: triet_mir::ReferenceForm::BorrowReadOnly,
                            inner: Box::new(elem_ty),
                        };
                        (name, MirType::Nullable(Box::new(ref_ty)))
                    } else if is_aggregate_elem {
                        let name = if base_ty.is_hashmap() {
                            "__triet_hashmap_get_copy"
                        } else if base_ty.is_vec() {
                            "__triet_vector_get_copy"
                        } else {
                            return Err(LowerError::heap_type_not_supported(
                                &format!("get() on type `{arg0_ty}` — expected Vector or HashMap"),
                                expr_span,
                            ));
                        };
                        (name, MirType::Nullable(Box::new(elem_ty)))
                    } else if is_borrow && is_heap_elem {
                        let name = if base_ty.is_hashmap() {
                            "__triet_hashmap_get_ref"
                        } else if base_ty.is_vec() {
                            "__triet_vector_get_ref"
                        } else {
                            return Err(LowerError::heap_type_not_supported(
                                &format!("get() on type `{arg0_ty}` — expected Vector or HashMap"),
                                expr_span,
                            ));
                        };
                        // Nullable(Reference): the reference is an i64 pointer
                        // at runtime; the borrow semantics are enforced by the
                        // borrowck (U2 PropagatedLoan), not the MIR type.
                        let ref_ty = MirType::Reference {
                            form: triet_mir::ReferenceForm::BorrowReadOnly,
                            inner: Box::new(elem_ty),
                        };
                        (name, MirType::Nullable(Box::new(ref_ty)))
                    } else {
                        let name = if base_ty.is_hashmap() {
                            "__triet_hashmap_get"
                        } else if base_ty.is_vec() {
                            "__triet_vector_get"
                        } else {
                            return Err(LowerError::heap_type_not_supported(
                                &format!("get() on type `{arg0_ty}` — expected Vector or HashMap"),
                                expr_span,
                            ));
                        };
                        (name, MirType::Nullable(Box::new(elem_ty)))
                    };
                    let dest = emit_shim_call(c, shim_name, args, dest_ty, expr_span);
                    return Ok(dest);
                }
                "push" => {
                    if arguments.len() != 2 {
                        return Err(LowerError::unsupported_expr(
                            &arena.expression(*callee).node,
                            expr_span,
                        ));
                    }
                    let args: Vec<Local> = arguments
                        .iter()
                        .map(|a| lower_expr(*a, None, arena, c))
                        .collect::<Result<Vec<_>, _>>()?;
                    // ADR-0077: the push result is `Vector<elem>` where `elem` is
                    // the type of the pushed value (`args[1]`). This is the
                    // AUTHORITATIVE element source: `args[0]` from a bare
                    // `vector_new()` defaults to `Vector<Integer>`, but typecheck
                    // guarantees every push to the same vector agrees, so the
                    // pushed value's type is the true element. The JIT keys
                    // stride/elem_kind off this type.
                    let elem_ty = c.local_decls[args[1].0].ty.clone();
                    let vec_ty = MirType::Vector(Box::new(elem_ty));
                    let dest = emit_shim_call(c, "__triet_vector_push", args, vec_ty, expr_span);
                    return Ok(dest);
                }
                "pop" => {
                    // ADR-0077 P1.5: pop(v) → T? moves the last element out.
                    // Returns Nullable(T) (empty → ~0). The element type comes
                    // from the vector's `Vector(inner)` so the dest carries the
                    // right type → JIT allocates a slot (fat element=String? slot
                    // 24B; scalar=integer var). At JIT: vector_pop_fat=true when
                    // inner is String (stride 24), the shim memcpy's into the
                    // dest out-ptr, and the dest var is bound to slot@0.
                    if arguments.len() != 1 {
                        return Err(LowerError::unsupported_expr(
                            &arena.expression(*callee).node,
                            expr_span,
                        ));
                    }
                    let arg = lower_expr(arguments[0], None, arena, c)?;
                    let vec_ty = &c.local_decls[arg.0].ty;
                    let elem_ty = if let MirType::Vector(inner) = vec_ty {
                        (**inner).clone()
                    } else {
                        MirType::Integer
                    };
                    let dest_ty = MirType::Nullable(Box::new(elem_ty));
                    let dest =
                        emit_shim_call(c, "__triet_vector_pop", vec![arg], dest_ty, expr_span);
                    return Ok(dest);
                }
                "pop_front" => {
                    // ADR-0082: pop_front(v) → T? moves the FIRST element out.
                    // Byte-identical lowering to `pop` — same Nullable(T) dest
                    // (empty → ~0), same slot-alloc / tag-prepend inheritance.
                    // The only difference is the shim name → the JIT emits the
                    // index-0 + shift variant.
                    if arguments.len() != 1 {
                        return Err(LowerError::unsupported_expr(
                            &arena.expression(*callee).node,
                            expr_span,
                        ));
                    }
                    let arg = lower_expr(arguments[0], None, arena, c)?;
                    let vec_ty = &c.local_decls[arg.0].ty;
                    let elem_ty = if let MirType::Vector(inner) = vec_ty {
                        (**inner).clone()
                    } else {
                        MirType::Integer
                    };
                    let dest_ty = MirType::Nullable(Box::new(elem_ty));
                    let dest = emit_shim_call(
                        c,
                        "__triet_vector_pop_front",
                        vec![arg],
                        dest_ty,
                        expr_span,
                    );
                    return Ok(dest);
                }
                "vector_new" => {
                    if !arguments.is_empty() {
                        return Err(LowerError::unsupported_expr(
                            &arena.expression(*callee).node,
                            expr_span,
                        ));
                    }
                    // vector_new() → __triet_vector_alloc(0, 2)
                    // cap=2 to ensure realloc exercises the free-old-ptr path.
                    let len_local = c.alloc_local_ty(MirType::Integer);
                    c.push(Statement::Const {
                        dest: Place::local(len_local),
                        value: ConstValue::Integer(0),
                        span: DUMMY_SPAN,
                    });
                    let cap_local = c.alloc_local_ty(MirType::Integer);
                    c.push(Statement::Const {
                        dest: Place::local(cap_local),
                        value: ConstValue::Integer(2),
                        span: DUMMY_SPAN,
                    });
                    // ADR-0077: an empty `vector_new()` carries its element type
                    // from the `expected` context (`let v: Vector<String> = …`).
                    // With no context (e.g. `push(vector_new(), "hi")`) it defaults
                    // to `Vector<Integer>` — `push` then refines the element from
                    // the pushed value, and the empty buffer (len=0) reallocs on
                    // first push with no stride-dependent copy.
                    let elem = match expected {
                        Some(MirType::Vector(inner)) => (**inner).clone(),
                        _ => MirType::Integer,
                    };
                    let dest = emit_shim_call(
                        c,
                        "__triet_vector_alloc",
                        vec![len_local, cap_local],
                        MirType::Vector(Box::new(elem)),
                        expr_span,
                    );
                    return Ok(dest);
                }
                // ── ADR-0043: HashMap builtins ──
                "hashmap_new" => {
                    if !arguments.is_empty() {
                        return Err(LowerError::unsupported_expr(
                            &arena.expression(*callee).node,
                            expr_span,
                        ));
                    }
                    // ADR-0078 P1b: seed value type from expected context
                    // (e.g. `let m: HashMap<Integer,String> = hashmap_new()`).
                    // Key stays Integer cứng. With no context defaults to
                    // HashMap<Integer,Integer> (byte-compat).
                    let (key_ty, val_ty) = match expected {
                        Some(MirType::HashMap(k, v)) => ((**k).clone(), (**v).clone()),
                        _ => (MirType::Integer, MirType::Integer),
                    };
                    let len_local = c.alloc_local_ty(MirType::Integer);
                    c.push(Statement::Const {
                        dest: Place::local(len_local),
                        value: ConstValue::Integer(0),
                        span: DUMMY_SPAN,
                    });
                    let cap_local = c.alloc_local_ty(MirType::Integer);
                    c.push(Statement::Const {
                        dest: Place::local(cap_local),
                        value: ConstValue::Integer(4),
                        span: DUMMY_SPAN,
                    });
                    let dest = emit_shim_call(
                        c,
                        "__triet_hashmap_alloc",
                        vec![len_local, cap_local],
                        MirType::HashMap(Box::new(key_ty), Box::new(val_ty)),
                        expr_span,
                    );
                    return Ok(dest);
                }
                "insert" => {
                    let args: Vec<Local> = arguments
                        .iter()
                        .map(|a| lower_expr(*a, None, arena, c))
                        .collect::<Result<Vec<_>, _>>()?;
                    let map_ty = c.local_decls[args[0].0].ty.clone();
                    let dest =
                        emit_shim_call(c, "__triet_hashmap_insert", args, &map_ty, expr_span);
                    return Ok(dest);
                }
                "remove" => {
                    // ADR-0078 P1b: remove(map, key) → V? moves the value out
                    // of the map (tombstone slot). Returns Nullable(V) (key not
                    // present → ~0). The value type comes from the map's
                    // `HashMap(_, v)`. The JIT appends the out_ptr (fat=by-ptr,
                    // scalar=0) — same pattern as vector_pop.
                    if arguments.len() != 2 {
                        return Err(LowerError::unsupported_expr(
                            &arena.expression(*callee).node,
                            expr_span,
                        ));
                    }
                    let args: Vec<Local> = arguments
                        .iter()
                        .map(|a| lower_expr(*a, None, arena, c))
                        .collect::<Result<Vec<_>, _>>()?;
                    let map_ty = &c.local_decls[args[0].0].ty;
                    let val_ty = if let MirType::HashMap(_, v) = map_ty {
                        (**v).clone()
                    } else {
                        MirType::Integer
                    };
                    let dest_ty = MirType::Nullable(Box::new(val_ty));
                    let dest =
                        emit_shim_call(c, "__triet_hashmap_remove", args, dest_ty, expr_span);
                    return Ok(dest);
                }
                _ => { /* fall through to user-defined function dispatch */ }
            }

            let callee_ret = c
                .func_return_types
                .get(&callee_name)
                .cloned()
                .unwrap_or(MirType::Integer);
            let is_outcome_ret = matches!(callee_ret, MirType::Outcome { .. });
            // ADR-0058 Lát 1: heap Outcome → sret (treated as fat return).
            let is_heap_outcome_ret = is_outcome_ret && callee_ret.has_heap_payload();
            // P0 fix (2026-07-17, WO-enum-return-sret MÌN 1): this predicate is
            // one of THREE copies (here, Ctx::new callee-side, method-call
            // caller-side) that MUST recognize MirType::Enum together — a
            // caller/callee mismatch on is_fat_ret produces either a JIT panic
            // (caller expects a Cranelift return value the callee never
            // declares) or a silent Scalar miscompile. Do not edit this arm
            // without also updating the other two.
            let is_enum_ret = matches!(callee_ret, MirType::Enum(_));
            // ADR-0062: `String?` call return shares String's fat sret path.
            // ADR-0065 §14 Amend (WO-2 Lát A, chốt #8, 2026-07-20 — O+G
            // confirmed): `is_fat_ret` is one of the THREE copies the
            // `is_enum_ret` comment above warns about — it did not unwrap
            // `Nullable(Struct)`, so a call to a function returning `Struct?`
            // (now fat-return per Ctx::new's `is_struct_return` fix) would
            // dispatch here as a plain scalar call: the callee declares a
            // Cranelift signature with a hidden sret param but the caller
            // never passes one — an ABI arg-count/positional mismatch, not
            // merely "not yet wired". Unwrap the same way `is_struct_return`
            // does (`nullable_payload().unwrap_or`).
            let is_fat_ret = matches!(
                callee_ret.nullable_payload().unwrap_or(&callee_ret),
                MirType::Struct(_)
            ) || is_enum_ret
                || callee_ret.is_string_repr()
                || is_heap_outcome_ret;
            // sret slot layout name: `String?` reprs as the "String" layout
            // (ptr-sentinel, same 24-byte slot); `Struct?` reprs as the
            // INNER struct's layout name (the tag-prepend +8B is a JIT slot-
            // sizing decision, not a separate registered layout) —
            // `callee_ret.to_string()` would be "Point?", which has no
            // registered layout either.
            let sret_layout_name = if callee_ret.is_string_repr() {
                "String".to_string()
            } else if let Some(inner) = callee_ret.nullable_payload() {
                inner.to_string()
            } else {
                callee_ret.to_string()
            };

            let mut args: Vec<Local> = arguments
                .iter()
                .map(|a| lower_expr(*a, None, arena, c))
                .collect::<Result<Vec<_>, _>>()?;

            // B7-lift (ADR-0042): heap args now allowed.
            // Move semantics: caller zeroes slot after call, borrowck
            // enforces E2420 use-after-move.
            if is_fat_ret {
                // sret: allocate struct/string/heap-outcome/enum local for
                // return, pass as hidden arg[0].
                let ret_local = c.alloc_local_ty(callee_ret.clone());
                c.push(Statement::StorageLive(ret_local, expr_span.clone()));
                if is_heap_outcome_ret {
                    // ADR-0058 Lát 1: heap Outcome sret — OutcomeAlloc
                    // (JIT no-op; required for verifier/borrowck).
                    c.push(Statement::OutcomeAlloc {
                        dest: ret_local,
                        span: expr_span.clone(),
                    });
                } else if is_enum_ret {
                    // P0 fix: enum sret — EnumAlloc gives the caller's return
                    // buffer a real StackSlot (mirrors Expr::EnumLiteral); the
                    // callee block-copies its own slot's bytes into it, so no
                    // SetDiscriminant here (unlike EnumLiteral, this local's
                    // discriminant is written by the callee, not by us).
                    c.push(Statement::EnumAlloc {
                        dest: ret_local,
                        enum_name: sret_layout_name.clone(),
                        span: expr_span.clone(),
                    });
                } else {
                    c.push(Statement::StructAlloc {
                        dest: ret_local,
                        struct_name: sret_layout_name.clone(),
                        span: expr_span.clone(),
                    });
                }
                // Insert sret pointer as first argument.
                args.insert(0, ret_local);
                // ADR-0042 Q1 + ADR-0045 §2: collect args for zeroing.
                // Skip arg[0] — sret pointer is not user-visible.
                // Skip reference types (&0 String etc.) — callee borrows.
                let to_zero: Vec<Local> = args[1..]
                    .iter()
                    .filter(|&&arg| {
                        let ty = &c.local_decls[arg.0].ty;
                        if ty.is_reference() {
                            return false;
                        }
                        !ctx_is_copy(ty, c)
                    })
                    .copied()
                    .collect();
                let ret_bb = c.alloc_bb();
                let call_bb = c.cur;
                let return_shape = if is_enum_ret {
                    triet_mir::ReturnShape::Enum {
                        enum_name: sret_layout_name,
                    }
                } else {
                    triet_mir::ReturnShape::Struct {
                        struct_name: sret_layout_name,
                    }
                };
                c.term(
                    call_bb,
                    Terminator::CallDispatch {
                        callee: triet_mir::FunctionId(0),
                        callee_name,
                        target: CallTarget::Jit,
                        args,
                        return_bb: ret_bb,
                        dest: Vec::new(),
                        return_shape,
                        span: expr_span,
                    },
                );
                c.cur = ret_bb;
                // ADR-0042 Q1: Deinit tomb-stones Move-type args.
                for &arg in &to_zero {
                    c.push(Statement::Deinit(arg, DUMMY_SPAN));
                }
                Ok(ret_local)
            } else if is_outcome_ret {
                // ADR-0052 OP.4a: Outcome call — allocate slot, receive 2 return values.
                let dest = c.alloc_local_ty(callee_ret.clone());
                c.push(Statement::StorageLive(dest, expr_span.clone()));
                c.push(Statement::OutcomeAlloc {
                    dest,
                    span: expr_span.clone(),
                });
                let to_zero: Vec<Local> = args
                    .iter()
                    .filter(|&&arg| {
                        let ty = &c.local_decls[arg.0].ty;
                        if ty.is_reference() {
                            return false;
                        }
                        !ctx_is_copy(ty, c)
                    })
                    .copied()
                    .collect();
                let ret_bb = c.alloc_bb();
                let call_bb = c.cur;
                c.term(
                    call_bb,
                    Terminator::CallDispatch {
                        callee: triet_mir::FunctionId(0),
                        callee_name,
                        target: CallTarget::Jit,
                        args,
                        return_bb: ret_bb,
                        dest: vec![dest],
                        return_shape: match &callee_ret {
                            MirType::Outcome {
                                allow_null_state: true,
                                ..
                            } => triet_mir::ReturnShape::TernaryOutcome,
                            _ => triet_mir::ReturnShape::BinaryOutcome,
                        },
                        span: expr_span,
                    },
                );
                c.cur = ret_bb;
                for &arg in &to_zero {
                    c.push(Statement::Deinit(arg, DUMMY_SPAN));
                }
                Ok(dest)
            } else {
                let dest = c.alloc_local_ty(callee_ret.clone());
                c.push(Statement::StorageLive(dest, expr_span.clone()));
                // ADR-0042 Q1 + ADR-0045 §2: collect args for zeroing.
                // Skip reference types (&0 String etc.) — callee borrows.
                let to_zero: Vec<Local> = args
                    .iter()
                    .filter(|&&arg| {
                        let ty = &c.local_decls[arg.0].ty;
                        if ty.is_reference() {
                            return false;
                        }
                        !ctx_is_copy(ty, c)
                    })
                    .copied()
                    .collect();
                let ret_bb = c.alloc_bb();
                let call_bb = c.cur;
                c.term(
                    call_bb,
                    Terminator::CallDispatch {
                        callee: triet_mir::FunctionId(0),
                        callee_name,
                        target: CallTarget::Jit,
                        args,
                        return_bb: ret_bb,
                        dest: vec![dest],
                        return_shape: triet_mir::ReturnShape::Scalar,
                        span: expr_span,
                    },
                );
                c.cur = ret_bb;
                // ADR-0042 Q1: Deinit tomb-stones Move-type args.
                for &arg in &to_zero {
                    c.push(Statement::Deinit(arg, DUMMY_SPAN));
                }
                Ok(dest)
            }
        }
        Expr::Block { .. } => {
            // Expression blocks introduce a new scope for `let` bindings.
            c.push_scope();
            let final_expr = match &arena.expression(expr_id).node {
                Expr::Block {
                    statements,
                    final_expr,
                } => {
                    for &stmt_id in statements {
                        let spanned_stmt = arena.statement(stmt_id);
                        let stmt = spanned_stmt.node.clone();
                        let stmt_span = spanned_stmt.span.clone();
                        lower_stmt(&stmt, stmt_span, arena, c)?;
                    }
                    *final_expr
                }
                _ => None,
            };
            let result = match final_expr {
                Some(e) => {
                    // Bug A (CFG-tail lát 1): the tail expression is the block's
                    // VALUE — it escapes the block scope. Returning its local
                    // directly let pop_scope() drop it as a block-local while a
                    // caller (e.g. `let v = { … }; len(v)`) still used it →
                    // E2421 use-after-storage-end. Mirror the Expr::If idiom
                    // (2472): move the tail value into a fresh result local that
                    // outlives this scope; the move tombstones the tail local
                    // (M1), so the pop_scope drop below is a no-op.
                    //
                    // Exception — reference tails (`{ … id(&0 x) }`): a reference
                    // is Copy and is NOT dropped by pop_scope (only the owned
                    // source it borrows is). The original direct-return is
                    // already correct, and inserting an Assign would model a
                    // reborrow that conflicts with the live loan, masking the
                    // intended drop-while-borrowed diagnostic (E2450 → E2440,
                    // fixture 102). Reference tails keep the direct path.
                    // ADR-0072 §2.2 TRANSPARENT: the block tail IS the block's
                    // value — forward the block's expected type to it.
                    let tail_val = lower_expr(e, expected, arena, c)?;
                    if c.local_decls[tail_val.0].ty.is_reference() {
                        Ok(tail_val)
                    } else {
                        let result = c.alloc_local_ty(c.local_decls[tail_val.0].ty.clone());
                        c.push(Statement::StorageLive(result, expr_span.clone()));
                        c.push(Statement::Assign {
                            dest: Place::local(result),
                            source: Place::local(tail_val),
                            span: expr_span.clone(),
                        });
                        Ok(result)
                    }
                }
                None => {
                    let u = c.alloc_local();
                    c.push(Statement::StorageLive(u, expr_span.clone()));
                    c.push(Statement::Const {
                        dest: Place::local(u),
                        value: ConstValue::Unit,
                        span: expr_span,
                    });
                    Ok(u)
                }
            };
            c.pop_scope();
            result
        }
        Expr::If {
            condition,
            then_branch,
            else_branch,
            ..
        } => {
            let then_branch = *then_branch;
            let else_branch = *else_branch;
            let cond = lower_expr(*condition, None, arena, c)?;
            let then_bb = c.alloc_bb();
            let merge_bb = c.alloc_bb();
            let else_bb = if else_branch.is_some() {
                c.alloc_bb()
            } else {
                merge_bb
            };

            let cur = c.cur;
            c.term(
                cur,
                Terminator::If {
                    cond,
                    positive_bb: then_bb,
                    zero_bb: None,
                    negative_bb: else_bb,
                    span: expr_span.clone(),
                },
            );

            c.cur = then_bb;
            // ADR-0072 §2.2 TRANSPARENT: both arms carry the if's expected type
            // (the condition stays OPAQUE → None).
            let then_val = lower_expr(then_branch, expected, arena, c)?;
            let then_end = c.cur;
            // ADR-0056: type the merge result from the branch value so a
            // Fat-Pointer ({ptr,len,cap}) survives the merge as a typed move
            // (JIT typed-Assign copies all 3 words). Untyped → scalar i64 →
            // only `ptr` moved → len/cap garbage. Type comes FROM the branch
            // (then_val), so Vector + scalar flow through the same path.
            let result = c.alloc_local_ty(c.local_decls[then_val.0].ty.clone());

            if let Some(eb) = else_branch {
                c.push(Statement::StorageLive(result, expr_span.clone()));
                c.push(Statement::Assign {
                    dest: Place::local(result),
                    source: Place::local(then_val),
                    span: expr_span.clone(),
                });
                c.term(
                    then_end,
                    Terminator::Goto {
                        target: merge_bb,
                        span: DUMMY_SPAN,
                    },
                );

                c.cur = else_bb;
                let else_val = lower_expr(eb, expected, arena, c)?;
                let else_end = c.cur;
                c.push(Statement::Assign {
                    dest: Place::local(result),
                    source: Place::local(else_val),
                    span: expr_span.clone(),
                });
                c.term(
                    else_end,
                    Terminator::Goto {
                        target: merge_bb,
                        span: DUMMY_SPAN,
                    },
                );

                c.cur = merge_bb;
                Ok(result)
            } else {
                c.term(
                    then_end,
                    Terminator::Goto {
                        target: merge_bb,
                        span: DUMMY_SPAN,
                    },
                );
                c.cur = merge_bb;
                let u = c.alloc_local();
                c.push(Statement::StorageLive(u, expr_span.clone()));
                c.push(Statement::Const {
                    dest: Place::local(u),
                    value: ConstValue::Unit,
                    span: expr_span,
                });
                Ok(u)
            }
        }
        Expr::ElvisOp { object, default } => {
            // `e ?: default` — if e is null (NULL_SENTINEL), evaluate
            // default; otherwise use e. RHS is an Expression with possible
            // side effects → branch-based, NOT select (ADR-0039).
            let obj_val = lower_expr(*object, None, arena, c)?;

            // Create a const local holding NULL_SENTINEL for comparison.
            let sentinel = c.alloc_local();
            c.push(Statement::StorageLive(sentinel, expr_span.clone()));
            c.push(Statement::Const {
                dest: Place::local(sentinel),
                value: ConstValue::Integer(i128::from(triet_mir::NULL_SENTINEL)),
                span: expr_span.clone(),
            });

            // Compare object == NULL_SENTINEL → Trilean! (+1 if null, -1 if present).
            let cmp = c.alloc_local();
            c.push(Statement::StorageLive(cmp, expr_span.clone()));
            c.push(Statement::BinaryOp {
                dest: Place::local(cmp),
                op: triet_mir::BinOp::Eq,
                left: Place::local(obj_val),
                right: Place::local(sentinel),
                span: expr_span.clone(),
            });

            // Branch: positive → null (use default), negative → present (use object).
            let null_bb = c.alloc_bb();
            let present_bb = c.alloc_bb();
            let merge_bb = c.alloc_bb();
            let cur = c.cur;
            c.term(
                cur,
                Terminator::If {
                    cond: cmp,
                    positive_bb: null_bb,
                    zero_bb: None,
                    negative_bb: present_bb,
                    span: expr_span.clone(),
                },
            );

            // ── Result type = T (payload), NOT T? ──
            // Must derive from the object's type to prevent "?"-typed locals
            // from carrying heap values downstream (ADR-0040 / bài học 4.3a
            // call-dest-"?" bug). Sentinel/cmp can stay "?" — scratch scalars.
            let obj_ty = c.local_decls[obj_val.0].ty.clone();
            let payload_ty = obj_ty
                .nullable_payload()
                .ok_or_else(|| {
                    LowerError::unsupported_expr(&arena.expression(expr_id).node, expr_span.clone())
                })?
                .clone();

            // ── Null path: evaluate default expression ──
            c.cur = null_bb;
            let default_val = lower_expr(*default, None, arena, c)?;
            let null_end = c.cur;
            let result = c.alloc_local_ty(payload_ty.clone());
            c.push(Statement::StorageLive(result, expr_span.clone()));
            c.push(Statement::Assign {
                dest: Place::local(result),
                source: Place::local(default_val),
                span: expr_span.clone(),
            });
            c.term(
                null_end,
                Terminator::Goto {
                    target: merge_bb,
                    span: DUMMY_SPAN,
                },
            );

            // ── Present path: use the original object value ──
            c.cur = present_bb;
            c.push(Statement::StorageLive(result, expr_span.clone()));
            c.push(Statement::Assign {
                dest: Place::local(result),
                source: Place::local(obj_val),
                span: expr_span.clone(),
            });
            let present_end = c.cur;
            c.term(
                present_end,
                Terminator::Goto {
                    target: merge_bb,
                    span: DUMMY_SPAN,
                },
            );

            c.cur = merge_bb;
            Ok(result)
        }
        Expr::NullableMap {
            inner,
            bind_var,
            body,
        } => {
            // ADR-0039 §1 (Phase 14.4): inline map/flatMap — NO closure
            // object. inner real → bind its value to `bind_var`, evaluate
            // body; the body's i64 IS the result (map auto-wrap and flatMap
            // flatten are both identity at the Bậc A value level — a
            // nullable body already carries NULL_SENTINEL-or-value). inner
            // null → pass NULL_SENTINEL straight through (body not run).
            let inner_val = lower_expr(*inner, None, arena, c)?;
            let inner_ty = c.local_decls[inner_val.0].ty.clone();
            let payload_ty = inner_ty
                .nullable_payload()
                .ok_or_else(|| {
                    LowerError::unsupported_expr(&arena.expression(expr_id).node, expr_span.clone())
                })?
                .clone();

            let sentinel = c.alloc_local();
            c.push(Statement::StorageLive(sentinel, expr_span.clone()));
            c.push(Statement::Const {
                dest: Place::local(sentinel),
                value: ConstValue::Integer(i128::from(triet_mir::NULL_SENTINEL)),
                span: expr_span.clone(),
            });
            let cmp = c.alloc_local();
            c.push(Statement::StorageLive(cmp, expr_span.clone()));
            c.push(Statement::BinaryOp {
                dest: Place::local(cmp),
                op: triet_mir::BinOp::Eq,
                left: Place::local(inner_val),
                right: Place::local(sentinel),
                span: expr_span.clone(),
            });

            let null_bb = c.alloc_bb();
            let present_bb = c.alloc_bb();
            let merge_bb = c.alloc_bb();
            // Result is T? (either NULL_SENTINEL or the mapped value).
            let result = c.alloc_local_ty(inner_ty.clone());
            let cur = c.cur;
            c.term(
                cur,
                Terminator::If {
                    cond: cmp,
                    positive_bb: null_bb,
                    zero_bb: None,
                    negative_bb: present_bb,
                    span: expr_span.clone(),
                },
            );

            // ── Null path: pass NULL_SENTINEL straight through ──
            c.cur = null_bb;
            c.push(Statement::StorageLive(result, expr_span.clone()));
            c.push(Statement::Assign {
                dest: Place::local(result),
                source: Place::local(sentinel),
                span: expr_span.clone(),
            });
            let null_end = c.cur;
            c.term(
                null_end,
                Terminator::Goto {
                    target: merge_bb,
                    span: DUMMY_SPAN,
                },
            );

            // ── Present path: bind value, evaluate body ──
            c.cur = present_bb;
            c.push(Statement::StorageLive(result, expr_span.clone()));
            c.push_scope();
            if !bind_var.is_empty() {
                let bind_local = c.alloc_local_ty(payload_ty.clone());
                c.push(Statement::StorageLive(bind_local, expr_span.clone()));
                c.push(Statement::Assign {
                    dest: Place::local(bind_local),
                    source: Place::local(inner_val),
                    span: expr_span.clone(),
                });
                c.vars.insert(bind_var.clone(), bind_local);
            }
            let body_val = lower_expr(*body, None, arena, c)?;
            // Bug B (ADR-0062 lát 5): the result was allocated with inner_ty
            // (the INPUT T?) at 2690, but `?+>` produces U? where U is the
            // body's type. When T ≠ U (e.g. String? ?+> |s| len(s) → Integer?)
            // the result was mistyped (heap-in scalar-out and vice versa),
            // making Drop(result) call the wrong free shim on a garbage value
            // — latent UB that only ran by luck (slot@0 == 0). Retype the
            // result from the body: flatMap (body already U?) keeps it; map
            // (body U) wraps to U?.
            let body_ty = c.local_decls[body_val.0].ty.clone();
            let result_ty = if body_ty.is_nullable() {
                body_ty
            } else {
                MirType::Nullable(Box::new(body_ty))
            };
            c.local_decls[result.0].ty = result_ty;
            c.push(Statement::Assign {
                dest: Place::local(result),
                source: Place::local(body_val),
                span: expr_span.clone(),
            });
            c.pop_scope();
            let present_end = c.cur;
            c.term(
                present_end,
                Terminator::Goto {
                    target: merge_bb,
                    span: DUMMY_SPAN,
                },
            );

            c.cur = merge_bb;
            Ok(result)
        }
        Expr::Borrow { form, operand } => {
            let mir_form = lower_ref_form(*form);
            // The operand is an lvalue (IDENT or field-access chain per
            // ADR-0031 §2) — lower it to a projected Place so the borrow
            // checker can track the loan at field granularity.
            let source = lower_place(*operand, arena, c)?;
            // S4: construct MirType::Reference directly — no string round-trip.
            let source_ty = c.local_decls[source.local.0].ty.clone();
            let ref_ty = MirType::Reference {
                form: mir_form,
                inner: Box::new(source_ty),
            };
            let dest = c.alloc_local_ty(ref_ty);
            c.push(Statement::StorageLive(dest, expr_span.clone()));
            c.push(Statement::Borrow {
                dest: Place::local(dest),
                form: mir_form,
                source,
                span: expr_span,
            });
            Ok(dest)
        }
        // `obj.field` as an rvalue: read through the projected place into a temp.
        // ADR-0071 Lát 2: `Type.Variant` is NO LONGER an enum literal — the
        // dot-form variant is gone (qualified `Type::Variant` parses as
        // `Expr::EnumLiteral`). A `FieldAccess` here is a genuine field read.
        Expr::FieldAccess { .. } => {
            let source = lower_place(expr_id, arena, c)?;
            let field_ty = place_result_type(&source, c);
            // ADR-0084 Slice 1b: an aggregate/heap field reached THROUGH a
            // reference (`source` carries a leading `Deref` projection —
            // `lower_place` inserts it when the base resolves to a
            // `MirType::Reference`) is a zero-copy SUB-BORROW, not a value
            // read or move-out — emit `Statement::Borrow` (address-only,
            // never touches the aggregate/heap bytes) instead of the
            // `Assign` value-copy below. Only fires for the field kinds
            // `check_field_access` already classified this way (plain
            // `Struct` or heap-leaf String/Vector/HashMap); the scalar
            // terminal case (Slice 1a) and the owned-base move-out case
            // (no `Deref`, pre-existing WO-0074/0075 semantics) both fall
            // through to the unchanged `Assign` path below.
            let source_has_deref = source
                .projection
                .iter()
                .any(|p| matches!(p, Projection::Deref));
            if source_has_deref
                && (matches!(&field_ty, MirType::Struct(_)) || field_ty.is_any_heap())
            {
                let ref_ty = MirType::Reference {
                    form: triet_mir::ReferenceForm::BorrowReadOnly,
                    inner: Box::new(field_ty),
                };
                let d = c.alloc_local_ty(ref_ty);
                c.push(Statement::StorageLive(d, expr_span.clone()));
                c.push(Statement::Borrow {
                    dest: Place::local(d),
                    form: triet_mir::ReferenceForm::BorrowReadOnly,
                    source,
                    span: expr_span,
                });
                return Ok(d);
            }
            // ADR-0065 §12: a field whose type is a nullable aggregate
            // (`Struct?`/`Enum?`) must carry that type so `match`/Elvis route to
            // the nullable path and the JIT copies the full tagged slot.
            // ADR-0070 read-side: a HEAP field (String/Vector/HashMap) likewise
            // carries its type so the move-out dest gets a real heap slot and a
            // Drop that frees it (an Unknown-typed temp leaks — Drop sees no
            // heap type). SCALAR field reads keep the legacy Unknown-typed temp
            // (scalar leaf loaded as i64), preserving existing behavior.
            let d = if matches!(&field_ty, MirType::Nullable(inner)
                if matches!(inner.as_ref(), MirType::Struct(_) | MirType::Enum(_)))
                || field_ty.is_any_heap()
                // Phase 2 (ADR-0070 read-side): a plain heap-owning STRUCT field
                // (e.g. `let m = h.inner`) must carry its `Struct` type so the JIT
                // pre-pass allocates a real stack slot for the move-out dest. An
                // Unknown-typed temp (the scalar-leaf path) gets NO slot, so the
                // aggregate field copy writes through a garbage address → SIGSEGV.
                || matches!(&field_ty, MirType::Struct(_))
                // WO-0074 (Phase 3 — Nợ A): a heap-carrying ENUM field
                // (e.g. `let e = h.msg`) likewise carries its `MirType::Enum(_)`
                // type so the JIT pre-pass allocates an enum stack slot for the
                // move-out dest. Without it the dest is Unknown-typed → NO slot →
                // the aggregate enum copy writes through a garbage address →
                // SIGSEGV (identical failure to the Struct case above).
                || matches!(&field_ty, MirType::Enum(_))
                // WO-NullableFieldMoveOut (ADR-0076 §AMEND): a heap-`T?` field
                // (e.g. `let s = b.s` with `b.s: String?`) carries its
                // `Nullable(heap)` type so the JIT pre-pass allocates a real slot
                // for the move-out dest (`String?` → 24B fat via `is_string_repr()`
                // @mir:1186; `Vector?`/`HashMap?` → i64-var). The `Nullable(Struct/
                // Enum)` clause above does NOT cover this (inner is heap-scalar,
                // not aggregate) and `is_any_heap()` does NOT unwrap Nullable —
                // without this clause the dest is Unknown-typed → NO slot →
                // SIGSEGV. (WO-0075's "Lower Site-H no-op" note does NOT apply to
                // Nullable-heap: this propagation is load-bearing.)
                || matches!(&field_ty, MirType::Nullable(inner) if inner.is_any_heap())
            {
                c.alloc_local_ty(field_ty)
            } else {
                c.alloc_local()
            };
            c.push(Statement::StorageLive(d, expr_span.clone()));
            c.push(Statement::Assign {
                dest: Place::local(d),
                source,
                span: expr_span,
            });
            Ok(d)
        }
        Expr::StructLiteral {
            struct_name,
            fields,
        } => {
            let d = c.alloc_local_ty(MirType::Struct(struct_name.to_string()));
            c.push(Statement::StorageLive(d, expr_span.clone()));
            c.push(Statement::StructAlloc {
                dest: d,
                struct_name: struct_name.clone(),
                span: expr_span.clone(),
            });
            for (field_name, field_expr) in fields {
                // The field's declared type, for nullable-aggregate construction
                // (`~0` null materialize + `~+` widen) — `~0`/`~+` carry no
                // annotation, so the field layout supplies the expected type.
                let field_decl_ty = c
                    .struct_layouts
                    .get(struct_name.as_str())
                    .and_then(|l| l.fields.iter().find(|f| &f.name == field_name))
                    .map(|f| f.ty.clone());
                let field_val = if is_null_expr(&arena.expression(*field_expr).node) {
                    // ADR-0065 §12: `~0` in a struct field materializes a
                    // NULL_SENTINEL of the field's declared type (mirror of the
                    // `let x: T? = ~0` path). Works for scalar `T?` and nested
                    // nullable aggregate `Struct?`/`Enum?` (tag@0 = MIN).
                    let decl_ty = field_decl_ty.clone().ok_or_else(|| {
                        LowerError::null_literal_without_expected_type(expr_span.clone())
                    })?;
                    let nd = c.alloc_local_ty(decl_ty);
                    c.push(Statement::StorageLive(nd, expr_span.clone()));
                    c.push(Statement::Const {
                        dest: Place::local(nd),
                        value: ConstValue::Integer(i128::from(triet_mir::NULL_SENTINEL)),
                        span: expr_span.clone(),
                    });
                    nd
                } else {
                    // ADR-0072 §2.3/§2.4: the field's declared type is the expected
                    // type. `~+ inner` / `~0` now consume it in the leaf-consumer
                    // (no bolt-on redirect); the field Assign / widening below
                    // builds the nullable repr from the lowered payload local.
                    lower_expr(*field_expr, field_decl_ty.as_ref(), arena, c)?
                };
                // ADR-0066 M-2: B8 relaxed for FLAT direct heap leaf. A direct
                // heap-leaf field (declared exactly `String`/`Vector`/`HashMap`)
                // is now ALLOWED — KCN-1 inline drop-glue frees it on scope exit.
                // Still REFUSED: a Struct/Enum field (transitively) containing
                // heap (→ Lát 2 recursive) and a `Nullable(heap)` field (heap-
                // nullable, ADR-0062 — NOT Lát 1). Decide on the DECLARED field
                // type, NOT the lowered value type: the `~+` redirect above
                // strips `String?` → plain `String` in the value, but the field
                // is still heap-nullable. `ctx_is_copy` consults the layout maps
                // (NOT `is_copy(None)`, which assumes Struct/Enum Copy → leaks
                // transitive heap). Scalars / Nullable(scalar) / Copy-aggregates
                // pass.
                let field_ty = &c.local_decls[field_val.0].ty;
                let decl_ty = field_decl_ty.as_ref().unwrap_or(field_ty);
                let is_direct_heap_leaf = matches!(
                    decl_ty,
                    MirType::String | MirType::Vector(_) | MirType::HashMap(..)
                );
                // ADR-0067 2a: a plain (non-nullable) nested STRUCT field whose
                // layout resolves is now ALLOWED even when it transitively
                // contains heap — the drop-glue / Deinit recurse it statically
                // (`collect_heap_leaves`). Self-ref is blocked upstream by
                // typecheck (`resolve_type` → UnknownType); the depth-64 limit in
                // `collect_heap_leaves` is the last-resort net. ENUM is NOT
                // widened here: enum-payload heap is tag-dependent (runtime
                // disc) → ADR-0067 2b; a Copy enum still passes via `ctx_is_copy`.
                // `Nullable(heap)` stays REFUSED (heap-nullable, ADR-0062), and
                // `&+`/box stay REFUSED — only a bare `Struct` is widened.
                let is_nested_struct = matches!(
                    decl_ty, MirType::Struct(n) if c.struct_layouts.contains_key(n.as_str()));
                // ADR-0067 2b+: a plain (non-nullable) ENUM field whose layout
                // resolves is now ALLOWED even when a variant carries heap — the
                // struct drop-glue / Deinit walk emits a runtime tag-switch for it
                // (`collect_heap_leaves` → `LeafKind::Enum`). Self-ref blocked
                // upstream by typecheck; the depth-64 net guards the rest.
                // `Nullable(Enum)` (heap-nullable, ADR-0062) stays REFUSED.
                let is_nested_enum = matches!(
                    decl_ty, MirType::Enum(n) if c.enum_layouts.contains_key(n.as_str()));
                // ADR-0070: a capability field is a ZST (size 0) with NO heap and
                // NO runtime payload — constructing a struct that holds one is
                // sound (the field store/read/drop are all no-ops at runtime).
                // It is non-copy ONLY so the borrow checker move-tracks it
                // (ctx_is_copy returns false), so it must be allowed explicitly.
                let is_capability = matches!(decl_ty, MirType::Capability(_));
                // ADR-0076: a heap-`T?` leaf field (`String?`/`Vector?`/
                // `HashMap?`) is now constructible. `~+ <heap>` lowers the inner
                // to a plain heap value (the widen `String → String?` is a repr
                // no-op — fat-ptr stored @field-offset); `~0`/null materializes
                // NULL_SENTINEL @field-offset (the is_null_expr branch above).
                // Drop-glue frees it sentinel-safe (`collect_heap_leaves` →
                // `LeafKind::Heap`). A `Nullable` of a heap-CONTAINING aggregate
                // stays refused (Move, no per-field drop-flag — Nợ defer).
                let is_heap_nullable_leaf =
                    matches!(decl_ty, MirType::Nullable(inner) if inner.is_any_heap());
                if !is_direct_heap_leaf
                    && !is_nested_struct
                    && !is_nested_enum
                    && !is_capability
                    && !is_heap_nullable_leaf
                    && !ctx_is_copy(decl_ty, c)
                {
                    return Err(LowerError::heap_type_not_supported(
                        &format!("struct `{struct_name}` field `{field_name}` type `{decl_ty}`"),
                        expr_span,
                    ));
                }
                c.push(Statement::Assign {
                    dest: Place::local(d).project(Projection::Field(field_name.clone())),
                    source: Place::local(field_val),
                    span: expr_span.clone(),
                });
                // AMEND ADR-0067: a heap-owning aggregate local moved into a field is dead —
                // tombstone it (atomic, same BB) so its scope-end Drop is a no-op. Without this,
                // the JIT aggregate byte-copy leaves a live duplicate ptr → double-free. Scalar
                // heap-leaf fields stay handled by the JIT M1-zeroing path (not here).
                if is_nested_struct || is_nested_enum {
                    c.push(Statement::Deinit(field_val, expr_span.clone()));
                }
            }
            Ok(d)
        }
        Expr::EnumLiteral {
            name,
            variant_name,
            payload,
        } => {
            // ADR-0071 Lát 2: an enum value is `MirType::Enum(name)` (NOT
            // `Struct`) — the JIT's discriminant/payload access keys off the
            // enum layout. (Mirrors the now-removed Call-resolution path.)
            let d = c.alloc_local_ty(MirType::Enum(name.to_string()));
            c.push(Statement::StorageLive(d, expr_span.clone()));
            c.push(Statement::EnumAlloc {
                dest: d,
                enum_name: name.clone(),
                span: expr_span.clone(),
            });
            // Look up the discriminant value for this variant.
            // Clone the value out of the HashMap before mutating ctx.
            let disc = {
                let layout = c.enum_layouts.get(name).ok_or_else(|| LowerError {
                    message: format!("unknown enum '{name}'"),
                    span: expr_span.clone(),
                })?;
                layout
                    .variants
                    .iter()
                    .find(|v| v.name == *variant_name)
                    .map(|v| v.discriminant_value)
                    .ok_or_else(|| LowerError {
                        message: format!("unknown variant '{variant_name}' in enum '{name}'"),
                        span: expr_span.clone(),
                    })?
            };
            c.push(Statement::SetDiscriminant {
                dest: d,
                value: disc,
                span: expr_span.clone(),
            });
            if let Some(payload_expr) = payload {
                let val = lower_expr(*payload_expr, None, arena, c)?;
                // ADR-0067 2b-1: a direct heap-leaf payload (String/Vector/
                // HashMap) is allowed (tag-switch drop-glue frees the active
                // variant); a struct-transitive-heap or Nullable(heap) payload
                // stays refused (B8). Mirrors the Call-resolution gate.
                let payload_ty = &c.local_decls[val.0].ty;
                let is_direct_heap_leaf = matches!(
                    payload_ty,
                    MirType::String | MirType::Vector(_) | MirType::HashMap(..)
                );
                if !is_direct_heap_leaf && !ctx_is_copy(payload_ty, c) {
                    return Err(LowerError::heap_type_not_supported(
                        &format!("enum `{name}::{variant_name}` payload type `{payload_ty}`"),
                        expr_span,
                    ));
                }
                c.push(Statement::Assign {
                    dest: Place::local(d).project(Projection::Payload(variant_name.clone())),
                    source: Place::local(val),
                    span: expr_span.clone(),
                });
            }
            Ok(d)
        }
        Expr::Match { scrutinee, arms } => {
            // Lower scrutinee into a temp.
            let scrut_local = lower_expr(*scrutinee, None, arena, c)?;

            let scrut_ty = c.local_decls[scrut_local.0].ty.clone();

            // ── ADR-0061 T6: match on a Trit scrutinee (value-keyed) ──
            // The scrutinee is already an i64 Trit (-1/0/1); SwitchInt
            // directly on its value — NO GetDiscriminant (that is the enum
            // path). Trit-literal arms become cases; a missing value with
            // no wildcard hits the default Trap (GAP-2: no silent
            // fall-through, JIT dies at the uncovered value). This branch
            // must run BEFORE the enum GetDiscriminant fallthrough below,
            // which would otherwise read a discriminant off a bare Trit.
            if scrut_ty == MirType::Trit {
                use triet_syntax::{LiteralPattern, NumericSuffix, Pattern};
                let cur_bb = c.cur;
                let merge_bb = c.alloc_bb();
                let result = c.alloc_local();
                c.push(Statement::StorageLive(result, expr_span.clone()));

                let mut cases: Vec<(i64, BasicBlock)> = Vec::new();
                let mut wildcard_arm: Option<&triet_syntax::MatchArm> = None;

                for arm in arms.iter() {
                    let pat = arena.pattern(arm.pattern);
                    let pat_span = pat.span.clone();
                    // Wildcard must be the last arm (arms after = unreachable).
                    if wildcard_arm.is_some() {
                        return Err(LowerError {
                            message: "wildcard `_` must be the last arm in a Trit match — \
                                      arms after wildcard are unreachable"
                                .to_string(),
                            span: pat_span,
                        });
                    }
                    match &pat.node {
                        Pattern::Wildcard => wildcard_arm = Some(arm),
                        // ADR-0064 §8: a variable binding (`other =>`) is a
                        // catch-all — bound to the scrutinee in the default block.
                        Pattern::Variable(_) => wildcard_arm = Some(arm),
                        Pattern::Literal(LiteralPattern::Integer {
                            value,
                            suffix: Some(NumericSuffix::Trit),
                        }) => {
                            let key = i64::try_from(*value).map_err(|_| LowerError {
                                message: format!("Trit literal value {value} out of range"),
                                span: pat_span.clone(),
                            })?;
                            let arm_bb = c.alloc_bb();
                            cases.push((key, arm_bb));
                            c.cur = arm_bb;
                            c.push_scope();
                            let body_val = lower_expr(arm.body, expected, arena, c)?;
                            let arm_end = c.cur;
                            // ADR-0056: type merge result from the arm value.
                            c.local_decls[result.0].ty = c.local_decls[body_val.0].ty.clone();
                            c.push(Statement::Assign {
                                dest: Place::local(result),
                                source: Place::local(body_val),
                                span: expr_span.clone(),
                            });
                            c.term(
                                arm_end,
                                Terminator::Goto {
                                    target: merge_bb,
                                    span: DUMMY_SPAN,
                                },
                            );
                            c.pop_scope();
                        }
                        other => {
                            return Err(LowerError {
                                message: format!(
                                    "unsupported pattern in Trit match: {other:?} — expected \
                                     `-1_trit`/`0_trit`/`1_trit` or `_`"
                                ),
                                span: pat_span,
                            });
                        }
                    }
                }

                // GAP-2: default → wildcard body if present, else Trap. A
                // Trit value not covered by any arm (and no `_`) traps —
                // never silently falls through.
                let default_bb = if let Some(wc) = wildcard_arm {
                    let wc_bb = c.alloc_bb();
                    c.cur = wc_bb;
                    c.push_scope();
                    bind_scalar_catch_all(c, arena, wc, scrut_local, &scrut_ty, &expr_span);
                    let body_val = lower_expr(wc.body, expected, arena, c)?;
                    let wc_end = c.cur;
                    c.local_decls[result.0].ty = c.local_decls[body_val.0].ty.clone();
                    c.push(Statement::Assign {
                        dest: Place::local(result),
                        source: Place::local(body_val),
                        span: expr_span.clone(),
                    });
                    c.term(
                        wc_end,
                        Terminator::Goto {
                            target: merge_bb,
                            span: DUMMY_SPAN,
                        },
                    );
                    c.pop_scope();
                    wc_bb
                } else {
                    let trap_bb = c.alloc_bb();
                    c.cur = trap_bb;
                    c.term(
                        trap_bb,
                        Terminator::Trap {
                            span: expr_span.clone(),
                        },
                    );
                    trap_bb
                };

                c.term(
                    cur_bb,
                    Terminator::SwitchInt {
                        discriminant: scrut_local,
                        cases,
                        default_bb,
                        span: expr_span.clone(),
                    },
                );
                c.cur = merge_bb;
                return Ok(result);
            }

            // ── ADR-0064: match on a Trilean scrutinee (value-keyed) ──
            // Trilean literals true/false/unknown encode to i64 1/-1/0
            // (lower:1464), the same encoding as the i64 scrutinee. SwitchInt
            // directly on the value (NO GetDiscriminant); a missing value with
            // no wildcard hits the default Trap (GAP-2). Mirror of the Trit
            // branch above; must run BEFORE the enum GetDiscriminant fallthrough.
            if scrut_ty == MirType::Trilean {
                use triet_syntax::{LiteralPattern, Pattern, TrileanValue};
                let cur_bb = c.cur;
                let merge_bb = c.alloc_bb();
                let result = c.alloc_local();
                c.push(Statement::StorageLive(result, expr_span.clone()));

                let mut cases: Vec<(i64, BasicBlock)> = Vec::new();
                let mut wildcard_arm: Option<&triet_syntax::MatchArm> = None;

                for arm in arms.iter() {
                    let pat = arena.pattern(arm.pattern);
                    let pat_span = pat.span.clone();
                    if wildcard_arm.is_some() {
                        return Err(LowerError {
                            message: "wildcard `_` must be the last arm in a Trilean match — \
                                      arms after wildcard are unreachable"
                                .to_string(),
                            span: pat_span,
                        });
                    }
                    match &pat.node {
                        Pattern::Wildcard => wildcard_arm = Some(arm),
                        // ADR-0064 §8: a variable binding (`other =>`) is a
                        // catch-all — bound to the scrutinee in the default block.
                        Pattern::Variable(_) => wildcard_arm = Some(arm),
                        Pattern::Literal(LiteralPattern::Trilean(v)) => {
                            let key: i64 = match v {
                                TrileanValue::True => 1,
                                TrileanValue::False => -1,
                                TrileanValue::Unknown => 0,
                            };
                            let arm_bb = c.alloc_bb();
                            cases.push((key, arm_bb));
                            c.cur = arm_bb;
                            c.push_scope();
                            let body_val = lower_expr(arm.body, expected, arena, c)?;
                            let arm_end = c.cur;
                            // ADR-0056: type merge result from the arm value.
                            c.local_decls[result.0].ty = c.local_decls[body_val.0].ty.clone();
                            c.push(Statement::Assign {
                                dest: Place::local(result),
                                source: Place::local(body_val),
                                span: expr_span.clone(),
                            });
                            c.term(
                                arm_end,
                                Terminator::Goto {
                                    target: merge_bb,
                                    span: DUMMY_SPAN,
                                },
                            );
                            c.pop_scope();
                        }
                        other => {
                            return Err(LowerError {
                                message: format!(
                                    "unsupported pattern in Trilean match: {other:?} — expected \
                                     `true`/`false`/`unknown` or `_`"
                                ),
                                span: pat_span,
                            });
                        }
                    }
                }

                // GAP-2: default → wildcard body if present, else Trap.
                let default_bb = if let Some(wc) = wildcard_arm {
                    let wc_bb = c.alloc_bb();
                    c.cur = wc_bb;
                    c.push_scope();
                    bind_scalar_catch_all(c, arena, wc, scrut_local, &scrut_ty, &expr_span);
                    let body_val = lower_expr(wc.body, expected, arena, c)?;
                    let wc_end = c.cur;
                    c.local_decls[result.0].ty = c.local_decls[body_val.0].ty.clone();
                    c.push(Statement::Assign {
                        dest: Place::local(result),
                        source: Place::local(body_val),
                        span: expr_span.clone(),
                    });
                    c.term(
                        wc_end,
                        Terminator::Goto {
                            target: merge_bb,
                            span: DUMMY_SPAN,
                        },
                    );
                    c.pop_scope();
                    wc_bb
                } else {
                    let trap_bb = c.alloc_bb();
                    c.cur = trap_bb;
                    c.term(
                        trap_bb,
                        Terminator::Trap {
                            span: expr_span.clone(),
                        },
                    );
                    trap_bb
                };

                c.term(
                    cur_bb,
                    Terminator::SwitchInt {
                        discriminant: scrut_local,
                        cases,
                        default_bb,
                        span: expr_span.clone(),
                    },
                );
                c.cur = merge_bb;
                return Ok(result);
            }

            // ── ADR-0064 §8 + §A1.2: match on an Integer/Tryte/Long scrutinee ──
            // All three are i64 value-keyed SwitchInt — same key extraction,
            // only the accepted literal suffix and diagnostic name differ. The
            // domain is infinite (Integer/Long) or impractically large (Tryte),
            // so exhaustiveness REQUIRES a wildcard — an uncovered value with no
            // `_` hits the default Trap (GAP-2 runtime; compile-time
            // exhaustiveness lives in typecheck per §A1.2). Mirror of the Trit
            // branch. One helper avoids the 5-copy smell.
            if matches!(scrut_ty, MirType::Integer | MirType::Tryte | MirType::Long) {
                return lower_value_keyed_match(
                    c,
                    arena,
                    arms,
                    scrut_local,
                    &scrut_ty,
                    &expr_span,
                    expected,
                );
            }

            // ── Nullable match: branch on ~+ / ~0 via NULL_SENTINEL ──
            if scrut_ty.is_nullable() {
                use triet_syntax::{MatchArm, OutcomeArm, Pattern};
                let payload_ty = scrut_ty
                    .nullable_payload()
                    .ok_or_else(|| {
                        LowerError::unsupported_expr(
                            &arena.expression(expr_id).node,
                            expr_span.clone(),
                        )
                    })?
                    .clone();

                // ── Scan arms (first-match-wins via slot-per-state ──
                // Guards enforce that the slot model is equivalent to
                // first-match-wins for all accepted programs:
                //   1. No duplicate arms of the same kind.
                //   2. Wildcard must be last (any arm after _ is unreachable).
                //   3. ~+ sub-patterns must be Variable or Wildcard.
                let mut wildcard_arm: Option<&MatchArm> = None;
                let mut present_arm: Option<&MatchArm> = None;
                let mut null_arm: Option<&MatchArm> = None;

                for arm in arms.iter() {
                    let pat = arena.pattern(arm.pattern);
                    let pat_span = pat.span.clone();

                    // Guard 2: any arm after wildcard is unreachable.
                    if wildcard_arm.is_some() {
                        return Err(LowerError {
                            message: "wildcard `_` must be the last arm in a nullable match — \
                                      arms after wildcard are unreachable"
                                .to_string(),
                            span: pat_span,
                        });
                    }

                    match &pat.node {
                        Pattern::Wildcard => wildcard_arm = Some(arm),
                        Pattern::OutcomeArm {
                            arm: OutcomeArm::Positive,
                            payload,
                        } => {
                            // Guard 1: duplicate ~+ arm.
                            if present_arm.is_some() {
                                return Err(LowerError {
                                    message: "duplicate `~+` arm in nullable match".to_string(),
                                    span: pat_span,
                                });
                            }
                            // Guard 3: sub-pattern must be Variable or Wildcard.
                            if let Some(sub_pat) = payload {
                                match &arena.pattern(*sub_pat).node {
                                    Pattern::Variable(_) | Pattern::Wildcard => {}
                                    other => {
                                        return Err(LowerError {
                                            message: format!(
                                                "unsupported sub-pattern in `~+` arm: \
                                                 {other:?} — only variable bindings and `_` \
                                                 are supported"
                                            ),
                                            span: arena.pattern(*sub_pat).span.clone(),
                                        });
                                    }
                                }
                            }
                            present_arm = Some(arm);
                        }
                        Pattern::OutcomeArm {
                            arm: OutcomeArm::Zero,
                            ..
                        } => {
                            // Guard 1: duplicate ~0 arm.
                            if null_arm.is_some() {
                                return Err(LowerError {
                                    message: "duplicate `~0` arm in nullable match".to_string(),
                                    span: pat_span,
                                });
                            }
                            null_arm = Some(arm);
                        }
                        Pattern::OutcomeArm {
                            arm: OutcomeArm::Negative,
                            ..
                        } => {
                            return Err(LowerError {
                                message:
                                    "`~-` arm on nullable type — typechecker should have rejected this"
                                        .to_string(),
                                span: pat_span,
                            });
                        }
                        other => {
                            return Err(LowerError {
                                message: format!(
                                    "unsupported match pattern on nullable scrutinee: {other:?}"
                                ),
                                span: pat_span,
                            });
                        }
                    }
                }

                // Wildcard-only: no branching needed.
                if let Some(arm) = wildcard_arm
                    && present_arm.is_none()
                    && null_arm.is_none()
                {
                    c.push_scope();
                    let body_val = lower_expr(arm.body, expected, arena, c)?;
                    let result = c.alloc_local_ty(c.local_decls[body_val.0].ty.clone());
                    c.push(Statement::StorageLive(result, expr_span.clone()));
                    c.push(Statement::Assign {
                        dest: Place::local(result),
                        source: Place::local(body_val),
                        span: expr_span.clone(),
                    });
                    c.pop_scope();
                    return Ok(result);
                }

                // ── Branch-based lowering (Elvis pattern) ──
                let merge_bb = c.alloc_bb();
                let result = c.alloc_local_ty(payload_ty.clone());
                c.push(Statement::StorageLive(result, expr_span.clone()));

                let null_bb = c.alloc_bb();
                let present_bb = c.alloc_bb();

                // Create NULL_SENTINEL constant.
                let sentinel = c.alloc_local();
                c.push(Statement::StorageLive(sentinel, expr_span.clone()));
                c.push(Statement::Const {
                    dest: Place::local(sentinel),
                    value: ConstValue::Integer(i128::from(triet_mir::NULL_SENTINEL)),
                    span: expr_span.clone(),
                });

                // Compare scrutinee == NULL_SENTINEL.
                let cmp = c.alloc_local();
                c.push(Statement::StorageLive(cmp, expr_span.clone()));
                c.push(Statement::BinaryOp {
                    dest: Place::local(cmp),
                    op: triet_mir::BinOp::Eq,
                    left: Place::local(scrut_local),
                    right: Place::local(sentinel),
                    span: expr_span.clone(),
                });

                let cur_bb = c.cur;
                c.term(
                    cur_bb,
                    Terminator::If {
                        cond: cmp,
                        positive_bb: null_bb,
                        zero_bb: None,
                        negative_bb: present_bb,
                        span: expr_span.clone(),
                    },
                );

                // Helper: lower an arm body (no pattern binding).
                let lower_arm_no_bind = |arm: &MatchArm,
                                         c: &mut Ctx,
                                         arena,
                                         result: Local,
                                         merge_bb: BasicBlock,
                                         expr_span: Span|
                 -> Result<(), LowerError> {
                    c.push_scope();
                    let body_val = lower_expr(arm.body, expected, arena, c)?;
                    let arm_end = c.cur;
                    // ADR-0056: merge result type from the arm value. The arm
                    // body need not have the payload type (e.g. `~+ v => 1`
                    // returns Integer); pinning result to payload_ty would make
                    // a >8B Enum? result aggregate-copy a scalar → segfault.
                    c.local_decls[result.0].ty = c.local_decls[body_val.0].ty.clone();
                    c.push(Statement::Assign {
                        dest: Place::local(result),
                        source: Place::local(body_val),
                        span: expr_span.clone(),
                    });
                    c.term(
                        arm_end,
                        Terminator::Goto {
                            target: merge_bb,
                            span: DUMMY_SPAN,
                        },
                    );
                    c.pop_scope();
                    Ok(())
                };

                // ── Null branch: ~0 arm or wildcard fallback ──
                c.cur = null_bb;
                let arm_for_null = null_arm.or(wildcard_arm).ok_or_else(|| LowerError {
                    message: "nullable match: no arm for null (~0) state — \
                              typechecker should have rejected this"
                        .to_string(),
                    span: expr_span.clone(),
                })?;
                lower_arm_no_bind(arm_for_null, c, arena, result, merge_bb, expr_span.clone())?;

                // ── Present branch: ~+ arm or wildcard fallback ──
                c.cur = present_bb;
                let arm_for_present = present_arm.or(wildcard_arm).ok_or_else(|| LowerError {
                    message: "nullable match: no arm for present (~+) state — \
                              typechecker should have rejected this"
                        .to_string(),
                    span: expr_span.clone(),
                })?;

                c.push_scope();
                // Bind ~+ variable if present and has a variable sub-pattern.
                if let Pattern::OutcomeArm {
                    arm: OutcomeArm::Positive,
                    payload: Some(sub_pat),
                } = &arena.pattern(arm_for_present.pattern).node
                {
                    let sub_pat = arena.pattern(*sub_pat);
                    if let Pattern::Variable(var_name) = &sub_pat.node {
                        // PA-3c: identity — scrutinee IS the payload.
                        let bind_local = c.alloc_local_ty(payload_ty.clone());
                        c.push(Statement::StorageLive(bind_local, expr_span.clone()));
                        c.push(Statement::Assign {
                            dest: Place::local(bind_local),
                            source: Place::local(scrut_local),
                            span: expr_span.clone(),
                        });
                        c.vars.insert(var_name.clone(), bind_local);
                        c.push_owned(bind_local);
                        // ADR-0076: the present-bind is a value-model COPY of the
                        // niche payload (scrut+tag → bind), so `bind_local` and the
                        // scrutinee now alias the same heap. Tombstone the scrutinee
                        // (Deinit → tag/disc@0 = NULL_SENTINEL) so its join-point
                        // Drop no-ops — the moved-out heap is freed ONCE, by the
                        // bind target. The niche tag IS the drop-flag (no dynamic
                        // drop-flag needed). Only for a Move (heap-bearing)
                        // aggregate; a Copy Struct?/Enum?/scalar never drops, so the
                        // Deinit is unnecessary (and the bind is a real copy, not a
                        // move that would double-free).
                        if !ctx_is_copy(&scrut_ty, c) {
                            c.push(Statement::Deinit(scrut_local, expr_span.clone()));
                        }
                    }
                }
                let body_val = lower_expr(arm_for_present.body, expected, arena, c)?;
                let arm_end = c.cur;
                // ADR-0056: merge result type from the arm value (see above).
                c.local_decls[result.0].ty = c.local_decls[body_val.0].ty.clone();
                c.push(Statement::Assign {
                    dest: Place::local(result),
                    source: Place::local(body_val),
                    span: expr_span.clone(),
                });
                c.term(
                    arm_end,
                    Terminator::Goto {
                        target: merge_bb,
                        span: DUMMY_SPAN,
                    },
                );
                c.pop_scope();

                c.cur = merge_bb;
                return Ok(result);
            }

            // ── Outcome match: branch on disc Trit via If ──
            if matches!(scrut_ty, MirType::Outcome { .. }) {
                use triet_syntax::{MatchArm, OutcomeArm, Pattern};

                let is_ternary = matches!(
                    scrut_ty,
                    MirType::Outcome {
                        allow_null_state: true,
                        ..
                    }
                );

                let mut positive_arm: Option<&MatchArm> = None;
                let mut negative_arm: Option<&MatchArm> = None;
                let mut zero_arm: Option<&MatchArm> = None;
                let mut wildcard_arm: Option<&MatchArm> = None;

                for arm in arms.iter() {
                    let pat = arena.pattern(arm.pattern);
                    let pat_span = pat.span.clone();
                    if wildcard_arm.is_some() {
                        return Err(LowerError {
                            message: "wildcard `_` must be the last arm".to_string(),
                            span: pat_span,
                        });
                    }
                    match &pat.node {
                        Pattern::Wildcard => wildcard_arm = Some(arm),
                        Pattern::OutcomeArm {
                            arm: OutcomeArm::Positive,
                            payload,
                        } => {
                            if positive_arm.is_some() {
                                return Err(LowerError {
                                    message: "duplicate `~+` arm".to_string(),
                                    span: pat_span,
                                });
                            }
                            if let Some(sub) = payload {
                                match &arena.pattern(*sub).node {
                                    Pattern::Variable(_) | Pattern::Wildcard => {}
                                    other => {
                                        return Err(LowerError {
                                            message: format!(
                                                "unsupported sub-pattern in `~+` arm: {other:?}"
                                            ),
                                            span: arena.pattern(*sub).span.clone(),
                                        });
                                    }
                                }
                            }
                            positive_arm = Some(arm);
                        }
                        Pattern::OutcomeArm {
                            arm: OutcomeArm::Negative,
                            payload,
                        } => {
                            if negative_arm.is_some() {
                                return Err(LowerError {
                                    message: "duplicate `~-` arm".to_string(),
                                    span: pat_span,
                                });
                            }
                            if let Some(sub) = payload {
                                match &arena.pattern(*sub).node {
                                    Pattern::Variable(_) | Pattern::Wildcard => {}
                                    other => {
                                        return Err(LowerError {
                                            message: format!(
                                                "unsupported sub-pattern in `~-` arm: {other:?}"
                                            ),
                                            span: arena.pattern(*sub).span.clone(),
                                        });
                                    }
                                }
                            }
                            negative_arm = Some(arm);
                        }
                        Pattern::OutcomeArm {
                            arm: OutcomeArm::Zero,
                            ..
                        } => {
                            if !is_ternary {
                                return Err(LowerError {
                                    message: "`~0` arm on binary Outcome — typechecker should have rejected this".to_string(),
                                    span: pat_span,
                                });
                            }
                            if zero_arm.is_some() {
                                return Err(LowerError {
                                    message: "duplicate `~0` arm".to_string(),
                                    span: pat_span,
                                });
                            }
                            zero_arm = Some(arm);
                        }
                        other => {
                            return Err(LowerError {
                                message: format!(
                                    "unsupported pattern on Outcome scrutinee: {other:?}"
                                ),
                                span: pat_span,
                            });
                        }
                    }
                }

                // Read the Outcome discriminant via projection (offset 0).
                let disc_local = c.alloc_local_ty(MirType::Trit);
                c.push(Statement::StorageLive(disc_local, expr_span.clone()));
                c.push(Statement::Assign {
                    dest: Place::local(disc_local),
                    source: Place::local(scrut_local).project(Projection::OutcomeDiscriminant),
                    span: expr_span.clone(),
                });

                let pos_bb = c.alloc_bb();
                let neg_bb = c.alloc_bb();
                let merge_bb = c.alloc_bb();
                let result = c.alloc_local_ty(MirType::Unknown);
                c.push(Statement::StorageLive(result, expr_span.clone()));

                // Branch on Trit discriminant — 3-way for ternary, 2-way for binary.
                let zero_bb = if is_ternary { Some(c.alloc_bb()) } else { None };
                let cur_bb = c.cur;
                c.term(
                    cur_bb,
                    Terminator::If {
                        cond: disc_local,
                        positive_bb: pos_bb,
                        zero_bb,
                        negative_bb: neg_bb,
                        span: expr_span.clone(),
                    },
                );

                // Helper: lower an arm with optional OutcomeUnwrap bind.
                let lower_outcome_arm = |c: &mut Ctx,
                                         arena: &Arena,
                                         arm: &MatchArm,
                                         unwrap_stmt: &dyn Fn(Local) -> Statement,
                                         needs_deinit: bool,
                                         payload_ty: MirType,
                                         result: Local,
                                         merge_bb: BasicBlock,
                                         expr_span: &Span|
                 -> Result<(), LowerError> {
                    c.push_scope();
                    // Bind variable if present.
                    let mut did_bind = false;
                    if let Pattern::OutcomeArm {
                        payload: Some(sub_pat),
                        ..
                    } = &arena.pattern(arm.pattern).node
                    {
                        let sub = arena.pattern(*sub_pat);
                        if let Pattern::Variable(var_name) = &sub.node {
                            // HP.5: bind type is per-arm — `value_type` for the
                            // `~+` arm, `error_type` for `~-`. A heap error
                            // (e.g. `Integer~String`) must bind as the String
                            // struct, not the success type, or the heap
                            // decompose below targets the wrong layout.
                            let bind_local = c.alloc_local_ty(payload_ty.clone());
                            c.push(Statement::StorageLive(bind_local, expr_span.clone()));
                            if needs_deinit {
                                // HP.3: heap payload → decompose {ptr,len,cap}
                                // from Outcome slot into bind_local's struct slot.
                                let ptr_tmp = c.alloc_local_ty(MirType::Integer);
                                c.push(Statement::StorageLive(ptr_tmp, expr_span.clone()));
                                c.push(Statement::Assign {
                                    dest: Place::local(ptr_tmp),
                                    source: Place::local(scrut_local)
                                        .project(Projection::OutcomePayload),
                                    span: expr_span.clone(),
                                });
                                c.push(Statement::Assign {
                                    dest: Place::local(bind_local)
                                        .project(Projection::Field("ptr".to_string())),
                                    source: Place::local(ptr_tmp),
                                    span: expr_span.clone(),
                                });

                                let len_tmp = c.alloc_local_ty(MirType::Integer);
                                c.push(Statement::StorageLive(len_tmp, expr_span.clone()));
                                c.push(Statement::Assign {
                                    dest: Place::local(len_tmp),
                                    source: Place::local(scrut_local)
                                        .project(Projection::OutcomePayloadLen),
                                    span: expr_span.clone(),
                                });
                                c.push(Statement::Assign {
                                    dest: Place::local(bind_local)
                                        .project(Projection::Field("len".to_string())),
                                    source: Place::local(len_tmp),
                                    span: expr_span.clone(),
                                });

                                let cap_tmp = c.alloc_local_ty(MirType::Integer);
                                c.push(Statement::StorageLive(cap_tmp, expr_span.clone()));
                                c.push(Statement::Assign {
                                    dest: Place::local(cap_tmp),
                                    source: Place::local(scrut_local)
                                        .project(Projection::OutcomePayloadCap),
                                    span: expr_span.clone(),
                                });
                                c.push(Statement::Assign {
                                    dest: Place::local(bind_local)
                                        .project(Projection::Field("cap".to_string())),
                                    source: Place::local(cap_tmp),
                                    span: expr_span.clone(),
                                });
                            } else {
                                c.push(unwrap_stmt(bind_local));
                            }
                            c.vars.insert(var_name.clone(), bind_local);
                            c.push_owned(bind_local);
                            did_bind = true;
                        }
                    }
                    // HP.3: Deinit(o) sau bind heap payload → tombstone disc=0
                    // → drop glue của o (HP.2 SwitchInt) gặp Zero→no-op → chống double-free.
                    if did_bind && needs_deinit {
                        c.push(Statement::Deinit(scrut_local, expr_span.clone()));
                    }
                    let body_val = lower_expr(arm.body, expected, arena, c)?;
                    c.push(Statement::Assign {
                        dest: Place::local(result),
                        source: Place::local(body_val),
                        span: expr_span.clone(),
                    });
                    let arm_end = c.cur;
                    c.term(
                        arm_end,
                        Terminator::Goto {
                            target: merge_bb,
                            span: DUMMY_SPAN,
                        },
                    );
                    c.pop_scope();
                    Ok(())
                };

                // ── Positive arm (~+ x): OutcomeUnwrap → bind payload ──
                c.cur = pos_bb;
                let pos_arm = positive_arm.or(wildcard_arm).ok_or_else(|| LowerError {
                    message: "missing `~+` arm in Outcome match".to_string(),
                    span: expr_span.clone(),
                })?;
                let (pos_needs_deinit, pos_payload_ty) =
                    if let MirType::Outcome { ref value_type, .. } = scrut_ty {
                        (value_type.is_any_heap(), (**value_type).clone())
                    } else {
                        (false, MirType::Unknown)
                    };
                lower_outcome_arm(
                    c,
                    arena,
                    pos_arm,
                    &|bind_local| Statement::Assign {
                        dest: Place::local(bind_local),
                        source: Place::local(scrut_local).project(Projection::OutcomePayload),
                        span: expr_span.clone(),
                    },
                    pos_needs_deinit,
                    pos_payload_ty,
                    result,
                    merge_bb,
                    &expr_span,
                )?;

                // ── Negative arm (~- e): OutcomeUnwrapError → bind payload ──
                c.cur = neg_bb;
                let neg_arm = negative_arm.or(wildcard_arm).ok_or_else(|| LowerError {
                    message: "missing `~-` arm in Outcome match".to_string(),
                    span: expr_span.clone(),
                })?;
                let (neg_needs_deinit, neg_payload_ty) =
                    if let MirType::Outcome { ref error_type, .. } = scrut_ty {
                        (error_type.is_any_heap(), (**error_type).clone())
                    } else {
                        (false, MirType::Unknown)
                    };
                lower_outcome_arm(
                    c,
                    arena,
                    neg_arm,
                    &|bind_local| Statement::Assign {
                        dest: Place::local(bind_local),
                        source: Place::local(scrut_local).project(Projection::OutcomePayload),
                        span: expr_span.clone(),
                    },
                    neg_needs_deinit,
                    neg_payload_ty,
                    result,
                    merge_bb,
                    &expr_span,
                )?;

                // ── Zero arm (~0): no payload bind, just eval body ──
                if let Some(zb) = zero_bb {
                    c.cur = zb;
                    let z_arm = zero_arm.or(wildcard_arm).ok_or_else(|| LowerError {
                        message: "missing `~0` arm in ternary Outcome match".to_string(),
                        span: expr_span.clone(),
                    })?;
                    c.push_scope();
                    // ~0 has no payload — no variable binding.
                    let body_val = lower_expr(z_arm.body, expected, arena, c)?;
                    c.push(Statement::Assign {
                        dest: Place::local(result),
                        source: Place::local(body_val),
                        span: expr_span.clone(),
                    });
                    c.term(
                        c.cur,
                        Terminator::Goto {
                            target: merge_bb,
                            span: DUMMY_SPAN,
                        },
                    );
                    c.pop_scope();
                }

                c.cur = merge_bb;
                return Ok(result);
            }

            // ADR-0084 §AMEND Lát A: a `&0`/`&0 mutable` reference scrutinee
            // (e.g. `~+ ref_msg` unwrapped from `get(&0 xs, i)`) is a raw
            // pointer local — every read against it below (GetDiscriminant
            // AND the payload-bind Assign) must go through a leading `Deref`
            // so the MIR verifier (INV 4i-4) and JIT (`load_place`) route it
            // through the pointer instead of misreading the pointer bits as
            // the discriminant/payload (WO §0 mine). A plain (non-reference)
            // scrutinee keeps the unprojected `Place::local` unchanged.
            let scrut_place = if matches!(scrut_ty, MirType::Reference { .. }) {
                Place::local(scrut_local).project(Projection::Deref)
            } else {
                Place::local(scrut_local)
            };

            // Read discriminant.
            let disc_local = c.alloc_local();
            c.push(Statement::StorageLive(disc_local, expr_span.clone()));
            c.push(Statement::GetDiscriminant {
                dest: Place::local(disc_local),
                source: scrut_place.clone(),
                span: expr_span.clone(),
            });

            // Build target blocks for each arm and the final merge.
            let cur_bb = c.cur;
            let merge_bb = c.alloc_bb();
            let result = c.alloc_local();
            c.push(Statement::StorageLive(result, expr_span.clone()));

            let mut cases: Vec<(i64, BasicBlock)> = Vec::new();

            // C2: Scan for the catch-all arm BEFORE lowering. ADR-0071 Lát 2
            // §2.A: a bare `Variable` binding is a catch-all exactly like `_`
            // (it binds the scrutinee — see `bind_scalar_catch_all` in the
            // default block). The catch-all must be the last arm (arms after
            // are unreachable).
            let mut wildcard_arm: Option<&triet_syntax::MatchArm> = None;
            for arm in arms.iter() {
                let pat = arena.pattern(arm.pattern);
                let is_catch_all = matches!(
                    &pat.node,
                    triet_syntax::Pattern::Wildcard | triet_syntax::Pattern::Variable(_)
                );
                if is_catch_all {
                    if wildcard_arm.is_some() {
                        return Err(LowerError {
                            message: "duplicate catch-all (`_` or binding) in enum match"
                                .to_string(),
                            span: expr_span.clone(),
                        });
                    }
                    wildcard_arm = Some(arm);
                } else if wildcard_arm.is_some() {
                    return Err(LowerError {
                        message: "catch-all (`_` or binding) must be the last arm in an enum \
                                  match — arms after it are unreachable"
                            .to_string(),
                        span: expr_span.clone(),
                    });
                }
            }

            // Pre-allocate arm blocks and emit body lowering for each
            // non-wildcard arm. Variant resolution is provided by the type
            // checker via pattern_resolutions.
            for arm in arms.iter() {
                let pat = arena.pattern(arm.pattern);

                // Skip the catch-all (`_` or bare binding) — lowered
                // separately as default_bb (ADR-0071 Lát 2 §2.A).
                if matches!(
                    &pat.node,
                    triet_syntax::Pattern::Wildcard | triet_syntax::Pattern::Variable(_)
                ) {
                    continue;
                }

                let arm_bb = c.alloc_bb();

                // Look up the type checker's resolution for this pattern.
                // Clone to release the immutable borrow before mutating ctx.
                let resolution = c.pattern_resolutions.get(&arm.pattern).cloned();

                match &pat.node {
                    triet_syntax::Pattern::EnumVariant {
                        variant_name,
                        payload: sub_pattern,
                        ..
                    } => {
                        let res = resolution.ok_or_else(|| LowerError {
                            message: format!(
                                "unresolved enum variant '{variant_name}' — type checker should have resolved this"
                            ),
                            span: expr_span.clone(),
                        })?;
                        cases.push((res.discriminant, arm_bb));

                        c.cur = arm_bb;
                        c.push_scope();

                        // Bind payload variable if present.
                        if let Some(sub_pat) = sub_pattern {
                            let sub_pat = arena.pattern(*sub_pat);
                            match &sub_pat.node {
                                triet_syntax::Pattern::Variable(var_name) => {
                                    let payload_ty = c
                                        .enum_layouts
                                        .get(&res.enum_name)
                                        .and_then(|layout| {
                                            layout
                                                .variants
                                                .iter()
                                                .find(|v| v.name == res.variant_name)
                                                .and_then(|v| {
                                                    v.payload.as_ref().map(|p| p.ty.clone())
                                                })
                                        })
                                        .unwrap_or(MirType::Unknown);
                                    // ADR-0084 §AMEND Lát B: an aggregate/heap
                                    // payload reached THROUGH a reference
                                    // (`scrut_place` carries a leading `Deref`
                                    // — set above when `scrut_ty` is
                                    // `MirType::Reference`) is a zero-copy
                                    // SUB-BORROW, not a value read or move-out
                                    // (mirrors Slice 1b `Expr::FieldAccess`,
                                    // lib.rs ~3427-3462). Only fires for
                                    // Struct/Enum/heap payload kinds typecheck
                                    // now binds as a `&0` reference (formerly
                                    // E1050-refused); the scalar terminal case
                                    // (Lát A) and the owned-scrutinee case (no
                                    // `Deref`) both fall through to the
                                    // unchanged `Assign` path below.
                                    //
                                    // TWO sub-cases, NOT one — probed live
                                    // (fixtures 406/407 silent-MISS before this
                                    // split; WO §4 flagged Vector/HashMap as
                                    // O-unmeasured):
                                    // - Struct/Enum/String are INLINE-repr
                                    //   (their bytes live where they're
                                    //   stored — struct_slots/enum_slots) —
                                    //   `&0 T` must be the ADDRESS of those
                                    //   bytes → `Statement::Borrow`.
                                    // - Vector/HashMap are HANDLE-repr (the
                                    //   "value" IS ALREADY an opaque i64
                                    //   pointer to Rust-heap state — see
                                    //   `walk_projections`'s Deref doc + the
                                    //   bare-local `Expr::Borrow` codegen,
                                    //   which for a non-slot-backed local
                                    //   (Vector/HashMap are never in
                                    //   struct_slots/enum_slots) falls to
                                    //   `use_var` = a HANDLE COPY, not an
                                    //   address). A `&0 Vector` is therefore
                                    //   bit-identical to a `Vector` value at
                                    //   runtime — the loan is enforced by
                                    //   borrowck, not by an extra indirection.
                                    //   Emitting `Statement::Borrow` here would
                                    //   yield the ADDRESS of the payload's
                                    //   8-byte handle slot instead of the
                                    //   handle itself — every heap shim call
                                    //   (`__triet_vector_len` etc.) reads that
                                    //   address as if it WERE the handle →
                                    //   silent-MISS garbage. Fix: LOAD the
                                    //   handle via the pre-existing scalar
                                    //   `Assign` mechanics (identical to the
                                    //   Lát A Integer-payload path — Vector's
                                    //   handle is also 8 bytes), but type the
                                    //   dest as `Reference{..}` (so
                                    //   `len(&0 Vector)` overload resolution +
                                    //   borrowck loan-tracking apply) and skip
                                    //   `push_owned` (no Drop — the copied
                                    //   handle bit-aliases the container's
                                    //   copy; a Drop would double-free).
                                    let scrut_has_deref = scrut_place
                                        .projection
                                        .iter()
                                        .any(|p| matches!(p, Projection::Deref));
                                    let is_inline_aggregate = matches!(
                                        &payload_ty,
                                        MirType::Struct(_) | MirType::Enum(_)
                                    ) || payload_ty.is_string_repr();
                                    let is_handle_heap = matches!(
                                        &payload_ty,
                                        MirType::Vector(_) | MirType::HashMap(_, _)
                                    );
                                    if scrut_has_deref && is_inline_aggregate {
                                        let ref_ty = MirType::Reference {
                                            form: triet_mir::ReferenceForm::BorrowReadOnly,
                                            inner: Box::new(payload_ty.clone()),
                                        };
                                        let d = c.alloc_local_ty(ref_ty);
                                        c.push(Statement::StorageLive(d, expr_span.clone()));
                                        c.push(Statement::Borrow {
                                            dest: Place::local(d),
                                            form: triet_mir::ReferenceForm::BorrowReadOnly,
                                            source: scrut_place
                                                .clone()
                                                .project(Projection::Payload(variant_name.clone())),
                                            span: expr_span.clone(),
                                        });
                                        c.vars.insert(var_name.clone(), d);
                                    } else if scrut_has_deref && is_handle_heap {
                                        let ref_ty = MirType::Reference {
                                            form: triet_mir::ReferenceForm::BorrowReadOnly,
                                            inner: Box::new(payload_ty.clone()),
                                        };
                                        let d = c.alloc_local_ty(ref_ty);
                                        c.push(Statement::StorageLive(d, expr_span.clone()));
                                        c.push(Statement::Assign {
                                            dest: Place::local(d),
                                            source: scrut_place
                                                .clone()
                                                .project(Projection::Payload(variant_name.clone())),
                                            span: expr_span.clone(),
                                        });
                                        c.vars.insert(var_name.clone(), d);
                                        // Deliberately NOT push_owned — `d` is a
                                        // handle-copy loan, not an owning
                                        // binding (see comment above).
                                    } else {
                                        let bind_local = c.alloc_local_ty(payload_ty.clone());
                                        // ADR-0060 P2-Boundary: aggregate payload
                                        // needs a stack slot (StructAlloc) so the
                                        // JIT can resolve its address.
                                        if let MirType::Struct(name) = &payload_ty {
                                            c.push(Statement::StructAlloc {
                                                dest: bind_local,
                                                struct_name: name.clone(),
                                                span: expr_span.clone(),
                                            });
                                        }
                                        c.push(Statement::StorageLive(
                                            bind_local,
                                            expr_span.clone(),
                                        ));
                                        // Read payload into the binding. ADR-0084
                                        // §AMEND Lát A: `scrut_place` carries the
                                        // leading `Deref` when the scrutinee is a
                                        // reference (typecheck already refused a
                                        // non-scalar bind through one — Cọc 1a —
                                        // so `payload_ty` here is always scalar
                                        // Copy when `scrut_place` is Deref'd).
                                        c.push(Statement::Assign {
                                            dest: Place::local(bind_local),
                                            source: scrut_place
                                                .clone()
                                                .project(Projection::Payload(variant_name.clone())),
                                            span: expr_span.clone(),
                                        });
                                        c.vars.insert(var_name.clone(), bind_local);
                                        c.push_owned(bind_local);
                                    }
                                }
                                triet_syntax::Pattern::Wildcard => {
                                    // _ — do nothing, no binding
                                }
                                other => {
                                    return Err(LowerError {
                                        message: format!(
                                            "unsupported match sub-pattern: {other:?}"
                                        ),
                                        span: expr_span.clone(),
                                    });
                                }
                            }
                        }

                        // Lower arm body.
                        let body_val = lower_expr(arm.body, expected, arena, c)?;
                        let arm_end = c.cur;
                        // ADR-0056: type the merge result from the arm value so
                        // a Fat-Pointer survives as a typed move. Idempotent —
                        // typecheck guarantees every arm has the same type.
                        c.local_decls[result.0].ty = c.local_decls[body_val.0].ty.clone();
                        c.push(Statement::Assign {
                            dest: Place::local(result),
                            source: Place::local(body_val),
                            span: expr_span.clone(),
                        });
                        c.term(
                            arm_end,
                            Terminator::Goto {
                                target: merge_bb,
                                span: DUMMY_SPAN,
                            },
                        );
                        c.pop_scope();
                    }
                    // ADR-0071 Lát 2: a bare `Pattern::Variable` is the
                    // catch-all binding, already skipped above and lowered as
                    // `default_bb`. It never reaches this match.
                    other => {
                        return Err(LowerError {
                            message: format!(
                                "unsupported match pattern (expected enum variant): {other:?}"
                            ),
                            span: expr_span.clone(),
                        });
                    }
                }
            }

            // C2: catch-all arm → default_bb instead of trap.
            let default_bb = if let Some(wc) = wildcard_arm {
                let wc_bb = c.alloc_bb();
                c.cur = wc_bb;
                c.push_scope();
                // ADR-0071 Lát 2 §2.A: a bare `Variable` catch-all binds the
                // scrutinee into scope (no-op for `_`). REFUSE-NARROW for a
                // heap-payload (non-copy) enum: the binding is a value-copy of
                // the enum, which would alias the heap pointer and double-free
                // on drop. (WO chỉ thị #3 — refuse, never miscompile silently.)
                if matches!(
                    &arena.pattern(wc.pattern).node,
                    triet_syntax::Pattern::Variable(_)
                ) {
                    if !ctx_is_copy(&scrut_ty, c) {
                        return Err(LowerError::heap_type_not_supported(
                            &format!(
                                "bare binding catch-all on a heap-payload enum `{scrut_ty}` \
                                 (would alias + double-free) — match the variants explicitly"
                            ),
                            expr_span,
                        ));
                    }
                    bind_scalar_catch_all(c, arena, wc, scrut_local, &scrut_ty, &expr_span);
                }
                let body_val = lower_expr(wc.body, expected, arena, c)?;
                let wc_end = c.cur;
                // ADR-0056: type the merge result from the wildcard arm value.
                c.local_decls[result.0].ty = c.local_decls[body_val.0].ty.clone();
                c.push(Statement::Assign {
                    dest: Place::local(result),
                    source: Place::local(body_val),
                    span: expr_span.clone(),
                });
                c.term(
                    wc_end,
                    Terminator::Goto {
                        target: merge_bb,
                        span: DUMMY_SPAN,
                    },
                );
                c.pop_scope();
                wc_bb
            } else {
                // No wildcard → trap on unknown discriminant.
                let trap_bb = c.alloc_bb();
                c.cur = trap_bb;
                c.term(
                    trap_bb,
                    Terminator::Trap {
                        span: expr_span.clone(),
                    },
                );
                trap_bb
            };

            // Emit SwitchInt terminator.
            c.term(
                cur_bb,
                Terminator::SwitchInt {
                    discriminant: disc_local,
                    cases,
                    default_bb,
                    span: expr_span.clone(),
                },
            );

            c.cur = merge_bb;
            Ok(result)
        }
        Expr::MethodCall {
            receiver,
            method: _,
            arguments,
        } => {
            // ADR-0061 T5.1/T5.3: trait-method dispatch. The type checker
            // resolved this call to a concrete mangled function; emit a
            // direct CallDispatch to its Body (no second table lookup — the
            // mangled name is taken straight from MethodResolution). The
            // receiver becomes arg[0]; explicit args follow.
            if let Some(res) = c.method_resolutions.get(&expr_id).cloned() {
                let callee_name = res.concrete_fn;
                let callee_ret = c
                    .func_return_types
                    .get(&callee_name)
                    .cloned()
                    .unwrap_or(MirType::Integer);
                // ADR-0061 nợ #2: trait methods now lower fat returns through
                // the same sret / 2-register machinery as Expr::Call. Support
                // set = {Struct, String, heap-binary-Outcome, scalar-Outcome,
                // scalar}. Everything else (Vector / HashMap / Enum /
                // Reference) is refused below — falling into the scalar branch
                // would miscompile a fat pointer into a single i64 (a silent
                // soundness hole), so refuse cleanly instead.
                let is_outcome_ret = matches!(callee_ret, MirType::Outcome { .. });
                let is_heap_outcome_ret = is_outcome_ret && callee_ret.has_heap_payload();
                // ADR-0062 Lát 4.5: `String?` method return shares String's fat
                // sret path (mirror Expr::Call). `is_string_repr()` covers both
                // `String` and `String?` (Nullable(String)).
                let is_fat_ret = matches!(callee_ret, MirType::Struct(_))
                    || callee_ret.is_string_repr()
                    || is_heap_outcome_ret;
                // Only SCALAR nullables (Integer? etc., PA-3c single-i64) count
                // as scalar returns. `Nullable(_)` blanket-matching swallowed
                // `String?`/`Vector?`/etc. into the scalar branch → miscompile a
                // fat pointer into one i64. Narrow to the existing
                // `is_scalar_nullable_payload` predicate; non-scalar nullables
                // fall through to the clean refuse below.
                let scalar_ret = matches!(
                    callee_ret,
                    MirType::Integer
                        | MirType::Trit
                        | MirType::Tryte
                        | MirType::Long
                        | MirType::Trilean
                        | MirType::Unit
                        | MirType::Unknown
                ) || matches!(
                    &callee_ret,
                    MirType::Nullable(inner) if triet_mir::is_scalar_nullable_payload(inner)
                );
                // sret slot layout name: `String?` reprs as the "String" layout
                // (ptr-sentinel, same 24-byte slot); `to_string()` would yield
                // "String?", which has no registered layout (mirror Call:2245).
                let sret_layout_name = if callee_ret.is_string_repr() {
                    "String".to_string()
                } else {
                    callee_ret.to_string()
                };
                // Receiver becomes arg[0]; explicit args follow. For a fat
                // return the hidden sret pointer is inserted BEFORE the
                // receiver → [sret, receiver, explicit...].
                let mut args = Vec::with_capacity(arguments.len() + 1);
                args.push(lower_expr(*receiver, None, arena, c)?);
                for &a in arguments {
                    args.push(lower_expr(a, None, arena, c)?);
                }
                if is_fat_ret {
                    // sret: allocate the struct/string/heap-outcome return
                    // local and pass it as the hidden arg[0] (ADR-0049/0058).
                    let ret_local = c.alloc_local_ty(callee_ret.clone());
                    c.push(Statement::StorageLive(ret_local, expr_span.clone()));
                    if is_heap_outcome_ret {
                        c.push(Statement::OutcomeAlloc {
                            dest: ret_local,
                            span: expr_span.clone(),
                        });
                    } else {
                        c.push(Statement::StructAlloc {
                            dest: ret_local,
                            struct_name: sret_layout_name.clone(),
                            span: expr_span.clone(),
                        });
                    }
                    args.insert(0, ret_local);
                    // Zero Move-type args; skip arg[0] (sret pointer) and
                    // borrows + Copy scalars (ADR-0042 Q1 + ADR-0045 §2).
                    let to_zero: Vec<Local> = args[1..]
                        .iter()
                        .filter(|&&arg| {
                            let ty = &c.local_decls[arg.0].ty;
                            if ty.is_reference() {
                                return false;
                            }
                            !ctx_is_copy(ty, c)
                        })
                        .copied()
                        .collect();
                    let ret_bb = c.alloc_bb();
                    let call_bb = c.cur;
                    c.term(
                        call_bb,
                        Terminator::CallDispatch {
                            callee: triet_mir::FunctionId(0),
                            callee_name,
                            target: CallTarget::Jit,
                            args,
                            return_bb: ret_bb,
                            dest: Vec::new(),
                            return_shape: triet_mir::ReturnShape::Struct {
                                struct_name: sret_layout_name,
                            },
                            span: expr_span,
                        },
                    );
                    c.cur = ret_bb;
                    for &arg in &to_zero {
                        c.push(Statement::Deinit(arg, DUMMY_SPAN));
                    }
                    return Ok(ret_local);
                } else if is_outcome_ret {
                    // ADR-0052 OP.4a: scalar Outcome — slot + 2 return values.
                    let dest = c.alloc_local_ty(callee_ret.clone());
                    c.push(Statement::StorageLive(dest, expr_span.clone()));
                    c.push(Statement::OutcomeAlloc {
                        dest,
                        span: expr_span.clone(),
                    });
                    let to_zero: Vec<Local> = args
                        .iter()
                        .filter(|&&arg| {
                            let ty = &c.local_decls[arg.0].ty;
                            if ty.is_reference() {
                                return false;
                            }
                            !ctx_is_copy(ty, c)
                        })
                        .copied()
                        .collect();
                    let ret_bb = c.alloc_bb();
                    let call_bb = c.cur;
                    c.term(
                        call_bb,
                        Terminator::CallDispatch {
                            callee: triet_mir::FunctionId(0),
                            callee_name,
                            target: CallTarget::Jit,
                            args,
                            return_bb: ret_bb,
                            dest: vec![dest],
                            return_shape: match &callee_ret {
                                MirType::Outcome {
                                    allow_null_state: true,
                                    ..
                                } => triet_mir::ReturnShape::TernaryOutcome,
                                _ => triet_mir::ReturnShape::BinaryOutcome,
                            },
                            span: expr_span,
                        },
                    );
                    c.cur = ret_bb;
                    for &arg in &to_zero {
                        c.push(Statement::Deinit(arg, DUMMY_SPAN));
                    }
                    return Ok(dest);
                } else if scalar_ret {
                    let dest = c.alloc_local_ty(callee_ret);
                    c.push(Statement::StorageLive(dest, expr_span.clone()));
                    // ADR-0042 Q1 + ADR-0045 §2: zero Move-type args (skip
                    // borrows + Copy scalars) — same rule as the scalar call.
                    let to_zero: Vec<Local> = args
                        .iter()
                        .filter(|&&arg| {
                            let ty = &c.local_decls[arg.0].ty;
                            if ty.is_reference() {
                                return false;
                            }
                            !ctx_is_copy(ty, c)
                        })
                        .copied()
                        .collect();
                    let ret_bb = c.alloc_bb();
                    let call_bb = c.cur;
                    c.term(
                        call_bb,
                        Terminator::CallDispatch {
                            callee: triet_mir::FunctionId(0),
                            callee_name,
                            target: CallTarget::Jit,
                            args,
                            return_bb: ret_bb,
                            dest: vec![dest],
                            return_shape: triet_mir::ReturnShape::Scalar,
                            span: expr_span,
                        },
                    );
                    c.cur = ret_bb;
                    for &arg in &to_zero {
                        c.push(Statement::Deinit(arg, DUMMY_SPAN));
                    }
                    return Ok(dest);
                }
                // Refuse NARROW (nợ #2 §2): Vector / HashMap / Enum /
                // Reference trait-method returns still need their own ABI.
                // Refuse rather than fall through to a scalar miscompile.
                return Err(LowerError {
                    message: format!(
                        "trait method `{callee_name}` returns `{callee_ret}` — \
                         Vector/HashMap/Enum/Reference returns deferred (nợ #2 scope)"
                    ),
                    span: expr_span,
                });
            }

            // ADR-0071 Lát 2: the `OptionA.SomeInt(42)` MethodCall dot-form
            // variant construction is gone (qualified `OptionA::SomeInt(42)`
            // parses as `Expr::EnumLiteral`). A MethodCall that is not a
            // resolved trait method is unsupported.
            Err(LowerError::unsupported_expr(
                &arena.expression(expr_id).node,
                expr_span,
            ))
        }
        Expr::Return { value } => {
            // `return` in expression position (e.g. inside a `~->` propagate
            // closure body). ADR-0072 §2.3: this is a value-context SOURCE — the
            // function return type is the expected type, so `~+`/`~0`/`~-`
            // consume it in the leaf-consumer (fixtures 115/116 `~-> |e| return
            // ~- e`). The is_null special-case is gone (§2.4 generalises it).
            let mut values = Vec::new();
            if let Some(v) = value {
                let ret_ty = c.sig.return_type.clone();
                let val = lower_expr(*v, Some(&ret_ty), arena, c)?;
                values.extend(lower_outcome_return_values(val, c));
            }
            c.flush_all_for_return();
            let cur = c.cur;
            c.term(
                cur,
                Terminator::Return {
                    values,
                    span: expr_span.clone(),
                },
            );
            // After return, control never reaches — allocate dead block.
            let dead = c.alloc_bb();
            c.cur = dead;
            // Return value is unit (never used).
            let u = c.alloc_local();
            c.push(Statement::StorageLive(u, expr_span.clone()));
            c.push(Statement::Const {
                dest: Place::local(u),
                value: ConstValue::Unit,
                span: expr_span,
            });
            Ok(u)
        }
        Expr::OutcomeArmHandler {
            inner,
            arm,
            capture_name,
            body,
        } => {
            // APP.1: ~-> Mode 2 propagate (Negative, Trap-exit).
            // APP.2a: ~+> Mode 1 MAP (Positive, CFG-merge).
            // APP.2c: ~-> Mode 1 MAP (Negative, CFG-merge) — error transformer.
            if matches!(arm, triet_syntax::OutcomeArm::Zero) {
                // APP Mũi-A: ~0> Mode 1 MAP — null-state handler (ternary).
                // Type-preserving (ADR-0020 §3.2): body must match value_type T.
                // Zero → eval body, rewrap as ~+ (null eliminated → binary).
                // Positive/Negative → passthrough inner unchanged.
                let inner_val = lower_expr(*inner, None, arena, c)?;

                let disc = c.alloc_local_ty(MirType::Trit);
                c.push(Statement::StorageLive(disc, expr_span.clone()));
                c.push(Statement::Assign {
                    dest: Place::local(disc),
                    source: Place::local(inner_val).project(Projection::OutcomeDiscriminant),
                    span: expr_span.clone(),
                });

                // Result: binary Outcome (null eliminated).
                let result = c.alloc_local_ty(MirType::Outcome {
                    value_type: Box::new(MirType::Unknown),
                    error_type: Box::new(MirType::Unknown),
                    allow_null_state: false,
                });
                c.push(Statement::StorageLive(result, expr_span.clone()));
                c.push(Statement::OutcomeAlloc {
                    dest: result,
                    span: expr_span.clone(),
                });

                let zero_bb = c.alloc_bb();
                let non_zero_bb = c.alloc_bb();
                let merge_bb = c.alloc_bb();

                // Branch: positive+negative → passthrough, zero → map.
                let cur_bb = c.cur;
                c.term(
                    cur_bb,
                    Terminator::If {
                        cond: disc,
                        positive_bb: non_zero_bb,
                        zero_bb: Some(zero_bb),
                        negative_bb: non_zero_bb,
                        span: expr_span.clone(),
                    },
                );

                // ── Non-zero: copy inner → result (passthrough) ──
                c.cur = non_zero_bb;
                c.push(Statement::Assign {
                    dest: Place::local(result).project(Projection::OutcomeDiscriminant),
                    source: Place::local(inner_val).project(Projection::OutcomeDiscriminant),
                    span: expr_span.clone(),
                });
                c.push(Statement::Assign {
                    dest: Place::local(result).project(Projection::OutcomePayload),
                    source: Place::local(inner_val).project(Projection::OutcomePayload),
                    span: expr_span.clone(),
                });
                c.term(
                    c.cur,
                    Terminator::Goto {
                        target: merge_bb,
                        span: DUMMY_SPAN,
                    },
                );

                // ── Zero: eval body (no capture), rewrap as ~+ ──
                c.cur = zero_bb;
                let body_val = lower_expr(*body, None, arena, c)?;
                let disc_tmp = c.alloc_local_ty(MirType::Trit);
                c.push(Statement::StorageLive(disc_tmp, expr_span.clone()));
                c.push(Statement::Const {
                    dest: Place::local(disc_tmp),
                    value: ConstValue::Trit(1),
                    span: expr_span.clone(),
                });
                c.push(Statement::Assign {
                    dest: Place::local(result).project(Projection::OutcomeDiscriminant),
                    source: Place::local(disc_tmp),
                    span: expr_span.clone(),
                });
                c.push(Statement::Assign {
                    dest: Place::local(result).project(Projection::OutcomePayload),
                    source: Place::local(body_val),
                    span: expr_span.clone(),
                });
                c.term(
                    c.cur,
                    Terminator::Goto {
                        target: merge_bb,
                        span: DUMMY_SPAN,
                    },
                );

                c.cur = merge_bb;
                return Ok(result);
            }
            let is_positive = matches!(arm, triet_syntax::OutcomeArm::Positive);
            // Negative arm with tail-expr body → Mode 1 map (CFG-merge).
            // Negative arm with return body → Mode 2 propagate (existing).
            let is_negative_mode1 = !is_positive
                && !matches!(
                    arena.expression(*body).node,
                    triet_syntax::Expr::Return { .. }
                );

            // Lower inner → Outcome 2-slot value.
            let inner_val = lower_expr(*inner, None, arena, c)?;

            // HP.4: inspect inner Outcome's payload types so each arm can
            // decide scalar (8-byte) vs heap (24-byte {ptr,len,cap}) moves.
            let (inner_value_ty, inner_error_ty) = if let MirType::Outcome {
                value_type,
                error_type,
                ..
            } = &c.local_decls[inner_val.0].ty
            {
                ((**value_type).clone(), (**error_type).clone())
            } else {
                (MirType::Unknown, MirType::Unknown)
            };

            // Read disc via projection (offset 0).
            let disc = c.alloc_local_ty(MirType::Trit);
            c.push(Statement::StorageLive(disc, expr_span.clone()));
            c.push(Statement::Assign {
                dest: Place::local(disc),
                source: Place::local(inner_val).project(Projection::OutcomeDiscriminant),
                span: expr_span.clone(),
            });

            let pos_bb = c.alloc_bb();
            let neg_bb = c.alloc_bb();
            let merge_bb = c.alloc_bb();

            // ── Allocate shared result BEFORE If (both arms write to it) ──
            let result = if is_positive || is_negative_mode1 {
                // Use generic Outcome type — body_ty may differ from sig
                // return type (APP.2b-1 type-change scalar). Both are i64.
                let r = c.alloc_local_ty(MirType::Outcome {
                    value_type: Box::new(MirType::Unknown),
                    error_type: Box::new(MirType::Unknown),
                    allow_null_state: false,
                });
                c.push(Statement::StorageLive(r, expr_span.clone()));
                c.push(Statement::OutcomeAlloc {
                    dest: r,
                    span: expr_span.clone(),
                });
                r
            } else {
                let r = c.alloc_local_ty(MirType::Unknown);
                c.push(Statement::StorageLive(r, expr_span.clone()));
                r
            };

            // Branch on disc Trit.
            let cur_bb = c.cur;
            c.term(
                cur_bb,
                Terminator::If {
                    cond: disc,
                    positive_bb: pos_bb,
                    zero_bb: None,
                    negative_bb: neg_bb,
                    span: expr_span.clone(),
                },
            );

            // ── Error arm (negative_bb) ──
            c.cur = neg_bb;
            if is_positive {
                // APP.2a: error passthrough — copy inner→result.
                c.push(Statement::Assign {
                    dest: Place::local(result).project(Projection::OutcomeDiscriminant),
                    source: Place::local(inner_val).project(Projection::OutcomeDiscriminant),
                    span: expr_span.clone(),
                });
                if inner_error_ty.is_any_heap() {
                    // HP.4: move 24-byte {ptr,len,cap}; Deinit(inner) so its
                    // drop glue cannot also free the error → no double-free.
                    c.copy_heap_outcome_payload(result, inner_val, &expr_span);
                    c.push(Statement::Deinit(inner_val, expr_span.clone()));
                } else {
                    c.push(Statement::Assign {
                        dest: Place::local(result).project(Projection::OutcomePayload),
                        source: Place::local(inner_val).project(Projection::OutcomePayload),
                        span: expr_span.clone(),
                    });
                }
                c.term(
                    c.cur,
                    Terminator::Goto {
                        target: merge_bb,
                        span: DUMMY_SPAN,
                    },
                );
            } else if is_negative_mode1 {
                // APP.2c: bind e, eval body, rewrap ~- into result.
                c.push_scope();
                let mapped_heap = inner_error_ty.is_any_heap();
                if let Some(name) = capture_name {
                    let e_ty = if mapped_heap {
                        inner_error_ty.clone()
                    } else {
                        MirType::Unknown
                    };
                    let e_local = c.alloc_local_ty(e_ty);
                    c.push(Statement::StorageLive(e_local, expr_span.clone()));
                    if mapped_heap {
                        // HP.4: decompose heap error into e; tombstone inner.
                        c.bind_heap_outcome_payload(e_local, inner_val, &expr_span);
                        c.push(Statement::Deinit(inner_val, expr_span.clone()));
                    } else {
                        c.push(Statement::Assign {
                            dest: Place::local(e_local),
                            source: Place::local(inner_val).project(Projection::OutcomePayload),
                            span: expr_span.clone(),
                        });
                    }
                    c.vars.insert(name.clone(), e_local);
                    c.local_names.insert(e_local, name.clone());
                    c.push_owned(e_local);
                }
                let body_val = lower_expr(*body, None, arena, c)?;
                // HP.4: result error type = mapped body type (32-byte slot
                // when heap). Success type = inner success (passthrough).
                let body_ty = c.local_decls[body_val.0].ty.clone();
                let body_heap = body_ty.is_any_heap();
                c.local_decls[result.0].ty = MirType::Outcome {
                    value_type: Box::new(inner_value_ty.clone()),
                    error_type: Box::new(body_ty),
                    allow_null_state: false,
                };
                let disc_tmp = c.alloc_local_ty(MirType::Trit);
                c.push(Statement::StorageLive(disc_tmp, expr_span.clone()));
                c.push(Statement::Const {
                    dest: Place::local(disc_tmp),
                    value: ConstValue::Trit(-1),
                    span: expr_span.clone(),
                });
                c.push(Statement::Assign {
                    dest: Place::local(result).project(Projection::OutcomeDiscriminant),
                    source: Place::local(disc_tmp),
                    span: expr_span.clone(),
                });
                if body_heap {
                    // HP.4 F1: write result THEN Deinit(body_val) so the
                    // pop_scope Drop of the captured/identity value is a no-op
                    // (result now owns it) — no Drop-then-move (UAF) race.
                    c.write_heap_outcome_payload(result, body_val, &expr_span);
                    c.push(Statement::Deinit(body_val, expr_span.clone()));
                } else {
                    c.push(Statement::Assign {
                        dest: Place::local(result).project(Projection::OutcomePayload),
                        source: Place::local(body_val),
                        span: expr_span.clone(),
                    });
                }
                // F1 fix: pop scope AFTER result-write + Deinit, never before.
                c.pop_scope();
                c.term(
                    c.cur,
                    Terminator::Goto {
                        target: merge_bb,
                        span: DUMMY_SPAN,
                    },
                );
            } else {
                // APP.1: bind e, lower return, Trap exit.
                c.push_scope();
                if let Some(name) = capture_name {
                    let error_heap = inner_error_ty.is_any_heap();
                    let e_ty = if error_heap {
                        inner_error_ty.clone()
                    } else {
                        MirType::Unknown
                    };
                    let e_local = c.alloc_local_ty(e_ty);
                    c.push(Statement::StorageLive(e_local, expr_span.clone()));
                    if error_heap {
                        // WO-OutcomeEarlyReturnHeapPayload Site B: same bug
                        // class as HP.4's map-mode branches (4950-4953) — a
                        // flat 1-word Assign only copies the payload `ptr`
                        // (dropping len/cap) and never Deinit(inner_val), so
                        // inner_val's later Drop frees the same buffer `e`
                        // now (partially) owns → double-free. Mirror the
                        // heap-aware decompose + tombstone.
                        c.bind_heap_outcome_payload(e_local, inner_val, &expr_span);
                        c.push(Statement::Deinit(inner_val, expr_span.clone()));
                    } else {
                        c.push(Statement::Assign {
                            dest: Place::local(e_local),
                            source: Place::local(inner_val).project(Projection::OutcomePayload),
                            span: expr_span.clone(),
                        });
                    }
                    c.vars.insert(name.clone(), e_local);
                    c.local_names.insert(e_local, name.clone());
                    c.push_owned(e_local);
                }
                lower_expr(*body, None, arena, c)?;
                c.pop_scope();
                c.term(
                    c.cur,
                    Terminator::Trap {
                        span: expr_span.clone(),
                    },
                );
            }

            // ── Success arm (positive_bb) ──
            c.cur = pos_bb;
            if is_positive {
                // APP.2a: bind v, eval body, rewrap ~+ into result.
                c.push_scope();
                let mapped_heap = inner_value_ty.is_any_heap();
                if let Some(name) = capture_name {
                    let v_ty = if mapped_heap {
                        inner_value_ty.clone()
                    } else {
                        MirType::Unknown
                    };
                    let v_local = c.alloc_local_ty(v_ty);
                    c.push(Statement::StorageLive(v_local, expr_span.clone()));
                    if mapped_heap {
                        // HP.4: decompose heap success into v; tombstone inner.
                        c.bind_heap_outcome_payload(v_local, inner_val, &expr_span);
                        c.push(Statement::Deinit(inner_val, expr_span.clone()));
                    } else {
                        c.push(Statement::Assign {
                            dest: Place::local(v_local),
                            source: Place::local(inner_val).project(Projection::OutcomePayload),
                            span: expr_span.clone(),
                        });
                    }
                    c.vars.insert(name.clone(), v_local);
                    c.local_names.insert(v_local, name.clone());
                    c.push_owned(v_local);
                }
                let body_val = lower_expr(*body, None, arena, c)?;
                // HP.4: result success type = mapped body type (32-byte slot
                // when heap). Error type = inner error (passthrough).
                let body_ty = c.local_decls[body_val.0].ty.clone();
                let body_heap = body_ty.is_any_heap();
                c.local_decls[result.0].ty = MirType::Outcome {
                    value_type: Box::new(body_ty),
                    error_type: Box::new(inner_error_ty.clone()),
                    allow_null_state: false,
                };
                let disc_tmp = c.alloc_local_ty(MirType::Trit);
                c.push(Statement::StorageLive(disc_tmp, expr_span.clone()));
                c.push(Statement::Const {
                    dest: Place::local(disc_tmp),
                    value: ConstValue::Trit(1),
                    span: expr_span.clone(),
                });
                c.push(Statement::Assign {
                    dest: Place::local(result).project(Projection::OutcomeDiscriminant),
                    source: Place::local(disc_tmp),
                    span: expr_span.clone(),
                });
                if body_heap {
                    // HP.4 F1: write result THEN Deinit(body_val) so the
                    // pop_scope Drop of the captured/identity value is a no-op
                    // (result now owns it) — no Drop-then-move (UAF) race.
                    c.write_heap_outcome_payload(result, body_val, &expr_span);
                    c.push(Statement::Deinit(body_val, expr_span.clone()));
                } else {
                    c.push(Statement::Assign {
                        dest: Place::local(result).project(Projection::OutcomePayload),
                        source: Place::local(body_val),
                        span: expr_span.clone(),
                    });
                }
                // F1 fix: pop scope AFTER result-write + Deinit, never before.
                c.pop_scope();
                c.term(
                    c.cur,
                    Terminator::Goto {
                        target: merge_bb,
                        span: DUMMY_SPAN,
                    },
                );
            } else if is_negative_mode1 {
                // APP.2c: success passthrough — copy inner→result.
                c.push(Statement::Assign {
                    dest: Place::local(result).project(Projection::OutcomeDiscriminant),
                    source: Place::local(inner_val).project(Projection::OutcomeDiscriminant),
                    span: expr_span.clone(),
                });
                if inner_value_ty.is_any_heap() {
                    // HP.4: move 24-byte {ptr,len,cap}; Deinit(inner) so its
                    // drop glue cannot also free the success → no double-free.
                    c.copy_heap_outcome_payload(result, inner_val, &expr_span);
                    c.push(Statement::Deinit(inner_val, expr_span.clone()));
                } else {
                    c.push(Statement::Assign {
                        dest: Place::local(result).project(Projection::OutcomePayload),
                        source: Place::local(inner_val).project(Projection::OutcomePayload),
                        span: expr_span.clone(),
                    });
                }
                c.term(
                    c.cur,
                    Terminator::Goto {
                        target: merge_bb,
                        span: DUMMY_SPAN,
                    },
                );
            } else {
                // APP.1: unwrap payload.
                if inner_value_ty.is_any_heap() {
                    // WO-OutcomeEarlyReturnHeapPayload Site A: same bug class
                    // as HP.4's map-mode branches (5114-5118) — a flat
                    // 1-word Assign only copies the payload `ptr` (dropping
                    // len/cap, `result` stays typed Unknown so the {len,cap}
                    // slot fields are never even allocated for it) and never
                    // Deinit(inner_val), so inner_val's later Drop frees the
                    // same buffer `result` now (partially) owns → truncated
                    // reads downstream AND double-free. Retype `result` to
                    // the unwrapped heap type and mirror the heap-aware
                    // decompose + tombstone.
                    c.local_decls[result.0].ty = inner_value_ty.clone();
                    c.bind_heap_outcome_payload(result, inner_val, &expr_span);
                    c.push(Statement::Deinit(inner_val, expr_span.clone()));
                } else {
                    c.push(Statement::Assign {
                        dest: Place::local(result),
                        source: Place::local(inner_val).project(Projection::OutcomePayload),
                        span: expr_span.clone(),
                    });
                }
                c.term(
                    c.cur,
                    Terminator::Goto {
                        target: merge_bb,
                        span: DUMMY_SPAN,
                    },
                );
            }

            c.cur = merge_bb;
            Ok(result)
        }
        // ADR-0061/ADR-0039 recon (Phase 14.0): general first-class /
        // escaping closures are sealed (YAGNI). Nullable/Outcome operator
        // families (`?+>`, `~->`) lower via dedicated inline AST nodes —
        // there is no first-class closure consumer, so a `Lambda` reaching
        // the lowerer is refused explicitly rather than via the generic
        // catch-all (clearer diagnostic + intentional seal, not a gap).
        // ADR-0061/ADR-0039 recon (Phase 14.0): general first-class /
        // escaping closures are sealed (YAGNI). Nullable/Outcome operator
        // families (`?+>`, `~->`) lower via dedicated inline AST nodes —
        // there is no first-class closure consumer, so a `Lambda` reaching
        // the lowerer is refused explicitly rather than via the generic
        // catch-all (clearer diagnostic + intentional seal, not a gap).
        Expr::Lambda { .. } => Err(LowerError {
            message: "general escaping closure sealed (YAGNI per ADR-0039 recon — \
                      nullable/Outcome ops use inline nodes, no first-class closure consumer)"
                .to_string(),
            span: expr_span,
        }),
        other => Err(LowerError::unsupported_expr(other, expr_span)),
    }
}

fn lower_ref_form(form: triet_syntax::type_ast::ReferenceForm) -> triet_mir::ReferenceForm {
    use triet_mir::ReferenceForm as MRF;
    use triet_syntax::type_ast::ReferenceForm as RF;
    match form {
        RF::StrongFrozen => MRF::StrongFrozen,
        RF::StrongMutable => MRF::StrongMutable,
        RF::BorrowReadOnly => MRF::BorrowReadOnly,
        RF::BorrowExclusiveMutable => MRF::BorrowExclusiveMutable,
        RF::WeakObserver => MRF::WeakObserver,
    }
}

fn binop_result_type(op: &BinaryOperator) -> triet_mir::MirType {
    // FIXME(Mentor_G): Đây là tech-debt do Typechecker không pass type xuống MIR.
    // Phải đập đi xây lại bằng Typecheck->MIR bridge (Option C) ở một campaign khác.
    use BinaryOperator::*;
    match op {
        Add | Sub | Mul | Div | Mod | Pow => triet_mir::MirType::Integer,
        Eq | Ne | Lt | Le | Gt | Ge | LukAnd | LukOr | LukXor | LukImplies | LukIff
        | KleeneImplies | KleeneXor | KleeneIff => triet_mir::MirType::Trilean,
    }
}

/// ADR-0064 §8: bind a scalar-match Variable catch-all (`other =>`) to the
/// scrutinee value. No-op for `_` wildcard. Scalar types (Integer/Trit/Trilean)
/// are Copy — no push_owned / Drop. Mirror of the nullable bind idiom.
fn bind_scalar_catch_all(
    c: &mut Ctx,
    arena: &Arena,
    catch_all: &triet_syntax::MatchArm,
    scrut_local: Local,
    scrut_ty: &MirType,
    span: &Span,
) {
    if let triet_syntax::Pattern::Variable(name) = &arena.pattern(catch_all.pattern).node {
        let bind_local = c.alloc_local_ty(scrut_ty.clone());
        c.push(Statement::StorageLive(bind_local, span.clone()));
        c.push(Statement::Assign {
            dest: Place::local(bind_local),
            source: Place::local(scrut_local),
            span: span.clone(),
        });
        c.vars.insert(name.clone(), bind_local);
    }
}

/// ADR-0064 §8 + §A1: lower a value-keyed `match` on an i64 scalar scrutinee
/// (`Integer`/`Tryte`/`Long`). All three share the i64 SwitchInt key
/// extraction; only the accepted literal suffix and the diagnostic name differ
/// — one helper instead of three near-identical branches. The domain is
/// infinite (Integer/Long) or impractically large (Tryte), so a missing
/// wildcard leaves the default block as a `Trap` (GAP-2 runtime enforcement;
/// compile-time exhaustiveness is in typecheck per §A1.2). `Long` keys beyond
/// i64 are rejected ("out of range"), inheriting the value-model i64 cap
/// (§A1.4 — bignum deferred).
fn lower_value_keyed_match(
    c: &mut Ctx,
    arena: &Arena,
    arms: &[triet_syntax::MatchArm],
    scrut_local: Local,
    scrut_ty: &MirType,
    expr_span: &Span,
    expected: Option<&MirType>, // ADR-0072 §2.2: forwarded to arm bodies (TRANSPARENT)
) -> Result<Local, LowerError> {
    use triet_syntax::{LiteralPattern, NumericSuffix, Pattern};

    let (expected_suffix, type_name): (Option<NumericSuffix>, &str) = match scrut_ty {
        MirType::Integer => (None, "Integer"),
        MirType::Tryte => (Some(NumericSuffix::Tryte), "Tryte"),
        MirType::Long => (Some(NumericSuffix::Long), "Long"),
        // Caller gates to these three; defend against drift.
        _ => {
            return Err(LowerError {
                message: "value-keyed match dispatched on a non-integer scalar".to_string(),
                span: expr_span.clone(),
            });
        }
    };

    let cur_bb = c.cur;
    let merge_bb = c.alloc_bb();
    let result = c.alloc_local();
    c.push(Statement::StorageLive(result, expr_span.clone()));

    let mut cases: Vec<(i64, BasicBlock)> = Vec::new();
    let mut wildcard_arm: Option<&triet_syntax::MatchArm> = None;

    for arm in arms.iter() {
        let pat = arena.pattern(arm.pattern);
        let pat_span = pat.span.clone();
        if wildcard_arm.is_some() {
            return Err(LowerError {
                message: format!(
                    "wildcard `_` must be the last arm in a {type_name} match — \
                     arms after wildcard are unreachable"
                ),
                span: pat_span,
            });
        }
        match &pat.node {
            Pattern::Wildcard => wildcard_arm = Some(arm),
            // ADR-0064 §8: a variable binding (`other =>`) is a catch-all —
            // bound to the scrutinee in the default block.
            Pattern::Variable(_) => wildcard_arm = Some(arm),
            Pattern::Literal(LiteralPattern::Integer { value, suffix })
                if *suffix == expected_suffix =>
            {
                let key = i64::try_from(*value).map_err(|_| LowerError {
                    message: format!("{type_name} literal value {value} out of range"),
                    span: pat_span.clone(),
                })?;
                let arm_bb = c.alloc_bb();
                cases.push((key, arm_bb));
                c.cur = arm_bb;
                c.push_scope();
                let body_val = lower_expr(arm.body, expected, arena, c)?;
                let arm_end = c.cur;
                // ADR-0056: type merge result from the arm value.
                c.local_decls[result.0].ty = c.local_decls[body_val.0].ty.clone();
                c.push(Statement::Assign {
                    dest: Place::local(result),
                    source: Place::local(body_val),
                    span: expr_span.clone(),
                });
                c.term(
                    arm_end,
                    Terminator::Goto {
                        target: merge_bb,
                        span: DUMMY_SPAN,
                    },
                );
                c.pop_scope();
            }
            other => {
                return Err(LowerError {
                    message: format!(
                        "unsupported pattern in {type_name} match: {other:?} — expected \
                         a {type_name} literal or `_`"
                    ),
                    span: pat_span,
                });
            }
        }
    }

    // GAP-2: default → wildcard body if present, else Trap.
    let default_bb = if let Some(wc) = wildcard_arm {
        let wc_bb = c.alloc_bb();
        c.cur = wc_bb;
        c.push_scope();
        bind_scalar_catch_all(c, arena, wc, scrut_local, scrut_ty, expr_span);
        let body_val = lower_expr(wc.body, expected, arena, c)?;
        let wc_end = c.cur;
        c.local_decls[result.0].ty = c.local_decls[body_val.0].ty.clone();
        c.push(Statement::Assign {
            dest: Place::local(result),
            source: Place::local(body_val),
            span: expr_span.clone(),
        });
        c.term(
            wc_end,
            Terminator::Goto {
                target: merge_bb,
                span: DUMMY_SPAN,
            },
        );
        c.pop_scope();
        wc_bb
    } else {
        let trap_bb = c.alloc_bb();
        c.cur = trap_bb;
        c.term(
            trap_bb,
            Terminator::Trap {
                span: expr_span.clone(),
            },
        );
        trap_bb
    };

    c.term(
        cur_bb,
        Terminator::SwitchInt {
            discriminant: scrut_local,
            cases,
            default_bb,
            span: expr_span.clone(),
        },
    );
    c.cur = merge_bb;
    Ok(result)
}

/// Map an AST binary operator to a MIR `BinOp`, or `None` for operators the
/// MIR `BinOp` set does not yet model.
fn lower_binop(op: &BinaryOperator) -> Option<BinOp> {
    Some(match op {
        BinaryOperator::Add => BinOp::Add,
        BinaryOperator::Sub => BinOp::Sub,
        BinaryOperator::Mul => BinOp::Mul,
        BinaryOperator::Div => BinOp::Div,
        BinaryOperator::Mod => BinOp::Mod,
        BinaryOperator::Eq => BinOp::Eq,
        BinaryOperator::Ne => BinOp::Ne,
        BinaryOperator::Lt => BinOp::Lt,
        BinaryOperator::Le => BinOp::Le,
        BinaryOperator::Gt => BinOp::Gt,
        BinaryOperator::Ge => BinOp::Ge,
        BinaryOperator::LukAnd => BinOp::LukAnd,
        BinaryOperator::LukOr => BinOp::LukOr,
        BinaryOperator::LukXor => BinOp::LukXor,
        BinaryOperator::LukImplies => BinOp::LukImplies,
        BinaryOperator::LukIff => BinOp::LukIff,
        BinaryOperator::KleeneImplies => BinOp::KleeneImplies,
        BinaryOperator::KleeneXor => BinOp::KleeneXor,
        BinaryOperator::KleeneIff => BinOp::KleeneIff,
        // Pow is handled inline in lower_expr via __triet_pow shim.
        BinaryOperator::Pow => return None,
    })
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Like `lower_source` but returns the `Result` so refusal paths can be
    /// asserted. Lowers the first function in `source` with empty tables.
    fn try_lower_first_fn(source: &str) -> Result<Body, LowerError> {
        let (prog, errors) = triet_parser::parse(source);
        assert!(errors.is_empty(), "parse errors: {errors:?}");
        let symbols: std::collections::HashMap<String, TypeKind> = prog
            .items
            .iter()
            .filter_map(|item| match &item.node {
                Item::Struct { def, .. } => Some((def.name.clone(), TypeKind::Struct)),
                Item::Enum { def, .. } => Some((def.name.clone(), TypeKind::Enum)),
                _ => None,
            })
            .collect();
        let func = prog
            .items
            .iter()
            .find_map(|item| match &item.node {
                Item::Function { def } => Some((def.clone(), item.span.clone())),
                _ => None,
            })
            .expect("no function found");
        let input = LoweringInput {
            arena: &prog.arena,
            symbols,
            struct_layouts: HashMap::new(),
            enum_layouts: HashMap::new(),
            pattern_resolutions: PatternResolutions::new(),
            method_resolutions: MethodResolutions::new(),
            func_return_types: HashMap::new(),
            capabilities: std::collections::HashMap::new(),
        };
        lower_function(&input, &func.0, func.1, None)
    }

    #[test]
    fn lambda_is_sealed_yagni() {
        // Phase 14.0: a first-class/escaping closure reaching the lowerer is
        // refused with the explicit YAGNI message, NOT the generic
        // unsupported_expr catch-all. Poison: remove the Expr::Lambda arm →
        // it falls to the generic catch-all → this assertion goes red.
        let err = try_lower_first_fn("function main() -> Integer { let f = |x| x; 0 }")
            .expect_err("lambda must be refused");
        assert!(
            err.message.contains("closure sealed (YAGNI"),
            "lambda must hit the explicit YAGNI seal, got: {}",
            err.message
        );
    }

    /// WO-NullableEnumAggregate-Refuse PA-A §4 (G mandate, 2026-07-18): the
    /// ONLY teeth guarding N3 (`resolve_aggregate_size`'s `Nullable(Enum)`
    /// arm reading `enum_map`, not `struct_map`). N1 (the declaration-time
    /// `nullable_enum_payload_unsupported` refuse in `lower_program`) blocks
    /// EVERY fixture-level path before codegen ever consumes this size, so
    /// no `.tri` fixture can observe N3 regress. If someone reverts
    /// `enum_map` back to `struct_map` at that arm, the size silently falls
    /// back to `fallback` (always a MISS for an enum name — an enum is
    /// never registered in `struct_map`) and the underlying overflow bug
    /// (WO evidence: `Mid{m:5,e:E::V(42)}` reads back `42` instead of `5`)
    /// returns dormant: uncaught by `cargo test --workspace`, uncaught by
    /// the fixture corpus, gate stays green. This test calls the private
    /// `resolve_aggregate_size` fn directly (same module) so N1 cannot
    /// intercept it.
    #[test]
    fn resolve_aggregate_size_nullable_enum_reads_enum_map_not_struct_map() {
        // A payload-bearing enum: disc@0 (8B) + payload@8 (8B Integer) = 16B.
        let enum_layout = EnumLayout::compute(
            "E",
            &[
                (
                    "V".to_string(),
                    0,
                    Some((MirType::Integer, 8, 8, Vec::new())),
                ),
                ("N".to_string(), 1, None),
            ],
        );
        assert_eq!(
            enum_layout.total_size, 16,
            "test setup sanity: expected a 16B payload-bearing enum"
        );
        let mut enum_map = HashMap::new();
        enum_map.insert("E".to_string(), enum_layout);
        // `struct_map` deliberately does NOT contain "E" — an enum is never
        // registered in struct_map (see the `MirType::Enum` arm's comment in
        // `resolve_aggregate_size`). This mirrors production reality: if the
        // Nullable(Enum) arm queries struct_map, it is GUARANTEED a MISS.
        let struct_map: HashMap<String, StructLayout> = HashMap::new();

        let ty = MirType::Nullable(Box::new(MirType::Enum("E".to_string())));
        let fallback = 8usize; // the old pre-fixpoint 8B seed, deliberately != 16
        let size = resolve_aggregate_size(&ty, &struct_map, &enum_map, fallback);

        assert_eq!(
            size, 16,
            "Nullable(Enum) must resolve its size from enum_map (16B, the \
             real payload-bearing size), not silently fall back to the 8B \
             seed via a bogus struct_map lookup — got {size}"
        );
    }

    fn lower_source(source: &str) -> Body {
        let (prog, errors) = triet_parser::parse(source);
        assert!(errors.is_empty(), "parse errors: {errors:?}");
        // Build ItemSymbolTable for lower_type/lower_type_simple.
        let symbols: std::collections::HashMap<String, TypeKind> = prog
            .items
            .iter()
            .filter_map(|item| match &item.node {
                Item::Struct { def, .. } => Some((def.name.clone(), TypeKind::Struct)),
                Item::Enum { def, .. } => Some((def.name.clone(), TypeKind::Enum)),
                _ => None,
            })
            .collect();
        let func = prog
            .items
            .iter()
            .find_map(|item| {
                if let Item::Function { def } = &item.node {
                    Some((def.clone(), item.span.clone()))
                } else {
                    None
                }
            })
            .expect("no function found");
        let input = LoweringInput {
            arena: &prog.arena,
            symbols,
            struct_layouts: HashMap::new(),
            enum_layouts: HashMap::new(),
            pattern_resolutions: PatternResolutions::new(),
            method_resolutions: MethodResolutions::new(),
            func_return_types: HashMap::new(),
            capabilities: std::collections::HashMap::new(),
        };
        lower_function(&input, &func.0, func.1, None).expect("lowering failed")
    }

    #[test]
    fn lowers_let_chain_and_prints_cfg() {
        let body = lower_source("function demo() -> Integer { let x = 1; let y = x + 2; }");
        println!("=== MIR ===\n{body}");
        let cfg = body.build_cfg();
        println!("=== CFG ===\n{cfg}");

        assert_eq!(body.blocks.len(), 1);
        assert!(matches!(
            body.blocks[0].terminator,
            Terminator::Return { .. }
        ));
        assert!(
            body.blocks[0]
                .statements
                .iter()
                .any(|s| matches!(s, Statement::BinaryOp { op: BinOp::Add, .. })),
            "expected a BinaryOp(Add) for `x + 2`"
        );
        // LocalDecls are tracked per local.
        assert_eq!(body.local_decls.len(), body.num_locals);
    }

    #[test]
    fn lowers_if_into_branching_cfg() {
        let body = lower_source(
            "function pick(n: Integer) -> Integer { if n > 0 { return 1; }; return 2; }",
        );
        println!("=== MIR ===\n{body}");
        let cfg = body.build_cfg();
        assert!(cfg.blocks.len() > 1);
        assert!(
            body.blocks
                .iter()
                .any(|b| matches!(b.terminator, Terminator::If { .. })),
            "expected an If terminator"
        );
        // The single `Point`-less function's param gets a typed LocalDecl.
        assert_eq!(body.local_decls[0].ty, MirType::Integer);
    }

    /// HP.1: heap Outcome producer emits OutcomePayloadLen + OutcomePayloadCap
    /// for String payload (3-field decomposition into 32-byte slot).
    #[test]
    fn heap_outcome_producer_emits_len_cap_projections() {
        let body = lower_source("function greet() -> String~Integer { ~+ \"hello\" }");
        println!("=== MIR (heap prod) ===\n{body}");
        let has_payload_len = body.blocks.iter().any(|b| {
            b.statements.iter().any(|s| {
                if let Statement::Assign { dest, .. } = s {
                    dest.projection
                        .iter()
                        .any(|p| matches!(p, Projection::OutcomePayloadLen))
                } else {
                    false
                }
            })
        });
        assert!(
            has_payload_len,
            "heap Outcome MUST emit OutcomePayloadLen projection"
        );
        let has_payload_cap = body.blocks.iter().any(|b| {
            b.statements.iter().any(|s| {
                if let Statement::Assign { dest, .. } = s {
                    dest.projection
                        .iter()
                        .any(|p| matches!(p, Projection::OutcomePayloadCap))
                } else {
                    false
                }
            })
        });
        assert!(
            has_payload_cap,
            "heap Outcome MUST emit OutcomePayloadCap projection"
        );
    }

    /// HP.1: scalar Outcome does NOT emit heap projections (no regress).
    #[test]
    fn scalar_outcome_producer_no_heap_projections() {
        let body = lower_source("function ok() -> Integer~Integer { ~+ 42 }");
        println!("=== MIR (scalar prod) ===\n{body}");
        let has_len = body.blocks.iter().any(|b| {
            b.statements.iter().any(|s| {
                if let Statement::Assign { dest, .. } = s {
                    dest.projection
                        .iter()
                        .any(|p| matches!(p, Projection::OutcomePayloadLen))
                } else {
                    false
                }
            })
        });
        assert!(!has_len, "scalar Outcome MUST NOT emit OutcomePayloadLen");
    }

    /// HP.4: within every block, no local read by an `Assign` source may have
    /// been `Drop`ped earlier in that block (Drop-then-move = use-after-free).
    /// This is the structural witness for the F1 fix (ADR-0053 §9.3).
    fn assert_no_drop_then_move(body: &Body) {
        for b in &body.blocks {
            let mut dropped: std::collections::HashSet<Local> = std::collections::HashSet::new();
            for s in &b.statements {
                if let Statement::Assign { source, .. } = s {
                    assert!(
                        !dropped.contains(&source.local),
                        "F1 race: local {} moved after Drop (use-after-free)",
                        source.local
                    );
                }
                if let Statement::Drop(l, _) = s {
                    dropped.insert(*l);
                }
            }
        }
    }

    /// HP.4 F1: heap `~+>` map (identity `|v| v`) must move the captured heap
    /// value into the result BEFORE the scope-pop Drop, and tombstone it via
    /// Deinit so the Drop is a no-op. Asserts (a) ≥2 Deinit (inner + moved
    /// value) and (b) no Drop-then-move race.
    /// Teeth: revert the pop_scope reorder (pop before result-write) → the
    /// captured local is Dropped, then read by the result Assign → RED.
    #[test]
    fn map_heap_success_no_drop_then_move() {
        let body = lower_source(
            "function demo() -> String~Integer { let o: String~Integer = ~+ \"hi\"; o ~+> |v| v }",
        );
        println!("=== MIR (map heap ~+>) ===\n{body}");
        let deinit_count = body
            .blocks
            .iter()
            .flat_map(|b| &b.statements)
            .filter(|s| matches!(s, Statement::Deinit(..)))
            .count();
        assert!(
            deinit_count >= 2,
            "heap map MUST Deinit inner + moved value (got {deinit_count})"
        );
        assert_no_drop_then_move(&body);
    }

    /// HP.4 F1: heap `~->` map (error transformer, identity `|e| e`) — same
    /// invariant on the error arm. Success type is scalar (passthrough).
    #[test]
    fn map_heap_error_no_drop_then_move() {
        let body = lower_source(
            "function demo() -> Integer~String { let o: Integer~String = ~- \"err\"; o ~-> |e| e }",
        );
        println!("=== MIR (map heap ~->) ===\n{body}");
        let deinit_count = body
            .blocks
            .iter()
            .flat_map(|b| &b.statements)
            .filter(|s| matches!(s, Statement::Deinit(..)))
            .count();
        assert!(
            deinit_count >= 2,
            "heap error map MUST Deinit inner + moved value (got {deinit_count})"
        );
        assert_no_drop_then_move(&body);
    }

    /// M2: `let b = a` with Move-type local (String) must emit an Assign
    /// and create a new local, not alias the same Local.
    #[test]
    fn let_move_type_emits_assign_not_alias() {
        let body = lower_source("function f() -> Integer { let s = \"hi\"; let b = s; return 1; }");
        println!("=== MIR (M2) ===\n{body}");
        // Must have an Assign from old String local to new local (M2).
        let assigns: Vec<_> = body.blocks[0]
            .statements
            .iter()
            .filter_map(|s| match s {
                Statement::Assign { dest, source, .. } => Some((dest.local, source.local)),
                _ => None,
            })
            .collect();
        assert!(!assigns.is_empty(), "M2: let-Move-type must emit Assign");
        // Source and dest must be different locals (not aliased).
        for (dest, src) in &assigns {
            assert_ne!(dest, src, "M2: source and dest must be different locals");
        }
    }

    /// 4i-4: Call destination for user-function returning a Move type
    /// HP.3: match heap Outcome bind → MIR must contain Deinit(scrut).
    /// Teeth: poison lowerer 2884 (if false on Deinit) → test RED.
    #[test]
    fn match_heap_bind_emits_deinit() {
        let body = lower_source(
            "function consume() -> Integer { let o: String~Integer = ~+ \"hi\"; match o { ~+ x => 0  ~- e => 1 } }",
        );
        println!("=== MIR (match_deinit) ===\n{body}");
        let has_deinit = body.blocks.iter().any(|b| {
            b.statements
                .iter()
                .any(|s| matches!(s, Statement::Deinit(..)))
        });
        assert!(
            has_deinit,
            "match bind heap Outcome MUST emit Deinit(scrut) to tombstone disc=0"
        );
    }

    /// must have the correct type (e.g. "String"), not "?".
    /// Teeth: revert call-dest fix → type is "?" → test RED.
    #[test]
    fn call_dest_has_correct_type_for_heap_return() {
        let source = "
function make() -> String {
    let s = \"hi\";
    return s;
}
function main() -> Integer {
    let t = make();
    return 1;
}
";
        let (prog, errors) = triet_parser::parse(source);
        assert!(errors.is_empty(), "parse errors: {errors:?}");

        let bodies = lower_program(&prog, &PatternResolutions::new(), &MethodResolutions::new())
            .expect("lowering");

        // Find main's body.
        let main_body = bodies
            .iter()
            .find(|b| b.signature.name == "main")
            .expect("main not found");

        println!("=== MAIN MIR ===\n{main_body}");

        // Find the CallDispatch terminator and get its dest local.
        let dest_locals: Vec<_> = main_body
            .blocks
            .iter()
            .filter_map(|b| match &b.terminator {
                Terminator::CallDispatch { dest, .. } => Some(dest.clone()),
                _ => None,
            })
            .collect();

        assert!(!dest_locals.is_empty(), "main must have a CallDispatch");
        // ADR-0049 L6: sret calls have empty dest (return via sret buffer).
        // Find first CallDispatch with a non-empty dest.
        let dest = dest_locals
            .iter()
            .find(|d| !d.is_empty())
            .map(|d| d[0])
            .or_else(|| {
                // All dests are empty (sret) — use the sret local from args.
                main_body.blocks.iter().find_map(|b| match &b.terminator {
                    Terminator::CallDispatch { args, .. } if !args.is_empty() => Some(args[0]),
                    _ => None,
                })
            })
            .expect("must have a dest or sret arg");

        // Check the type of the dest local.
        let dest_ty = &main_body.local_decls[dest.0].ty;
        assert_eq!(
            *dest_ty,
            MirType::String,
            "4i-4: call-dest for make()->String must have type String, got `{dest_ty}` \
             (call-dest fix regressed — using alloc_local() instead of alloc_local_ty(&callee_ret))"
        );
    }

    #[test]
    fn lowers_field_borrow_into_projected_place() {
        // `&0 obj.x` must lower to a Borrow whose source is the projected
        // Place { local: _0 (obj), projection: [Field("x")] }.
        let body = lower_source(
            "struct Point { x: Integer } function f(obj: Point) -> Integer { let r = &0 obj.x; return r; }",
        );
        println!("=== MIR ===\n{body}");

        let borrow_source = body
            .blocks
            .iter()
            .flat_map(|b| &b.statements)
            .find_map(|s| match s {
                Statement::Borrow { source, .. } => Some(source.clone()),
                _ => None,
            })
            .expect("expected a Borrow statement");

        assert_eq!(
            borrow_source.local,
            Local(0),
            "base local should be obj (_0)"
        );
        assert_eq!(
            borrow_source.projection,
            vec![Projection::Field("x".to_string())],
            "borrow source must carry a Field(\"x\") projection, not the whole struct"
        );
    }

    // ── Phase 4.3b: Vector builtin dispatch ──

    #[test]
    fn push_emits_vector_push_shim() {
        let body = lower_source("function main() { let v = vector_new(); let v2 = push(v, 1); }");
        println!("=== MIR ===\n{body}");
        let has_push = body.blocks.iter().any(|b| {
            matches!(
                &b.terminator,
                Terminator::CallDispatch {
                    callee_name,
                    target: CallTarget::Shim,
                    ..
                } if callee_name == "__triet_vector_push"
            )
        });
        assert!(has_push, "expected __triet_vector_push CallDispatch in MIR");
    }

    #[test]
    fn len_dispatches_by_arg_type() {
        // ADR-0049: len(owned String) → field-read "len" from slot, not shim.
        let body_str = lower_source(r#"function main() { let x = len("hello") }"#);
        println!("=== MIR string_len ===\n{body_str}");
        let has_field_read = body_str.blocks.iter().any(|b| {
            b.statements.iter().any(|s| {
                matches!(s,
                    Statement::Assign {
                        source: Place {
                            projection,
                            ..
                        },
                        ..
                    } if projection == &vec![triet_mir::Projection::Field("len".into())]
                )
            })
        });
        assert!(
            has_field_read,
            "len(owned String) should emit field-read `_0.len`, not shim"
        );

        // len(vector) → __triet_vector_len
        let body_vec = lower_source("function main() { let v = vector_new(); let x = len(v) }");
        println!("=== MIR vector_len ===\n{body_vec}");
        let has_vec_len = body_vec.blocks.iter().any(|b| {
            matches!(
                &b.terminator,
                Terminator::CallDispatch {
                    callee_name,
                    target: CallTarget::Shim,
                    ..
                } if callee_name == "__triet_vector_len"
            )
        });
        assert!(
            has_vec_len,
            "len(Vector) should dispatch to __triet_vector_len"
        );
    }

    // ── Nullable lowering tests (ADR-0041 Bước 3) ──────────────

    /// N3: `~0` without expected type → `Err(LowerError)` with span, not panic.
    #[test]
    fn null_literal_without_expected_type_is_error() {
        // Positive: `let x: Integer? = ~0` has expected type → must succeed.
        let body = lower_source("function test() { let x: Integer? = ~0; }");
        assert!(
            body.local_decls
                .iter()
                .any(|d| d.ty == MirType::Nullable(Box::new(MirType::Integer))),
            "~0 with annotation should produce Integer? local"
        );

        // Negative: `~0` as a standalone expression statement — no expected type.
        let result = std::panic::catch_unwind(|| {
            lower_source("function test() { ~0; }");
        });
        match result {
            Err(panic) => {
                let msg = panic
                    .downcast_ref::<String>()
                    .map(|s| s.as_str())
                    .or_else(|| panic.downcast_ref::<&str>().copied())
                    .unwrap_or("unknown");
                assert!(
                    msg.contains("requires an expected type"),
                    "error should mention expected type, got: {msg}"
                );
            }
            Ok(_body) => {
                panic!("~0 without expected type should have failed to lower");
            }
        }
    }

    /// B2 (§2 callee): no Drop for reference-type borrow parameters.
    /// Callee receives a reference handle — it does not own the heap.
    #[test]
    fn borrow_param_no_drop_in_callee() {
        let body = lower_source(
            "function peek(s: &0 String) -> Integer { return 0; } \
             function main() -> Integer { return 0; }",
        );
        let has_drop = body
            .blocks
            .iter()
            .flat_map(|b| &b.statements)
            .any(|stmt| matches!(stmt, Statement::Drop(local, _) if local.0 == 0));
        assert!(!has_drop, "B2 regression: borrow param must NOT have Drop.");
    }
}

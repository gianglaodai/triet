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
    Arena, BinaryOperator, Expr, ExprId, ExprResolutions, FunctionBody, FunctionDefinition, Item,
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

    fn null_literal_without_expected_type(span: Span) -> Self {
        Self {
            message: "`~0` (null literal) requires an expected type from context \
                 (e.g. `let x: Integer? = ~0` or `return ~0` from a function returning `T?`). \
                 Standalone `~0` without a type annotation is not supported in Bậc A."
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
    /// Resolved enum variants from the type checker, keyed by expression ID.
    pub expr_resolutions: ExprResolutions,
    /// Resolved enum variants from the type checker, keyed by pattern ID.
    pub pattern_resolutions: PatternResolutions,
    /// ADR-0061 T5: resolved trait-method calls (ExprId → mangled fn).
    pub method_resolutions: MethodResolutions,
    /// Map from (possibly mangled) function name to its return type.
    pub func_return_types: HashMap<String, MirType>,
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
    /// Resolved enum variants from the type checker, keyed by expression ID.
    /// The lowerer reads these instead of scanning enum_layouts by string match.
    expr_resolutions: ExprResolutions,
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
    fn new(name: &str, ret: &MirType, input: &LoweringInput) -> Self {
        let is_struct_return = matches!(ret, MirType::Struct(_));
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
        let is_fat_return = is_struct_return || ret.is_string_repr() || is_heap_outcome;
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
            expr_resolutions: input.expr_resolutions.clone(),
            pattern_resolutions: input.pattern_resolutions.clone(),
            method_resolutions: input.method_resolutions.clone(),
            func_return_types: input.func_return_types.clone(),
            sret_ptr: None,
            owned_locals: Vec::new(),
            scope_snapshots: Vec::new(),
            local_names: BTreeMap::new(),
        };
        if is_fat_return {
            ctx.sret_ptr = Some(ctx.alloc_local_ty(ret));
        }
        ctx
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
    expr_resolutions: &ExprResolutions,
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
                        (f.name.clone(), ty, 8, 8)
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

    // ADR-0060 P2: fixup pass — recompute layouts with correct sizes
    // for aggregate-typed fields (struct/enum). First pass hardcoded
    // size=8 for all fields; now replace with the nested struct's
    // total_size from struct_map. Iterate until stable (handles
    // A→B→C nesting without topological ordering).
    loop {
        let mut changed = false;
        let mut new_layouts: Vec<StructLayout> = Vec::with_capacity(struct_layouts.len());
        for layout in &struct_layouts {
            let new_fields: Vec<(String, MirType, usize, usize)> = layout
                .fields
                .iter()
                .map(|f| {
                    let size = match &f.ty {
                        MirType::Struct(name) | MirType::Enum(name) => struct_map
                            .get(name.as_str())
                            .map(|l| l.total_size)
                            .unwrap_or(f.size),
                        _ => f.size,
                    };
                    (f.name.clone(), f.ty.clone(), size, f.alignment)
                })
                .collect();
            let new_layout = StructLayout::compute(&layout.name, &new_fields);
            if new_layout.total_size != layout.total_size {
                changed = true;
            }
            new_layouts.push(new_layout);
        }
        struct_layouts = new_layouts;
        for l in &struct_layouts {
            struct_map.insert(l.name.clone(), l.clone());
        }
        if !changed {
            break;
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

    // ── Collect enum layouts from enum definitions ──────────────
    // Bậc A: every payload is 8 bytes (i64), alignment 8.
    // Unit variants have no payload (size 0).
    let enum_layouts: Vec<EnumLayout> = prog
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
                            // Bậc A: every type is 8-byte i64
                            (ty, 8usize, 8usize, Vec::new())
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

    let enum_map: HashMap<String, EnumLayout> = enum_layouts
        .iter()
        .map(|l| (l.name.clone(), l.clone()))
        .collect();

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
    let input = LoweringInput {
        arena: &prog.arena,
        symbols,
        struct_layouts: struct_map,
        enum_layouts: enum_map,
        expr_resolutions: expr_resolutions.clone(),
        pattern_resolutions: pattern_resolutions.clone(),
        method_resolutions: method_resolutions.clone(),
        func_return_types,
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
    let mut c = Ctx::new(body_name, &ret_ty, input);
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
        // ADR-0055 (A-hẹp): mirror the Stmt::Return null-~0 special case for
        // expr-body tail. `= ~0` materializes the PA-3c sentinel from the
        // function return type, identical to `return ~0` (the Stmt::Return
        // null branch above). Block-final / if-arm `~0` is a SEPARATE
        // expected-type-propagation gap (Heap-Nullable backlog) — NOT here.
        let val = if is_null_expr(&input.arena.expression(e).node)
            && !matches!(c.sig.return_type, MirType::Outcome { .. })
        {
            let span = input.arena.expression(e).span.clone();
            let d = c.alloc_local_ty(c.sig.return_type.clone());
            c.push(Statement::StorageLive(d, span.clone()));
            c.push(Statement::Const {
                dest: Place::local(d),
                value: ConstValue::Integer(i128::from(triet_mir::NULL_SENTINEL)),
                span,
            });
            d
        } else {
            lower_expr(e, input.arena, &mut c)?
        };
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
            other if other == "Vector" || other.starts_with("Vector<") => MirType::Vector,
            other if other == "HashMap" || other.starts_with("HashMap<") => MirType::HashMap,
            other if symbols.get(other) == Some(&TypeKind::Struct) => {
                MirType::Struct(other.to_string())
            }
            other if symbols.get(other) == Some(&TypeKind::Enum) => {
                MirType::Enum(other.to_string())
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
        TypeExpr::Generic { name, .. } => match name.as_str() {
            "Vector" => MirType::Vector,
            "HashMap" => MirType::HashMap,
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
            other if other == "Vector" || other.starts_with("Vector<") => MirType::Vector,
            other if other == "HashMap" || other.starts_with("HashMap<") => MirType::HashMap,
            other if c.struct_layouts.contains_key(other) => MirType::Struct(other.to_string()),
            other if c.enum_layouts.contains_key(other) => MirType::Enum(other.to_string()),
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
        TypeExpr::Generic { name, .. } => match name.as_str() {
            "Vector" => MirType::Vector,
            "HashMap" => MirType::HashMap,
            _ => MirType::Unknown,
        },
        _ => MirType::Unknown,
    }
}

/// Emit a `CallDispatch` terminator targeting a builtin shim, allocate a
/// return local of `dest_ty`, and advance `c.cur` to the return block.
/// Returns the destination local holding the shim's return value.
fn emit_shim_call(
    c: &mut Ctx,
    shim_name: &str,
    args: Vec<Local>,
    dest_ty: impl Into<MirType>,
    span: Span,
) -> Local {
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
                lower_expr(*e, arena, c)?;
            }
        }
        _ => {
            lower_expr(block_expr, arena, c)?;
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
                let v = lower_expr(*init, arena, c)?;
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
                    if ann_ty != MirType::Unknown {
                        c.local_decls[v.0].ty = ann_ty;
                    }
                }
                // M2: If init is an Identifier of a Move type, emit Assign + new local
                // instead of aliasing. This creates a genuine move-site so JIT's
                // Zeroing-on-Move (M1) can zero the source variable.
                let is_move_binding =
                    if let Expr::Identifier { name: _ } = &arena.expression(*init).node {
                        let ty = &c.local_decls[v.0].ty;
                        !ty.is_copy(None)
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
            lower_expr(*expr, arena, c)?;
        }
        Stmt::Return { value } => {
            if let (Some(_), Some(v)) = (c.sret_ptr, value) {
                // Fat return via sret. Struct → copy fields into the caller's
                // buffer (emit_struct_sret_copy); String/heap-Outcome → M4
                // escape Return[s] (JIT writes {ptr,len,cap} from slot to sret).
                let struct_local = lower_expr(*v, arena, c)?;
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
                    // ── NullLiteral ~0 in return position ──
                    // Under PA-3c uniform, ~0 is always iconst(MIN).
                    // The function's return type provides the local's type.
                    // ADR-0052: if the return type is an Outcome (ternary T?~E),
                    // let OutcomeConstructor handle ~0 as 2-slot {disc:0, payload}.
                    if is_null_expr(&arena.expression(*v).node)
                        && !matches!(c.sig.return_type, MirType::Outcome { .. })
                    {
                        let ret_ty = c.sig.return_type.clone();
                        let d = c.alloc_local_ty(ret_ty.clone());
                        c.push(Statement::StorageLive(d, stmt_span.clone()));
                        c.push(Statement::Const {
                            dest: Place::local(d),
                            value: ConstValue::Integer(i128::from(triet_mir::NULL_SENTINEL)),
                            span: stmt_span.clone(),
                        });
                        values.push(d);
                    } else {
                        let val = lower_expr(*v, arena, c)?;
                        values.extend(lower_outcome_return_values(val, c));
                    }
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
            let v = lower_expr(*value, arena, c)?;
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
            let cond = lower_expr(*condition, arena, c)?;
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
            Ok(base.project(Projection::Field(field.clone())))
        }
        _ => {
            let temp = lower_expr(expr_id, arena, c)?;
            Ok(Place::local(temp))
        }
    }
}

// ── Expression lowering ─────────────────────────────────────

fn lower_expr(expr_id: ExprId, arena: &Arena, c: &mut Ctx) -> Result<Local, LowerError> {
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
            // null keyword (deprecated) — same as ~0.
            Err(LowerError::null_literal_without_expected_type(expr_span))
        }
        Expr::OutcomeConstructor { arm, payload } => {
            // ── ADR-0052 §3.2 + OP.3.5: StackSlot 16-byte constructor ──
            // Outcome = 1 local → StackSlot {disc@0, payload@8}.
            // disc encoding: Positive=1, Negative=−1 (Trit).
            // Zero arm (T?~E) deferred to later OP.
            // Payload = value/error scalar (Bậc A — heap deferred).

            // ~0 (Zero arm) — constructor for T?~E ternary Outcome.
            // Has no payload — disc = Trit(0), payload = 0 (don't-care).
            if matches!(arm, triet_syntax::OutcomeArm::Zero) {
                if !matches!(c.sig.return_type, MirType::Outcome { .. }) {
                    // ~0 in non-Outcome context → same as NullLiteral.
                    return Err(LowerError::null_literal_without_expected_type(expr_span));
                }
                let outcome_ty = c.sig.return_type.clone();
                let outcome = c.alloc_local_ty(outcome_ty);
                c.push(Statement::StorageLive(outcome, expr_span.clone()));
                c.push(Statement::OutcomeAlloc {
                    dest: outcome,
                    span: expr_span.clone(),
                });
                // disc = Trit(0).
                let disc_tmp = c.alloc_local_ty(MirType::Trit);
                c.push(Statement::StorageLive(disc_tmp, expr_span.clone()));
                c.push(Statement::Const {
                    dest: Place::local(disc_tmp),
                    value: ConstValue::Trit(0),
                    span: expr_span.clone(),
                });
                c.push(Statement::Assign {
                    dest: Place::local(outcome).project(Projection::OutcomeDiscriminant),
                    source: Place::local(disc_tmp),
                    span: expr_span.clone(),
                });
                // payload = 0 (don't-care — ~0 has no associated data).
                let payload_tmp = c.alloc_local_ty(MirType::Integer);
                c.push(Statement::StorageLive(payload_tmp, expr_span.clone()));
                c.push(Statement::Const {
                    dest: Place::local(payload_tmp),
                    value: ConstValue::Integer(0),
                    span: expr_span.clone(),
                });
                c.push(Statement::Assign {
                    dest: Place::local(outcome).project(Projection::OutcomePayload),
                    source: Place::local(payload_tmp),
                    span: expr_span.clone(),
                });
                return Ok(outcome);
            }

            let payload_ty = if let MirType::Outcome {
                ref value_type,
                ref error_type,
                ..
            } = c.sig.return_type
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
            let outcome_ty = c.sig.return_type.clone();
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
                let val = lower_expr(*payload_expr, arena, c)?;
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
            // Check if this is a local variable first.
            if let Some(&local) = c.vars.get(name) {
                return Ok(local);
            }
            // Check the type checker's resolution map for unit enum variants.
            // Resolution is done by the type checker — the lowerer just reads
            // the map and emits MIR. No string-scanning of enum_layouts.
            // Clone to release the immutable borrow before mutating ctx.
            let resolution = c.expr_resolutions.get(&expr_id).cloned();
            if let Some(resolution) = resolution
                && !resolution.has_payload
            {
                let d = c.alloc_local_ty(MirType::Enum(resolution.enum_name.clone()));
                c.push(Statement::StorageLive(d, expr_span.clone()));
                c.push(Statement::EnumAlloc {
                    dest: d,
                    enum_name: resolution.enum_name.clone(),
                    span: expr_span.clone(),
                });
                c.push(Statement::SetDiscriminant {
                    dest: d,
                    value: resolution.discriminant,
                    span: expr_span.clone(),
                });
                return Ok(d);
            }
            Err(LowerError::undefined_local(name, expr_span))
        }
        Expr::UnaryOp { operator, operand } => {
            let val = lower_expr(*operand, arena, c)?;
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
            let lhs = lower_expr(*left, arena, c)?;
            let rhs = lower_expr(*right, arena, c)?;
            let d = c.alloc_local();

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
            // Detect enum literal via direct variant call: `Variant(args)`.
            // The type checker resolves these via resolve_enum_variant.
            // Resolution data is stored keyed by call expression ID.
            // Clone to release the immutable borrow before mutating ctx.
            let resolution = c.expr_resolutions.get(&expr_id).cloned();
            if let Some(resolution) = resolution {
                if resolution.has_payload {
                    if arguments.len() != 1 {
                        return Err(LowerError {
                            message: format!(
                                "enum variant '{}' expects 1 argument, got {}",
                                resolution.variant_name,
                                arguments.len()
                            ),
                            span: expr_span.clone(),
                        });
                    }
                    let d = c.alloc_local_ty(MirType::Enum(resolution.enum_name.clone()));
                    c.push(Statement::StorageLive(d, expr_span.clone()));
                    c.push(Statement::EnumAlloc {
                        dest: d,
                        enum_name: resolution.enum_name.clone(),
                        span: expr_span.clone(),
                    });
                    c.push(Statement::SetDiscriminant {
                        dest: d,
                        value: resolution.discriminant,
                        span: expr_span.clone(),
                    });
                    let val = lower_expr(arguments[0], arena, c)?;
                    // B8: aggregate-containing-heap — enum payload with Move type not supported.
                    let payload_ty = &c.local_decls[val.0].ty;
                    if !payload_ty.is_copy(None) {
                        return Err(LowerError::heap_type_not_supported(
                            &format!(
                                "enum variant `{}.{}` payload type `{}`",
                                resolution.enum_name, resolution.variant_name, payload_ty
                            ),
                            expr_span,
                        ));
                    }
                    c.push(Statement::Assign {
                        dest: Place::local(d)
                            .project(Projection::Payload(resolution.variant_name.clone())),
                        source: Place::local(val),
                        span: expr_span.clone(),
                    });
                    return Ok(d);
                }
                // Unit variant with args — error.
                if !arguments.is_empty() {
                    return Err(LowerError {
                        message: format!(
                            "unit variant '{}' does not take arguments",
                            resolution.variant_name
                        ),
                        span: expr_span.clone(),
                    });
                }
            }

            // Detect enum literal: `TypeName.Variant(payload)` parses as
            // Call(FieldAccess(Identifier(TypeName), Variant), [payload]).
            if let Expr::FieldAccess { object, field } = &arena.expression(*callee).node
                && let Expr::Identifier { name: enum_name } = &arena.expression(*object).node
                && c.enum_layouts.contains_key(enum_name)
                && arguments.len() == 1
            {
                // Lower as enum literal with payload.
                let d = c.alloc_local_ty(MirType::Enum(enum_name.to_string()));
                c.push(Statement::StorageLive(d, expr_span.clone()));
                c.push(Statement::EnumAlloc {
                    dest: d,
                    enum_name: enum_name.clone(),
                    span: expr_span.clone(),
                });
                let disc = {
                    let layout = c.enum_layouts.get(enum_name).ok_or_else(|| LowerError {
                        message: format!("unknown enum '{enum_name}'"),
                        span: expr_span.clone(),
                    })?;
                    layout
                        .variants
                        .iter()
                        .find(|v| v.name == *field)
                        .map(|v| v.discriminant_value)
                        .ok_or_else(|| LowerError {
                            message: format!("unknown variant '{field}' in enum '{enum_name}'"),
                            span: expr_span.clone(),
                        })?
                };
                c.push(Statement::SetDiscriminant {
                    dest: d,
                    value: disc,
                    span: expr_span.clone(),
                });
                let val = lower_expr(arguments[0], arena, c)?;
                c.push(Statement::Assign {
                    dest: Place::local(d).project(Projection::Payload(field.clone())),
                    source: Place::local(val),
                    span: expr_span.clone(),
                });
                return Ok(d);
            }

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
                        .map(|a| lower_expr(*a, arena, c))
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
                        .map(|a| lower_expr(*a, arena, c))
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
                    let arg = lower_expr(arguments[0], arena, c)?;
                    let arg_ty = &c.local_decls[arg.0].ty;
                    // ADR-0049 Phase-1 Lát 1 B4: length on owned String reads
                    // field-1 (len) from the StackSlot, not from the heap shim.
                    // Borrow (&0 String etc.) keeps the shim — the handle still
                    // points to the heap where len@body+0.
                    if matches!(arg_ty, MirType::String) {
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
                        .map(|a| lower_expr(*a, arena, c))
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
                    let arg = lower_expr(arguments[0], arena, c)?;
                    let arg_ty = &c.local_decls[arg.0].ty;

                    // ADR-0049 Lát 6.3: for owned String, read len from
                    // StackSlot (field projection), not heap shim.
                    // Same pattern as `length`/`len`.
                    let len_dest = if matches!(arg_ty, MirType::String) {
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
                    let arg = lower_expr(arguments[0], arena, c)?;
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
                    let arg = lower_expr(arguments[0], arena, c)?;
                    let byte_arg = lower_expr(arguments[1], arena, c)?;
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
                    // Type-aware dispatch: Vector → __triet_vector_get,
                    // HashMap → __triet_hashmap_get.
                    if arguments.len() != 2 {
                        return Err(LowerError::unsupported_expr(
                            &arena.expression(*callee).node,
                            expr_span,
                        ));
                    }
                    let args: Vec<Local> = arguments
                        .iter()
                        .map(|a| lower_expr(*a, arena, c))
                        .collect::<Result<Vec<_>, _>>()?;
                    let arg0_ty = &c.local_decls[args[0].0].ty;
                    let base_ty = if let MirType::Reference { inner, .. } = arg0_ty {
                        inner.as_ref()
                    } else {
                        arg0_ty
                    };
                    let shim_name = if base_ty.is_hashmap() {
                        "__triet_hashmap_get"
                    } else if base_ty.is_vec() {
                        "__triet_vector_get"
                    } else {
                        return Err(LowerError::heap_type_not_supported(
                            &format!("get() on type `{arg0_ty}` — expected Vector or HashMap"),
                            expr_span,
                        ));
                    };
                    let dest = emit_shim_call(
                        c,
                        shim_name,
                        args,
                        MirType::Nullable(Box::new(MirType::Integer)),
                        expr_span,
                    );
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
                        .map(|a| lower_expr(*a, arena, c))
                        .collect::<Result<Vec<_>, _>>()?;
                    let vec_ty = c.local_decls[args[0].0].ty.clone();
                    let dest = emit_shim_call(c, "__triet_vector_push", args, &vec_ty, expr_span);
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
                    let dest = emit_shim_call(
                        c,
                        "__triet_vector_alloc",
                        vec![len_local, cap_local],
                        MirType::Vector,
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
                        MirType::HashMap,
                        expr_span,
                    );
                    return Ok(dest);
                }
                "insert" => {
                    let args: Vec<Local> = arguments
                        .iter()
                        .map(|a| lower_expr(*a, arena, c))
                        .collect::<Result<Vec<_>, _>>()?;
                    let map_ty = c.local_decls[args[0].0].ty.clone();
                    let dest =
                        emit_shim_call(c, "__triet_hashmap_insert", args, &map_ty, expr_span);
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
            // ADR-0062: `String?` call return shares String's fat sret path.
            let is_fat_ret = matches!(callee_ret, MirType::Struct(_))
                || callee_ret.is_string_repr()
                || is_heap_outcome_ret;
            // sret slot layout name: `String?` reprs as the "String" layout
            // (ptr-sentinel, same 24-byte slot) — `callee_ret.to_string()`
            // would be "String?", which has no registered layout.
            let sret_layout_name = if callee_ret.is_string_repr() {
                "String".to_string()
            } else {
                callee_ret.to_string()
            };

            let mut args: Vec<Local> = arguments
                .iter()
                .map(|a| lower_expr(*a, arena, c))
                .collect::<Result<Vec<_>, _>>()?;

            // B7-lift (ADR-0042): heap args now allowed.
            // Move semantics: caller zeroes slot after call, borrowck
            // enforces E2420 use-after-move.
            if is_fat_ret {
                // sret: allocate struct/string/heap-outcome local for return,
                // pass as hidden arg[0].
                let ret_local = c.alloc_local_ty(callee_ret.clone());
                c.push(Statement::StorageLive(ret_local, expr_span.clone()));
                if is_heap_outcome_ret {
                    // ADR-0058 Lát 1: heap Outcome sret — OutcomeAlloc
                    // (JIT no-op; required for verifier/borrowck).
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
                        !ty.is_copy(None)
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
                        !ty.is_copy(None)
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
                        !ty.is_copy(None)
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
                    let tail_val = lower_expr(e, arena, c)?;
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
            let cond = lower_expr(*condition, arena, c)?;
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
            let then_val = lower_expr(then_branch, arena, c)?;
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
                let else_val = lower_expr(eb, arena, c)?;
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
            let obj_val = lower_expr(*object, arena, c)?;

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
            let default_val = lower_expr(*default, arena, c)?;
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
            let inner_val = lower_expr(*inner, arena, c)?;
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
            let body_val = lower_expr(*body, arena, c)?;
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
        // Also handles `Type.variant` enum literals — if the base is a known
        // enum type, route to EnumLiteral lowering.
        Expr::FieldAccess { .. } => {
            // Check the type checker's resolution map first.
            // Qualified enum variants (`Color.Red`, `CD.None`) are
            // resolved by the type checker via check_field_access.
            let resolution = c.expr_resolutions.get(&expr_id).cloned();
            if let Some(res) = resolution
                && !res.has_payload
            {
                // Unit variant constructor: `TypeName.Variant`
                let d = c.alloc_local_ty(MirType::Enum(res.enum_name.clone()));
                c.push(Statement::StorageLive(d, expr_span.clone()));
                c.push(Statement::EnumAlloc {
                    dest: d,
                    enum_name: res.enum_name.clone(),
                    span: expr_span.clone(),
                });
                c.push(Statement::SetDiscriminant {
                    dest: d,
                    value: res.discriminant,
                    span: expr_span.clone(),
                });
                return Ok(d);
            }
            // Payload variant without call syntax — shouldn't happen
            // (parser routes those through Call), but handle gracefully.

            let source = lower_place(expr_id, arena, c)?;
            let d = c.alloc_local();
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
                let field_val = lower_expr(*field_expr, arena, c)?;
                // B8: aggregate-containing-heap — struct field with Move type not supported.
                let field_ty = &c.local_decls[field_val.0].ty;
                if !field_ty.is_copy(None) {
                    return Err(LowerError::heap_type_not_supported(
                        &format!("struct `{struct_name}` field `{field_name}` type `{field_ty}`"),
                        expr_span,
                    ));
                }
                c.push(Statement::Assign {
                    dest: Place::local(d).project(Projection::Field(field_name.clone())),
                    source: Place::local(field_val),
                    span: expr_span.clone(),
                });
            }
            Ok(d)
        }
        Expr::EnumLiteral {
            name,
            variant_name,
            payload,
        } => {
            let d = c.alloc_local_ty(MirType::Struct(name.to_string()));
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
                let val = lower_expr(*payload_expr, arena, c)?;
                // B8: aggregate-containing-heap — enum payload with Move type not supported.
                let payload_ty = &c.local_decls[val.0].ty;
                if !payload_ty.is_copy(None) {
                    return Err(LowerError::heap_type_not_supported(
                        &format!("enum `{name}.{variant_name}` payload type `{payload_ty}`"),
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
            let scrut_local = lower_expr(*scrutinee, arena, c)?;

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
                            let body_val = lower_expr(arm.body, arena, c)?;
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
                    let body_val = lower_expr(wc.body, arena, c)?;
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
                            let body_val = lower_expr(arm.body, arena, c)?;
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
                    let body_val = lower_expr(wc.body, arena, c)?;
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

            // ── ADR-0064: match on an Integer scrutinee (value-keyed) ──
            // The scrutinee is an i64; SwitchInt directly on it. Integer's
            // domain is infinite, so exhaustiveness REQUIRES a wildcard — an
            // uncovered value with no `_` hits the default Trap (GAP-2,
            // runtime enforcement; compile-time exhaustiveness is a separate
            // campaign per ADR-0064 §4). Mirror of the Trit branch.
            if scrut_ty == MirType::Integer {
                use triet_syntax::{LiteralPattern, Pattern};
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
                            message: "wildcard `_` must be the last arm in an Integer match — \
                                      arms after wildcard are unreachable"
                                .to_string(),
                            span: pat_span,
                        });
                    }
                    match &pat.node {
                        Pattern::Wildcard => wildcard_arm = Some(arm),
                        Pattern::Literal(LiteralPattern::Integer {
                            value,
                            suffix: None,
                        }) => {
                            let key = i64::try_from(*value).map_err(|_| LowerError {
                                message: format!("Integer literal value {value} out of range"),
                                span: pat_span.clone(),
                            })?;
                            let arm_bb = c.alloc_bb();
                            cases.push((key, arm_bb));
                            c.cur = arm_bb;
                            c.push_scope();
                            let body_val = lower_expr(arm.body, arena, c)?;
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
                                    "unsupported pattern in Integer match: {other:?} — expected \
                                     an integer literal or `_`"
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
                    let body_val = lower_expr(wc.body, arena, c)?;
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
                    let body_val = lower_expr(arm.body, arena, c)?;
                    let result = c.alloc_local_ty(payload_ty.clone());
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
                    let body_val = lower_expr(arm.body, arena, c)?;
                    let arm_end = c.cur;
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
                    }
                }
                let body_val = lower_expr(arm_for_present.body, arena, c)?;
                let arm_end = c.cur;
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
                    let body_val = lower_expr(arm.body, arena, c)?;
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
                    let body_val = lower_expr(z_arm.body, arena, c)?;
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

            // Read discriminant.
            let disc_local = c.alloc_local();
            c.push(Statement::StorageLive(disc_local, expr_span.clone()));
            c.push(Statement::GetDiscriminant {
                dest: Place::local(disc_local),
                source: Place::local(scrut_local),
                span: expr_span.clone(),
            });

            // Build target blocks for each arm and the final merge.
            let cur_bb = c.cur;
            let merge_bb = c.alloc_bb();
            let result = c.alloc_local();
            c.push(Statement::StorageLive(result, expr_span.clone()));

            let mut cases: Vec<(i64, BasicBlock)> = Vec::new();

            // C2: Scan for wildcard arm BEFORE lowering.
            // Wildcard must be the last arm (arms after = unreachable).
            let mut wildcard_arm: Option<&triet_syntax::MatchArm> = None;
            for arm in arms.iter() {
                let pat = arena.pattern(arm.pattern);
                if matches!(&pat.node, triet_syntax::Pattern::Wildcard) {
                    if wildcard_arm.is_some() {
                        return Err(LowerError {
                            message: "duplicate wildcard `_` in enum match".to_string(),
                            span: expr_span.clone(),
                        });
                    }
                    wildcard_arm = Some(arm);
                } else if wildcard_arm.is_some() {
                    return Err(LowerError {
                        message: "wildcard `_` must be the last arm in an enum match — \
                                  arms after wildcard are unreachable"
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

                // Skip wildcard — lowered separately as default_bb.
                if matches!(&pat.node, triet_syntax::Pattern::Wildcard) {
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
                                    c.push(Statement::StorageLive(bind_local, expr_span.clone()));
                                    // Read payload into the binding.
                                    c.push(Statement::Assign {
                                        dest: Place::local(bind_local),
                                        source: Place::local(scrut_local)
                                            .project(Projection::Payload(variant_name.clone())),
                                        span: expr_span.clone(),
                                    });
                                    c.vars.insert(var_name.clone(), bind_local);
                                    c.push_owned(bind_local);
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
                        let body_val = lower_expr(arm.body, arena, c)?;
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
                    triet_syntax::Pattern::Variable(_var_name) => {
                        // Unit variant of the enum — the type checker
                        // records this in pattern_resolutions when the
                        // scrutinee is an enum and the name matches a
                        // unit variant.
                        if let Some(res) = resolution {
                            cases.push((res.discriminant, arm_bb));
                            c.cur = arm_bb;
                            c.push_scope();
                            // No payload to bind — unit variant.
                            let body_val = lower_expr(arm.body, arena, c)?;
                            let arm_end = c.cur;
                            // ADR-0056: type the merge result from the arm value.
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
                        } else {
                            return Err(LowerError {
                                message: "unsupported match pattern (expected enum variant, got variable not resolved by type checker)".to_string(),
                                span: expr_span.clone(),
                            });
                        }
                    }
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

            // C2: wildcard arm → default_bb instead of trap.
            let default_bb = if let Some(wc) = wildcard_arm {
                let wc_bb = c.alloc_bb();
                c.cur = wc_bb;
                c.push_scope();
                let body_val = lower_expr(wc.body, arena, c)?;
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
                args.push(lower_expr(*receiver, arena, c)?);
                for &a in arguments {
                    args.push(lower_expr(a, arena, c)?);
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
                            !ty.is_copy(None)
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
                            !ty.is_copy(None)
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
                            !ty.is_copy(None)
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

            // Qualified enum variant construction:
            // `OptionA.SomeInt(42)` parses as MethodCall.
            // Resolution was recorded by the type checker.
            let resolution = c.expr_resolutions.get(&expr_id).cloned();
            if let Some(res) = resolution {
                let d = c.alloc_local_ty(MirType::Enum(res.enum_name.clone()));
                c.push(Statement::StorageLive(d, expr_span.clone()));
                c.push(Statement::EnumAlloc {
                    dest: d,
                    enum_name: res.enum_name.clone(),
                    span: expr_span.clone(),
                });
                c.push(Statement::SetDiscriminant {
                    dest: d,
                    value: res.discriminant,
                    span: expr_span.clone(),
                });
                if res.has_payload {
                    let val = lower_expr(arguments[0], arena, c)?;
                    c.push(Statement::Assign {
                        dest: Place::local(d)
                            .project(Projection::Payload(res.variant_name.clone())),
                        source: Place::local(val),
                        span: expr_span.clone(),
                    });
                }
                return Ok(d);
            }
            Err(LowerError::unsupported_expr(
                &arena.expression(expr_id).node,
                expr_span,
            ))
        }
        Expr::Return { value } => {
            // `return` in expression position (e.g. inside `~->` body).
            let mut values = Vec::new();
            if let Some(v) = value {
                if is_null_expr(&arena.expression(*v).node)
                    && !matches!(c.sig.return_type, MirType::Outcome { .. })
                {
                    let ret_ty = c.sig.return_type.clone();
                    let d = c.alloc_local_ty(ret_ty);
                    c.push(Statement::StorageLive(d, expr_span.clone()));
                    c.push(Statement::Const {
                        dest: Place::local(d),
                        value: ConstValue::Integer(i128::from(triet_mir::NULL_SENTINEL)),
                        span: expr_span.clone(),
                    });
                    values.push(d);
                } else {
                    let val = lower_expr(*v, arena, c)?;
                    values.extend(lower_outcome_return_values(val, c));
                }
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
                let inner_val = lower_expr(*inner, arena, c)?;

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
                let body_val = lower_expr(*body, arena, c)?;
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
            let inner_val = lower_expr(*inner, arena, c)?;

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
                let body_val = lower_expr(*body, arena, c)?;
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
                    let e_local = c.alloc_local_ty(MirType::Unknown);
                    c.push(Statement::StorageLive(e_local, expr_span.clone()));
                    c.push(Statement::Assign {
                        dest: Place::local(e_local),
                        source: Place::local(inner_val).project(Projection::OutcomePayload),
                        span: expr_span.clone(),
                    });
                    c.vars.insert(name.clone(), e_local);
                    c.local_names.insert(e_local, name.clone());
                    c.push_owned(e_local);
                }
                lower_expr(*body, arena, c)?;
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
                let body_val = lower_expr(*body, arena, c)?;
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
                c.push(Statement::Assign {
                    dest: Place::local(result),
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
            expr_resolutions: ExprResolutions::new(),
            pattern_resolutions: PatternResolutions::new(),
            method_resolutions: MethodResolutions::new(),
            func_return_types: HashMap::new(),
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
            expr_resolutions: ExprResolutions::new(),
            pattern_resolutions: PatternResolutions::new(),
            method_resolutions: MethodResolutions::new(),
            func_return_types: HashMap::new(),
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

        let bodies = lower_program(
            &prog,
            &ExprResolutions::new(),
            &PatternResolutions::new(),
            &MethodResolutions::new(),
        )
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

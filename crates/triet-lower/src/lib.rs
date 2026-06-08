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
    FieldPath, FunctionSignature, Local, LocalDecl, ParameterPassing, Place, Projection, Span,
    Statement, StructLayout, Terminator,
};
use triet_syntax::{
    Arena, BinaryOperator, Expr, ExprId, ExprResolutions, FunctionBody, FunctionDef, Item,
    PatternResolutions, Program, ReferenceForm, Stmt, TypeExpr, TypeId, UnaryOperator,
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

/// Per-function lowering state: local/block allocation, variable scope,
/// the basic blocks built so far, and the signature under construction.
struct Ctx {
    vars: HashMap<String, Local>,
    local_decls: Vec<LocalDecl>,
    cur: BasicBlock,
    next_bb: usize,
    mir_blocks: Vec<triet_mir::BlockData>,
    sig: FunctionSignature,
    /// Set of type names that are user-defined structs. Used to detect
    /// struct params, returns, and copies.
    struct_names: std::collections::HashSet<String>,
    /// Struct layouts keyed by name (cloned from lower_program's computation).
    struct_layouts: HashMap<String, StructLayout>,
    /// Set of type names that are user-defined enums.
    /// Used for enum type detection (e.g., enum copy lowering — future commit).
    #[allow(dead_code)]
    enum_names: std::collections::HashSet<String>,
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
    /// Map from function name to its return type name.
    func_return_types: HashMap<String, String>,
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
        ret: &str,
        struct_names: std::collections::HashSet<String>,
        struct_layouts: HashMap<String, StructLayout>,
        enum_names: std::collections::HashSet<String>,
        enum_layouts: HashMap<String, EnumLayout>,
        expr_resolutions: ExprResolutions,
        pattern_resolutions: PatternResolutions,
        func_return_types: HashMap<String, String>,
    ) -> Self {
        let is_struct_return = struct_names.contains(ret);
        // ADR-0049 L6 Lối d: String uses JIT-sret but keeps M4-escape Return[s].
        let is_fat_return = is_struct_return || ret == "String";
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
                params: Vec::new(),
                return_type: ret.to_string(),
                return_borrow_map: triet_mir::ReturnBorrowMap::new(),
                return_shape: if is_fat_return {
                    triet_mir::ReturnShape::Struct {
                        struct_name: ret.to_string(),
                    }
                } else {
                    triet_mir::ReturnShape::Scalar
                },
            },
            struct_names,
            struct_layouts,
            enum_names,
            enum_layouts,
            expr_resolutions,
            pattern_resolutions,
            func_return_types,
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

    fn is_fat_type(&self, name: &str) -> bool {
        self.struct_names.contains(name) || name == "String"
    }

    #[allow(dead_code)]
    fn is_struct_type(&self, name: &str) -> bool {
        self.struct_names.contains(name)
    }

    #[allow(dead_code)]
    fn is_enum_type(&self, name: &str) -> bool {
        self.enum_names.contains(name)
    }

    /// Allocate a fresh local with a declared type name.
    fn alloc_local_ty(&mut self, ty: &str) -> Local {
        let l = Local(self.local_decls.len());
        self.local_decls.push(LocalDecl::new(ty));
        l
    }

    /// Allocate a temporary whose type is not tracked yet.
    fn alloc_local(&mut self) -> Local {
        self.alloc_local_ty("?")
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
            ty.starts_with('&')
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

/// Lower every function in a parsed program to its MIR body.
///
/// Returns `Err` if any function contains an AST construct the lowerer
/// does not yet support. The error carries a span for diagnostics.
///
/// Struct definitions in the program are lowered to `StructLayout` entries
/// and attached to every function body so that the JIT backend can compute
/// field offsets. In Bậc A every field is 8 bytes (i64), alignment 8.
pub fn lower_program(
    prog: &Program,
    expr_resolutions: &ExprResolutions,
    pattern_resolutions: &PatternResolutions,
) -> Result<Vec<Body>, LowerError> {
    // ── Collect struct layouts from struct definitions ──────────
    // Bậc A: every field is 8 bytes, alignment 8 (single i64).
    // Bậc C will compute real sizes from type information.
    let mut struct_layouts: Vec<StructLayout> = prog
        .items
        .iter()
        .filter_map(|item| {
            if let Item::Struct { def } = &item.node {
                let fields: Vec<(String, String, usize, usize)> = def
                    .fields
                    .iter()
                    .map(|f| {
                        let ty = type_name(&prog.arena, f.type_annotation);
                        (f.name.clone(), ty, 8, 8)
                    })
                    .collect();
                Some(StructLayout::compute(&def.name, &fields))
            } else {
                None
            }
        })
        .collect();

    let struct_names: std::collections::HashSet<String> =
        struct_layouts.iter().map(|l| l.name.clone()).collect();
    let mut struct_map: HashMap<String, StructLayout> = struct_layouts
        .iter()
        .map(|l| (l.name.clone(), l.clone()))
        .collect();

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
            ("ptr".to_string(), "Integer".to_string(), 8, 8),
            ("len".to_string(), "Integer".to_string(), 8, 8),
            ("cap".to_string(), "Integer".to_string(), 8, 8),
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
                    Option<(String, usize, usize, Vec<FieldLayout>)>,
                )> = def
                    .variants
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let disc = i as i64;
                        let payload = v.payload.map(|tid| {
                            let ty_name = type_name(&prog.arena, tid);
                            // Bậc A: every type is 8-byte i64
                            (ty_name, 8usize, 8usize, Vec::new())
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

    let enum_names: std::collections::HashSet<String> =
        enum_layouts.iter().map(|l| l.name.clone()).collect();
    let enum_map: HashMap<String, EnumLayout> = enum_layouts
        .iter()
        .map(|l| (l.name.clone(), l.clone()))
        .collect();

    let func_return_types: HashMap<String, String> = prog
        .items
        .iter()
        .filter_map(|item| {
            if let Item::Function { def } = &item.node {
                let ret = def
                    .return_type
                    .map(|tid| type_name(&prog.arena, tid))
                    .unwrap_or_else(|| "Integer".to_string());
                Some((def.name.clone(), ret))
            } else {
                None
            }
        })
        .collect();

    let mut bodies = Vec::new();
    for item in &prog.items {
        if let Item::Function { def } = &item.node {
            let mut body = lower_function(
                def,
                &prog.arena,
                item.span.clone(),
                struct_names.clone(),
                struct_map.clone(),
                enum_names.clone(),
                enum_map.clone(),
                expr_resolutions.clone(),
                pattern_resolutions.clone(),
                func_return_types.clone(),
            )?;
            body.struct_layouts = struct_layouts.clone();
            body.enum_layouts = enum_layouts.clone();
            bodies.push(body);
        }
    }
    Ok(bodies)
}

/// Lower one function definition to a MIR body.
///
/// `span` is the byte range of the function definition in the source file,
/// used as a fallback for body-level synthetic statements.
pub fn lower_function(
    func: &FunctionDef,
    arena: &Arena,
    span: Span,
    struct_names: std::collections::HashSet<String>,
    struct_layouts: HashMap<String, StructLayout>,
    enum_names: std::collections::HashSet<String>,
    enum_layouts: HashMap<String, EnumLayout>,
    expr_resolutions: ExprResolutions,
    pattern_resolutions: PatternResolutions,
    func_return_types: HashMap<String, String>,
) -> Result<Body, LowerError> {
    let ret_ty = func
        .return_type
        .as_ref()
        .map(|tid| type_name(arena, *tid))
        .unwrap_or_else(|| "Integer".to_string());
    let mut c = Ctx::new(
        &func.name,
        &ret_ty,
        struct_names,
        struct_layouts,
        enum_names,
        enum_layouts,
        expr_resolutions,
        pattern_resolutions,
        func_return_types,
    );
    let entry = c.cur;

    // Function scope: Drop all owned locals (params + let bindings) when
    // the function exits. Must start before pushing parameters.
    c.push_scope();

    for p in &func.params {
        let ty = type_name(arena, p.type_annotation);
        // B7-lift (ADR-0042): heap types now allowed as parameters.
        // Move semantics: callee owns + drops, caller zeroes slot after call.
        // ADR-0045 §2: reference types (&0 String etc.) are borrow params
        // — callee does NOT own, must NOT drop. Heap types with non-ref
        // annotations (e.g. s: String) remain Move.
        let l = c.alloc_local_ty(&ty);
        c.vars.insert(p.name.clone(), l);
        let passing = match p.passing_mode {
            triet_syntax::ParameterPassing::Borrow => ParameterPassing::Borrow,
            triet_syntax::ParameterPassing::Move => ParameterPassing::Move,
            triet_syntax::ParameterPassing::MutableBorrow => ParameterPassing::MutableBorrow,
        };
        // Only push_owned for Move params and non-reference types.
        // Reference types (&0 String) — callee borrows, no Drop.
        let is_ref_type = ty.starts_with('&');
        if matches!(passing, ParameterPassing::Move) || !is_ref_type {
            c.push_owned(l);
        }
        c.sig.params.push((p.name.clone(), passing));
    }

    // ADR-0046 §3: populate return_borrow_map for return-borrow elision.
    // If the return type is &0 T, tie it to the single ref-param.
    // Elision rule (check_lifetime_elision, check.rs:494) guarantees
    // exactly 0 or 1 non-owning ref-params — 0 = fn with no ref params
    // returning &0 T (unusual but valid: return a borrowed static/global);
    // 1 = tie to that param. 2+ is refused by E2400 (fatal at typecheck).
    // defense-in-depth: if typecheck leaks, Err — not panic — because
    // the harness (integration_tests.rs:64) runs through type errors and
    // panic would SIGABRT the entire corpus.
    if c.sig.return_type.starts_with("&0 ") {
        let ref_param_indices: Vec<usize> = c
            .sig
            .params
            .iter()
            .enumerate()
            .filter(|(_, (name, _))| {
                // ADR-0046 Blocker 2 fix: count by type-string & prefix
                // (Lối 1 — Move-vs-Borrow quyết theo type, không theo
                // ParameterPassing).  Mọi non-owning ref (&0/&0 mutable/&-)
                // bắt đầu bằng '&' nhưng KHÔNG bằng "&+".
                if let Some(&local) = c.vars.get(name) {
                    let ty = &c.local_decls[local.0].ty;
                    ty.starts_with('&') && !ty.starts_with("&+")
                } else {
                    false
                }
            })
            .map(|(i, _)| i)
            .collect();
        match ref_param_indices.len() {
            0 => {} // No ref-params to tie to — valid (static/global return).
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

    match &func.body {
        FunctionBody::Block { block } => {
            lower_block(*block, arena, &mut c)?;
        }
        FunctionBody::Expression { expr } => {
            let val = lower_expr(*expr, arena, &mut c)?;
            let cur = c.cur;
            let expr_span = arena.expression(*expr).span.clone();
            c.term(
                cur,
                Terminator::Return {
                    values: vec![val],
                    span: expr_span,
                },
            );
        }
        FunctionBody::External { .. } => {}
    }

    // Flush owned locals (params + let bindings) before building the body.
    // If a `return` already flushed everything, this is a no-op.
    c.pop_scope();

    // A block-form body that falls off the end returns unit.
    // This is a synthetic return — use DUMMY_SPAN since it has no source.
    let cur = c.cur;
    if c.is_open(cur) {
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

/// Resolve a type annotation to a display name (best-effort; non-`Named`
/// types collapse to "?").
fn type_name(arena: &Arena, id: TypeId) -> String {
    match &arena.type_expression(id).node {
        TypeExpr::Named(n) => n.clone(),
        TypeExpr::Nullable(inner) => {
            let inner_name = type_name(arena, *inner);
            format!("{inner_name}?")
        }
        TypeExpr::Reference { form, inner } => {
            let inner_name = type_name(arena, *inner);
            // TECH-DEBT(ADR-0045): MIR-type-as-string, xem §3.
            let prefix = match form {
                ReferenceForm::StrongFrozen => "&+ ",
                ReferenceForm::StrongMutable => "&+ mutable ",
                ReferenceForm::BorrowReadOnly => "&0 ",
                ReferenceForm::BorrowExclusiveMutable => "&0 mutable ",
                ReferenceForm::WeakObserver => "&- ",
            };
            format!("{prefix}{inner_name}")
        }
        _ => "?".to_string(),
    }
}

/// Simplified is_copy for use during lowering (before Body is built).
/// Only handles primitive type names and known heap types — does NOT
/// recurse into struct/enum layouts (those aren't built yet).
///
/// Must match the canonical [`triet_mir::is_copy`] semantics:
/// - Known stack primitives → Copy (Integer, Trit, Tryte, Long, Trilean,
///   Unit, "?").
/// - Known heap types → Move (String, Vector, HashMap). Vector uses a
///   prefix match ("Vector<Integer>" etc.).
/// - User-defined struct/enum types → Copy.
///   SOUND ONLY WHILE B8 REFUSES CONSTRUCTION of aggregates with heap
///   fields (B8 blocks StructLiteral/enum-payload construction, not
///   declaration). When B8 is relaxed in Bậc B, this function must
///   consult layout field types or be replaced by the canonical
///   [`triet_mir::is_copy`] which recurses into layouts.
/// - Unknown types → Move (refuse-over-guess, same as canonical).
fn simple_is_copy(
    ty: &str,
    struct_names: &std::collections::HashSet<String>,
    enum_names: &std::collections::HashSet<String>,
) -> bool {
    match ty {
        // Stack primitives — Copy per SPEC §10.1.
        "Integer" | "Trit" | "Tryte" | "Long" | "Trilean" | "Unit" | "?" => true,
        // Reference types — Copy by design (ADR-0045 §3).
        // TECH-DEBT(ADR-0045): MIR-type-as-string, xem §3.
        other if other.starts_with('&') => true,
        // Heap types — Move.
        "String" | "HashMap" => false,
        other if triet_mir::is_vec_type(other) => false,
        other if triet_mir::is_hashmap_type(other) => false,
        other if struct_names.contains(other) => true,
        other if enum_names.contains(other) => true,
        // Unknown types default to Move (refuse-over-guess).
        _ => false,
    }
}

/// Emit a `CallDispatch` terminator targeting a builtin shim, allocate a
/// return local of `dest_ty`, and advance `c.cur` to the return block.
/// Returns the destination local holding the shim's return value.
fn emit_shim_call(
    c: &mut Ctx,
    shim_name: &str,
    args: Vec<Local>,
    dest_ty: &str,
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
                    .map(|tid| type_name(arena, *tid))
                    .ok_or_else(|| {
                        LowerError::null_literal_without_expected_type(stmt_span.clone())
                    })?;
                let d = c.alloc_local_ty(&ann_ty);
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
                    let ann_ty = type_name(arena, *tid);
                    if ann_ty != "?" {
                        c.local_decls[v.0].ty = ann_ty;
                    }
                }
                // M2: If init is an Identifier of a Move type, emit Assign + new local
                // instead of aliasing. This creates a genuine move-site so JIT's
                // Zeroing-on-Move (M1) can zero the source variable.
                let is_move_binding =
                    if let Expr::Identifier { name: _ } = &arena.expression(*init).node {
                        let ty = &c.local_decls[v.0].ty;
                        !simple_is_copy(ty, &c.struct_names, &c.enum_names)
                    } else {
                        false
                    };
                if is_move_binding {
                    let new_local = c.alloc_local_ty(&c.local_decls[v.0].ty.clone());
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
            if let (Some(sret), Some(v)) = (c.sret_ptr, value) {
                // Struct return via sret: copy struct fields to the caller's buffer.
                let struct_local = lower_expr(*v, arena, c)?;
                let source_ty = c.local_decls[struct_local.0].ty.clone();
                if c.struct_names.contains(&source_ty) {
                    // Copy struct: copy fields to sret buffer.
                    if let Some(layout) = c.struct_layouts.get(&source_ty) {
                        let field_names: Vec<String> =
                            layout.fields.iter().map(|f| f.name.clone()).collect();
                        for field_name in field_names {
                            let dest_place =
                                Place::local(sret).project(Projection::Field(field_name.clone()));
                            let source_place =
                                Place::local(struct_local).project(Projection::Field(field_name));
                            c.push(Statement::Assign {
                                dest: dest_place,
                                source: source_place,
                                span: stmt_span.clone(),
                            });
                        }
                    }
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
                    if is_null_expr(&arena.expression(*v).node) {
                        let ret_ty = c.sig.return_type.clone();
                        let d = c.alloc_local_ty(&ret_ty);
                        c.push(Statement::StorageLive(d, stmt_span.clone()));
                        c.push(Statement::Const {
                            dest: Place::local(d),
                            value: ConstValue::Integer(i128::from(triet_mir::NULL_SENTINEL)),
                            span: stmt_span.clone(),
                        });
                        values.push(d);
                    } else {
                        values.push(lower_expr(*v, arena, c)?);
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
        Expr::IntegerLiteral { value, .. } => {
            let d = c.alloc_local();
            c.push(Statement::StorageLive(d, expr_span.clone()));
            c.push(Statement::Const {
                dest: Place::local(d),
                value: ConstValue::Integer(*value),
                span: expr_span,
            });
            Ok(d)
        }
        Expr::TernaryLiteral { value } => {
            let d = c.alloc_local();
            c.push(Statement::StorageLive(d, expr_span.clone()));
            c.push(Statement::Const {
                dest: Place::local(d),
                value: ConstValue::Integer(*value),
                span: expr_span,
            });
            Ok(d)
        }
        Expr::TritLiteral { value } => {
            let d = c.alloc_local();
            c.push(Statement::StorageLive(d, expr_span.clone()));
            c.push(Statement::Const {
                dest: Place::local(d),
                value: ConstValue::Trit(*value),
                span: expr_span,
            });
            Ok(d)
        }
        Expr::TrileanLiteral { value } => {
            let d = c.alloc_local();
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
        Expr::OutcomeConstructor { arm, payload } => match arm {
            triet_syntax::OutcomeArm::Positive => {
                // ~+ e — identity/widening, lower the payload directly.
                if let Some(p) = payload {
                    lower_expr(*p, arena, c)
                } else {
                    // ~+ with no payload (bare Positive arm) — unsupported.
                    Err(LowerError::unsupported_expr(
                        &arena.expression(expr_id).node,
                        expr_span,
                    ))
                }
            }
            triet_syntax::OutcomeArm::Zero => {
                // ~0 without expected type — same as NullLiteral.
                Err(LowerError::null_literal_without_expected_type(expr_span))
            }
            triet_syntax::OutcomeArm::Negative => {
                // ~- e — Outcome error arm, not in Bậc A scope.
                Err(LowerError::unsupported_expr(
                    &arena.expression(expr_id).node,
                    expr_span,
                ))
            }
        },
        Expr::StringLiteral { value } => {
            let d = c.alloc_local_ty("String");
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
                let d = c.alloc_local_ty(&resolution.enum_name);
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
                    let d = c.alloc_local_ty(&resolution.enum_name);
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
                    if !simple_is_copy(payload_ty, &c.struct_names, &c.enum_names) {
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
                && c.is_enum_type(enum_name)
                && arguments.len() == 1
            {
                // Lower as enum literal with payload.
                let d = c.alloc_local_ty(enum_name);
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
                    let dest =
                        emit_shim_call(c, "__triet_string_concat", args, "String", expr_span);
                    return Ok(dest);
                }
                "eq" => {
                    let args: Vec<Local> = arguments
                        .iter()
                        .map(|a| lower_expr(*a, arena, c))
                        .collect::<Result<Vec<_>, _>>()?;
                    let dest = emit_shim_call(c, "__triet_string_eq", args, "Integer", expr_span);
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
                    if arg_ty == "String" {
                        let d = c.alloc_local_ty("Integer");
                        c.push(Statement::Assign {
                            dest: Place::local(d),
                            source: Place::local(arg).project(Projection::Field("len".to_string())),
                            span: expr_span.clone(),
                        });
                        return Ok(d);
                    }
                    let base_ty = arg_ty
                        .strip_prefix("&0 ")
                        .or_else(|| arg_ty.strip_prefix("&+ "))
                        .or_else(|| arg_ty.strip_prefix("&+ mutable "))
                        .or_else(|| arg_ty.strip_prefix("&0 mutable "))
                        .or_else(|| arg_ty.strip_prefix("&- "))
                        .unwrap_or(arg_ty);
                    let shim_name = match base_ty {
                        "String" => "__triet_string_len",
                        ty if triet_mir::is_vec_type(ty) => "__triet_vector_len",
                        ty if triet_mir::is_hashmap_type(ty) => "__triet_hashmap_len",
                        other => {
                            return Err(LowerError::heap_type_not_supported(
                                &format!(
                                    "len() on type `{other}` — expected String, Vector, or HashMap"
                                ),
                                expr_span,
                            ));
                        }
                    };
                    let dest = emit_shim_call(c, shim_name, vec![arg], "Integer", expr_span);
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
                    let base_ty = arg0_ty
                        .strip_prefix("&0 ")
                        .or_else(|| arg0_ty.strip_prefix("&+ "))
                        .or_else(|| arg0_ty.strip_prefix("&+ mutable "))
                        .or_else(|| arg0_ty.strip_prefix("&0 mutable "))
                        .or_else(|| arg0_ty.strip_prefix("&- "))
                        .unwrap_or(arg0_ty);
                    let shim_name = match base_ty {
                        "String" => "__triet_string_contains",
                        ty if triet_mir::is_vec_type(ty) => "__triet_vector_contains",
                        ty if triet_mir::is_hashmap_type(ty) => "__triet_hashmap_contains",
                        other => {
                            return Err(LowerError::heap_type_not_supported(
                                &format!(
                                    "contains() on type `{other}` — expected String, Vector, or HashMap"
                                ),
                                expr_span,
                            ));
                        }
                    };
                    let dest = emit_shim_call(c, shim_name, args, "Trilean", expr_span);
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
                    let len_dest = if arg_ty == "String" {
                        let d = c.alloc_local_ty("Integer");
                        c.push(Statement::Assign {
                            dest: Place::local(d),
                            source: Place::local(arg).project(Projection::Field("len".to_string())),
                            span: expr_span.clone(),
                        });
                        d
                    } else {
                        let base_ty = arg_ty
                            .strip_prefix("&0 ")
                            .or_else(|| arg_ty.strip_prefix("&+ "))
                            .or_else(|| arg_ty.strip_prefix("&+ mutable "))
                            .or_else(|| arg_ty.strip_prefix("&0 mutable "))
                            .or_else(|| arg_ty.strip_prefix("&- "))
                            .unwrap_or(arg_ty);
                        let len_shim = match base_ty {
                            "String" => "__triet_string_len",
                            ty if triet_mir::is_vec_type(ty) => "__triet_vector_len",
                            ty if triet_mir::is_hashmap_type(ty) => "__triet_hashmap_len",
                            other => {
                                return Err(LowerError::heap_type_not_supported(
                                    &format!(
                                        "is_empty() on type `{other}` — expected String, Vector, or HashMap"
                                    ),
                                    expr_span,
                                ));
                            }
                        };
                        emit_shim_call(c, len_shim, vec![arg], "Integer", expr_span.clone())
                    };
                    // Compare len == 0 → Trilean encoding (Eq returns 1 or -1).
                    let result = c.alloc_local_ty("Trilean");
                    let zero = c.alloc_local_ty("Integer");
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
                    let base_ty = arg_ty
                        .strip_prefix("&0 mutable ")
                        .or_else(|| arg_ty.strip_prefix("&0 "))
                        .or_else(|| arg_ty.strip_prefix("&+ "))
                        .or_else(|| arg_ty.strip_prefix("&- "))
                        .unwrap_or(arg_ty);
                    let shim_name = match base_ty {
                        "String" => "__triet_string_clear",
                        other => {
                            return Err(LowerError::heap_type_not_supported(
                                &format!("clear() on type `{other}` — expected String"),
                                expr_span,
                            ));
                        }
                    };
                    let dest = emit_shim_call(c, shim_name, vec![arg], "Integer", expr_span);
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
                    let base_ty = arg_ty
                        .strip_prefix("&0 mutable ")
                        .or_else(|| arg_ty.strip_prefix("&0 "))
                        .or_else(|| arg_ty.strip_prefix("&+ "))
                        .or_else(|| arg_ty.strip_prefix("&- "))
                        .unwrap_or(arg_ty);
                    let shim_name = match base_ty {
                        "String" => "__triet_string_append",
                        other => {
                            return Err(LowerError::heap_type_not_supported(
                                &format!("append() on type `{other}` — expected String"),
                                expr_span,
                            ));
                        }
                    };
                    let dest =
                        emit_shim_call(c, shim_name, vec![arg, byte_arg], "Integer", expr_span);
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
                    let shim_name = if triet_mir::is_hashmap_type(arg0_ty) {
                        "__triet_hashmap_get"
                    } else if triet_mir::is_vec_type(arg0_ty) {
                        "__triet_vector_get"
                    } else {
                        return Err(LowerError::heap_type_not_supported(
                            &format!("get() on type `{arg0_ty}` — expected Vector or HashMap"),
                            expr_span,
                        ));
                    };
                    let dest = emit_shim_call(c, shim_name, args, "Integer?", expr_span);
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
                    let len_local = c.alloc_local_ty("Integer");
                    c.push(Statement::Const {
                        dest: Place::local(len_local),
                        value: ConstValue::Integer(0),
                        span: DUMMY_SPAN,
                    });
                    let cap_local = c.alloc_local_ty("Integer");
                    c.push(Statement::Const {
                        dest: Place::local(cap_local),
                        value: ConstValue::Integer(2),
                        span: DUMMY_SPAN,
                    });
                    let dest = emit_shim_call(
                        c,
                        "__triet_vector_alloc",
                        vec![len_local, cap_local],
                        "Vector<Integer>",
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
                    let len_local = c.alloc_local_ty("Integer");
                    c.push(Statement::Const {
                        dest: Place::local(len_local),
                        value: ConstValue::Integer(0),
                        span: DUMMY_SPAN,
                    });
                    let cap_local = c.alloc_local_ty("Integer");
                    c.push(Statement::Const {
                        dest: Place::local(cap_local),
                        value: ConstValue::Integer(4),
                        span: DUMMY_SPAN,
                    });
                    let dest = emit_shim_call(
                        c,
                        "__triet_hashmap_alloc",
                        vec![len_local, cap_local],
                        "HashMap<Integer,Integer>",
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
                .unwrap_or_else(|| "Integer".to_string());
            let is_fat_ret = c.is_fat_type(&callee_ret);

            let mut args: Vec<Local> = arguments
                .iter()
                .map(|a| lower_expr(*a, arena, c))
                .collect::<Result<Vec<_>, _>>()?;

            // B7-lift (ADR-0042): heap args now allowed.
            // Move semantics: caller zeroes slot after call, borrowck
            // enforces E2420 use-after-move.
            if is_fat_ret {
                // sret: allocate struct local for return, pass as hidden arg[0].
                let ret_local = c.alloc_local_ty(&callee_ret);
                c.push(Statement::StorageLive(ret_local, expr_span.clone()));
                c.push(Statement::StructAlloc {
                    dest: ret_local,
                    struct_name: callee_ret.clone(),
                    span: expr_span.clone(),
                });
                // Insert sret pointer as first argument.
                args.insert(0, ret_local);
                // ADR-0042 Q1 + ADR-0045 §2: collect args for zeroing.
                // Skip arg[0] — sret pointer is not user-visible.
                // Skip reference types (&0 String etc.) — callee borrows.
                let to_zero: Vec<Local> = args[1..]
                    .iter()
                    .filter(|&&arg| {
                        let ty = &c.local_decls[arg.0].ty;
                        if ty.starts_with('&') {
                            return false;
                        }
                        !simple_is_copy(ty, &c.struct_names, &c.enum_names)
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
                            struct_name: callee_ret,
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
            } else {
                let dest = c.alloc_local_ty(&callee_ret);
                c.push(Statement::StorageLive(dest, expr_span.clone()));
                // ADR-0042 Q1 + ADR-0045 §2: collect args for zeroing.
                // Skip reference types (&0 String etc.) — callee borrows.
                let to_zero: Vec<Local> = args
                    .iter()
                    .filter(|&&arg| {
                        let ty = &c.local_decls[arg.0].ty;
                        if ty.starts_with('&') {
                            return false;
                        }
                        !simple_is_copy(ty, &c.struct_names, &c.enum_names)
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
                Some(e) => lower_expr(e, arena, c),
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
            let result = c.alloc_local();

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
            let payload_ty = triet_mir::nullable_payload(&obj_ty)
                .ok_or_else(|| {
                    LowerError::unsupported_expr(&arena.expression(expr_id).node, expr_span.clone())
                })?
                .to_string();

            // ── Null path: evaluate default expression ──
            c.cur = null_bb;
            let default_val = lower_expr(*default, arena, c)?;
            let null_end = c.cur;
            let result = c.alloc_local_ty(&payload_ty);
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
        Expr::Borrow { form, operand } => {
            let mir_form = lower_ref_form(*form);
            // The operand is an lvalue (IDENT or field-access chain per
            // ADR-0031 §2) — lower it to a projected Place so the borrow
            // checker can track the loan at field granularity.
            let source = lower_place(*operand, arena, c)?;
            // ADR-0045 §3: set the borrow temporary's type to the real
            // reference type (e.g., "&0 String") so downstream dispatch
            // can strip the prefix and match the base heap type.
            let prefix = match mir_form {
                triet_mir::ReferenceForm::BorrowReadOnly => "&0 ",
                triet_mir::ReferenceForm::BorrowExclusiveMutable => "&0 mutable ",
                triet_mir::ReferenceForm::StrongFrozen => "&+ ",
                triet_mir::ReferenceForm::StrongMutable => "&+ mutable ",
                triet_mir::ReferenceForm::WeakObserver => "&- ",
            };
            let source_ty = &c.local_decls[source.local.0].ty;
            let ref_ty = format!("{prefix}{source_ty}");
            let dest = c.alloc_local_ty(&ref_ty);
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
                let d = c.alloc_local_ty(&res.enum_name);
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
            let d = c.alloc_local_ty(struct_name);
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
                if !simple_is_copy(field_ty, &c.struct_names, &c.enum_names) {
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
            let d = c.alloc_local_ty(name);
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
                if !simple_is_copy(payload_ty, &c.struct_names, &c.enum_names) {
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

            // ── Nullable match: branch on ~+ / ~0 via NULL_SENTINEL ──
            let scrut_ty = c.local_decls[scrut_local.0].ty.clone();
            if triet_mir::is_nullable_type(&scrut_ty) {
                use triet_syntax::{MatchArm, OutcomeArm, Pattern};
                let payload_ty = triet_mir::nullable_payload(&scrut_ty)
                    .ok_or_else(|| {
                        LowerError::unsupported_expr(
                            &arena.expression(expr_id).node,
                            expr_span.clone(),
                        )
                    })?
                    .to_string();

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
                    let result = c.alloc_local_ty(&payload_ty);
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
                let result = c.alloc_local_ty(&payload_ty);
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
                        let bind_local = c.alloc_local_ty(&payload_ty);
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

            // Pre-allocate arm blocks and emit body lowering for each.
            // Variant resolution is provided by the type checker via
            // pattern_resolutions — the lowerer does NOT scan enum_layouts.
            for arm in arms.iter() {
                let arm_bb = c.alloc_bb();
                let pat = arena.pattern(arm.pattern);

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
                                    let bind_local = c.alloc_local_ty("?");
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

            // Create trap block for unknown discriminants.
            let trap_bb = c.alloc_bb();
            c.cur = trap_bb;
            c.term(
                trap_bb,
                Terminator::Trap {
                    span: expr_span.clone(),
                },
            );

            // Emit SwitchInt terminator.
            c.term(
                cur_bb,
                Terminator::SwitchInt {
                    discriminant: disc_local,
                    cases,
                    default_bb: trap_bb,
                    span: expr_span.clone(),
                },
            );

            c.cur = merge_bb;
            Ok(result)
        }
        Expr::MethodCall {
            receiver: _,
            method: _,
            arguments,
        } => {
            // Qualified enum variant construction:
            // `OptionA.SomeInt(42)` parses as MethodCall.
            // Resolution was recorded by the type checker.
            let resolution = c.expr_resolutions.get(&expr_id).cloned();
            if let Some(res) = resolution {
                let d = c.alloc_local_ty(&res.enum_name);
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

    fn lower_source(source: &str) -> Body {
        let (prog, errors) = triet_parser::parse(source);
        assert!(errors.is_empty(), "parse errors: {errors:?}");
        // Scan for struct/enum definitions so simple_is_copy recognizes them.
        // Must match production (lib.rs:349/389): struct_names and enum_names
        // are separate sets — mixing them would cause is_struct_return to
        // misclassify enum-returning functions as struct returns.
        let struct_names: std::collections::HashSet<String> = prog
            .items
            .iter()
            .filter_map(|item| match &item.node {
                Item::Struct { def, .. } => Some(def.name.clone()),
                _ => None,
            })
            .collect();
        let enum_names: std::collections::HashSet<String> = prog
            .items
            .iter()
            .filter_map(|item| match &item.node {
                Item::Enum { def, .. } => Some(def.name.clone()),
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
        lower_function(
            &func.0,
            &prog.arena,
            func.1,
            struct_names,
            HashMap::new(),
            enum_names,
            HashMap::new(),
            ExprResolutions::new(),
            PatternResolutions::new(),
            HashMap::new(),
        )
        .expect("lowering failed")
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
        assert_eq!(body.local_decls[0].ty, "Integer");
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

        let bodies = lower_program(&prog, &ExprResolutions::new(), &PatternResolutions::new())
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
            dest_ty, "String",
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

    #[test]
    fn simple_is_copy_agrees_with_canonical_is_copy() {
        // Phase 4.3b M1.3: lowerer's simple_is_copy must match
        // triet_mir::is_copy for all type strings used in Bậc A.
        // Build a minimal body via lower_source so the struct shapes
        // are always correct without hardcoding field lists.
        let body = lower_source("function test() {}");
        let cases = &[
            ("Integer", true),
            ("String", false),
            ("HashMap", false),
            ("Vector<Integer>", false),
            ("Vector", false),
            ("Trilean", true),
            ("Unit", true),
            ("UnknownType", false), // canonical is_copy default-Move
        ];
        for &(ty, expected) in cases {
            let empty_set = std::collections::HashSet::new();
            let simple = simple_is_copy(ty, &empty_set, &empty_set);
            let canonical = triet_mir::is_copy(ty, &body);
            assert_eq!(
                simple, canonical,
                "simple_is_copy({ty:?}) = {simple}, triet_mir::is_copy = {canonical} — must agree"
            );
            assert_eq!(simple, expected, "is_copy({ty:?}) should be {expected}");
        }
    }

    // ── Nullable lowering tests (ADR-0041 Bước 3) ──────────────

    /// N3: `~0` without expected type → `Err(LowerError)` with span, not panic.
    #[test]
    fn null_literal_without_expected_type_is_error() {
        // Positive: `let x: Integer? = ~0` has expected type → must succeed.
        let body = lower_source("function test() { let x: Integer? = ~0; }");
        assert!(
            body.local_decls.iter().any(|d| d.ty == "Integer?"),
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

    /// Point 1 + M1: type_name emits "Integer?" + Elvis result typed T (not T?).
    #[test]
    fn type_name_nullable_and_elvis_result_typed_correctly() {
        let body =
            lower_source("function test() -> Integer { let x: Integer? = 5; return x ?: 0; }");
        // type_name must emit "Integer?" for the let binding.
        let has_nullable_local = body.local_decls.iter().any(|d| d.ty == "Integer?");
        assert!(
            has_nullable_local,
            "expected a local with type 'Integer?', got locals: {:?}",
            body.local_decls.iter().map(|d| &d.ty).collect::<Vec<_>>()
        );
        // M1: Elvis result must be typed "Integer" (payload, not T? and not "?").
        let has_result_integer = body.local_decls.iter().any(|d| d.ty == "Integer");
        assert!(
            has_result_integer,
            "Elvis result local must be typed Integer (payload), got locals: {:?}",
            body.local_decls.iter().map(|d| &d.ty).collect::<Vec<_>>()
        );
    }

    /// B1 (§3): type_name renders reference types (e.g. "&0 String"),
    /// not the old "?" fallback.
    #[test]
    fn type_name_renders_reference_type() {
        let body = lower_source(
            "function process(s: &0 String) -> Integer { return 0; } \
             function main() -> Integer { return 0; }",
        );
        let param_ty = &body.local_decls[0].ty;
        assert_eq!(
            param_ty, "&0 String",
            "B1 regression: reference param must carry real type, not '?'"
        );
    }

    /// B2 (§2 callee): no Drop for reference-type borrow params.
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

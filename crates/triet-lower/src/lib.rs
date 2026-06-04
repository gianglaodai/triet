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

use std::collections::HashMap;

use triet_mir::{
    BasicBlock, BinOp, Body, CallTarget, ConstValue, DUMMY_SPAN, FunctionSignature, Local,
    LocalDecl, ParameterPassing, Place, Projection, Span, Statement, StructLayout, Terminator,
};
use triet_syntax::{
    Arena, BinaryOperator, Expr, ExprId, FunctionBody, FunctionDef, Item, Program, Stmt, TypeExpr,
    TypeId,
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
}

impl Ctx {
    fn new(
        name: &str,
        ret: &str,
        struct_names: std::collections::HashSet<String>,
        struct_layouts: HashMap<String, StructLayout>,
        func_return_types: HashMap<String, String>,
    ) -> Self {
        let is_struct_return = struct_names.contains(ret);
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
                return_shape: if is_struct_return {
                    triet_mir::ReturnShape::Struct {
                        struct_name: ret.to_string(),
                    }
                } else {
                    triet_mir::ReturnShape::Scalar
                },
            },
            struct_names,
            struct_layouts,
            func_return_types,
            sret_ptr: None,
            owned_locals: Vec::new(),
            scope_snapshots: Vec::new(),
        };
        // If returning a struct, allocate the sret pointer as Local(0).
        // The JIT will receive this as a hidden first parameter.
        if is_struct_return {
            ctx.sret_ptr = Some(ctx.alloc_local_ty(ret));
        }
        ctx
    }

    fn is_struct_type(&self, name: &str) -> bool {
        self.struct_names.contains(name)
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
    /// owned local registered in it, in reverse allocation order.
    /// No-op if all scopes were already flushed by a `return`.
    fn pop_scope(&mut self) {
        let Some(snapshot) = self.scope_snapshots.pop() else {
            return;
        };
        for i in (snapshot..self.owned_locals.len()).rev() {
            self.push(Statement::Drop(self.owned_locals[i], DUMMY_SPAN));
        }
        self.owned_locals.truncate(snapshot);
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
pub fn lower_program(prog: &Program) -> Result<Vec<Body>, LowerError> {
    // ── Collect struct layouts from struct definitions ──────────
    // Bậc A: every field is 8 bytes, alignment 8 (single i64).
    // Bậc C will compute real sizes from type information.
    let struct_layouts: Vec<StructLayout> = prog
        .items
        .iter()
        .filter_map(|item| {
            if let Item::Struct { def } = &item.node {
                let fields: Vec<(String, usize, usize)> =
                    def.fields.iter().map(|f| (f.name.clone(), 8, 8)).collect();
                Some(StructLayout::compute(&def.name, &fields))
            } else {
                None
            }
        })
        .collect();

    let struct_names: std::collections::HashSet<String> =
        struct_layouts.iter().map(|l| l.name.clone()).collect();
    let struct_map: HashMap<String, StructLayout> = struct_layouts
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
                func_return_types.clone(),
            )?;
            body.struct_layouts = struct_layouts.clone();
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
    _span: Span,
    struct_names: std::collections::HashSet<String>,
    struct_layouts: HashMap<String, StructLayout>,
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
        func_return_types,
    );
    let entry = c.cur;

    // Function scope: Drop all owned locals (params + let bindings) when
    // the function exits. Must start before pushing parameters.
    c.push_scope();

    for p in &func.params {
        let ty = type_name(arena, p.type_annotation);
        let l = c.alloc_local_ty(&ty);
        c.vars.insert(p.name.clone(), l);
        c.push_owned(l);
        let passing = match p.passing_mode {
            triet_syntax::ParameterPassing::Borrow => ParameterPassing::Borrow,
            triet_syntax::ParameterPassing::Move => ParameterPassing::Move,
            triet_syntax::ParameterPassing::MutableBorrow => ParameterPassing::MutableBorrow,
        };
        c.sig.params.push((p.name.clone(), passing));
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
        _ => "?".to_string(),
    }
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

// ── Statement lowering ──────────────────────────────────────

fn lower_stmt(stmt: &Stmt, stmt_span: Span, arena: &Arena, c: &mut Ctx) -> Result<(), LowerError> {
    match stmt {
        Stmt::Let { name, init, .. } => {
            let v = lower_expr(*init, arena, c)?;
            c.vars.insert(name.clone(), v);
            c.push_owned(v);
        }
        Stmt::Expression { expr } => {
            lower_expr(*expr, arena, c)?;
        }
        Stmt::Return { value } => {
            if let (Some(sret), Some(v)) = (c.sret_ptr, value) {
                // Struct return via sret: copy struct fields to the caller's buffer.
                let struct_local = lower_expr(*v, arena, c)?;
                let source_ty = c.local_decls[struct_local.0].ty.clone();
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
                // Drop owned locals before Return, so the borrowck can
                // flag dangling references (E2450) on the return value.
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
                let mut values = Vec::new();
                if let Some(v) = value {
                    values.push(lower_expr(*v, arena, c)?);
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
        Expr::Identifier { name } => {
            let &local = c
                .vars
                .get(name)
                .ok_or_else(|| LowerError::undefined_local(name, expr_span))?;
            return Ok(local);
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
            let callee_name = match &arena.expression(*callee).node {
                Expr::Identifier { name } => name.clone(),
                other => return Err(LowerError::unsupported_callee(other, expr_span)),
            };
            let callee_ret = c
                .func_return_types
                .get(&callee_name)
                .cloned()
                .unwrap_or_else(|| "Integer".to_string());
            let is_struct_ret = c.is_struct_type(&callee_ret);

            let mut args: Vec<Local> = arguments
                .iter()
                .map(|a| lower_expr(*a, arena, c))
                .collect::<Result<Vec<_>, _>>()?;

            if is_struct_ret {
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
                Ok(ret_local)
            } else {
                let dest = c.alloc_local();
                c.push(Statement::StorageLive(dest, expr_span.clone()));
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
        Expr::Borrow { form, operand } => {
            let mir_form = lower_ref_form(*form);
            // The operand is an lvalue (IDENT or field-access chain per
            // ADR-0031 §2) — lower it to a projected Place so the borrow
            // checker can track the loan at field granularity.
            let source = lower_place(*operand, arena, c)?;
            let dest = c.alloc_local();
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
        Expr::FieldAccess { .. } => {
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
                c.push(Statement::Assign {
                    dest: Place::local(d).project(Projection::Field(field_name.clone())),
                    source: Place::local(field_val),
                    span: expr_span.clone(),
                });
            }
            Ok(d)
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
            std::collections::HashSet::new(),
            HashMap::new(),
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

    #[test]
    fn lowers_field_borrow_into_projected_place() {
        // `&0 obj.x` must lower to a Borrow whose source is the projected
        // Place { local: _0 (obj), projection: [Field("x")] }.
        let body =
            lower_source("function f(obj: Point) -> Integer { let r = &0 obj.x; return r; }");
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
}

//! The actual type-checker: walks a `Program`, accumulates `TypeError`s.

mod exprs;
mod methods;

use triet_syntax::{
    Arena, ExprId, FunctionBody, FunctionDefinition, Item, Pattern, PatternId, Program,
    ReferenceForm, Span, Spanned, Stmt, StmtId, TypeExpr, TypeId,
};

use crate::{
    env::TypeEnvironment,
    error::{BorrowError, TypeError},
    types::Type,
};

/// v0.9.x.atomic.7d: per-binding move-state for E2420
/// `UseAfterMove` enforcement per ADR-0025 §5.1 + ADR-0031 §4
/// Phương án A.
///
/// Tracks whether a local binding (function parameter or `let`-bound
/// name) is still owning its value (`Alive`) or has been consumed by
/// an owning-reference borrow expression (`Moved`). The `at` span
/// Type-check a `Program`, returning all errors found and enum variant
/// resolutions for the lowerer.
///
/// Returns an empty `Vec` on success. The checker accumulates errors
/// rather than aborting on the first one, so a single call can surface
/// every problem at once. `Type::Unknown` is used as a recovery
/// placeholder so cascading errors don't compound.
#[must_use]
pub fn check(
    program: &Program,
) -> (
    Vec<TypeError>,
    crate::ExprResolutions,
    crate::PatternResolutions,
) {
    let mut checker = Checker::new(program);
    // ADR-0061 T3.3: trait coherence — one `implement` per (Type, Trait).
    // The single-file path is what the driver runs (see triet-driver
    // integration_tests), so coherence must be enforced here too. Build a
    // name_table from this program's own declarations so user `for_type`s
    // resolve to their real names before keying the coherence check.
    let name_table: std::collections::HashMap<String, Type> =
        crate::check_resolved::collect_declared_types(
            &program.arena,
            &program.items,
            &std::collections::HashMap::new(),
        )
        .into_iter()
        .collect();
    let impl_keys =
        crate::check_resolved::collect_impl_keys(&program.arena, &program.items, &name_table);
    checker
        .errors
        .extend(crate::check_resolved::check_impl_coherence(impl_keys));
    // ADR-0061 T3.1/T3.2: build trait_table + impl_table and verify
    // conformance (E1044). Same single-file path the driver runs.
    let trait_table =
        crate::check_resolved::collect_trait_defs(&program.arena, &program.items, &name_table)
            .into_iter()
            .collect();
    let impl_table =
        crate::check_resolved::collect_impl_defs(&program.arena, &program.items, &name_table)
            .into_iter()
            .collect();
    checker
        .errors
        .extend(crate::check_resolved::check_conformance(
            &trait_table,
            &impl_table,
        ));
    checker.check_program();
    (
        checker.errors,
        checker.expr_resolutions,
        checker.pattern_resolutions,
    )
}

/// Type-check a `Program` with a pre-seeded [`TypeEnvironment`].
///
/// Import bindings from other modules are injected into the environment
/// before the declare/check passes. Used by `check_resolved` for
/// cross-module type checking.
pub(crate) fn check_with_env(program: &Program, env: TypeEnvironment) -> Vec<TypeError> {
    let mut checker = Checker::with_env(program, env);
    checker.check_program();
    checker.errors
}

/// Type-checker state.
struct Checker<'p> {
    arena: &'p Arena,
    items: &'p [Spanned<Item>],
    env: TypeEnvironment,
    /// The function whose body is currently being checked (for return-
    /// type enforcement). `None` at top level.
    current_return_type: Option<Type>,
    /// ADR-0061 T3.4: the concrete `for_type` of the `implement` block
    /// currently being checked, so `self` (`TypeExpr::SelfType`) resolves
    /// to it inside impl method bodies. `None` outside an impl context —
    /// a `self` there resolves to `Type::Unknown` (self out of place).
    current_self_type: Option<Type>,
    /// Local-context expected-type stack pushed by let/const annotations,
    /// struct-literal field positions, and call-argument positions per
    /// [v0.7.4.3-debt.3] (WA-5). Outcome constructors (`~0` especially)
    /// consult the TOP of this stack before falling back to
    /// `current_return_type` so a `let x: T? = ~0` binding inside a
    /// function returning `T~E` is accepted without firing E1025.
    expected_type_stack: Vec<Type>,
    /// v0.9.x.atomic.7d: per-function move state map per ADR-0025 §5.1.
    /// Reset on function entry; tracks local bindings (parameters + lets).
    /// Lookups for names NOT in the map are ignored (functions, types,
    /// v0.10.x.borrow.3: per-function set of names introduced by
    /// `let` bindings (NOT parameters). Used by E2403 enforcement to
    /// distinguish "weak ref to local owner" (escapes when returned)
    /// from "weak ref to parameter" (caller's owner outlives the
    /// function). Reset on function entry per [ADR-0025] §8.2.
    ///
    /// [ADR-0025]: ../../../docs/decisions/0025-borrow-checker-rules.md
    pub(crate) local_let_names: std::collections::HashSet<String>,
    /// Resolved enum variants keyed by expression ID. Populated during
    /// type inference; consumed by the lowerer (no re-scanning).
    pub(crate) expr_resolutions: crate::ExprResolutions,
    /// Resolved enum variants keyed by pattern ID. Populated during
    /// pattern binding; consumed by the lowerer.
    pub(crate) pattern_resolutions: crate::PatternResolutions,
    errors: Vec<TypeError>,
}

impl<'p> Checker<'p> {
    fn new(program: &'p Program) -> Self {
        Self {
            arena: &program.arena,
            items: &program.items,
            env: TypeEnvironment::with_prelude(),
            current_return_type: None,
            current_self_type: None,
            expected_type_stack: Vec::new(),
            local_let_names: std::collections::HashSet::new(),
            expr_resolutions: crate::ExprResolutions::new(),
            pattern_resolutions: crate::PatternResolutions::new(),
            errors: Vec::new(),
        }
    }

    /// Create a checker with a pre-built environment. Imported names
    /// are already declared in `env` before the declare pass runs.
    fn with_env(program: &'p Program, env: TypeEnvironment) -> Self {
        Self {
            arena: &program.arena,
            items: &program.items,
            env,
            current_return_type: None,
            current_self_type: None,
            expected_type_stack: Vec::new(),
            // B2.1a DELETED:             move_states: HashMap::new(),
            local_let_names: std::collections::HashSet::new(),
            expr_resolutions: crate::ExprResolutions::new(),
            pattern_resolutions: crate::PatternResolutions::new(),
            errors: Vec::new(),
        }
    }

    /// Run `body` with `expected` pushed onto the expected-type stack;
    /// pop on the way out. Mirrors RAII-style scope handling.
    fn with_expected<R>(&mut self, expected: Type, body: impl FnOnce(&mut Self) -> R) -> R {
        self.expected_type_stack.push(expected);
        let result = body(self);
        self.expected_type_stack.pop();
        result
    }

    // =======================================================================
    // v0.9.x.atomic.7d — E2420 UseAfterMove enforcement per ADR-0025 §5.1 +
    // ADR-0031 §4 Phương án A. Tracks local binding ownership state per
    // function body; fires E2420 when a moved binding is used. Branch
    // semantics: snapshot/restore/join with "any-branch-moves => moved"
    // (over-strict; NLL refinement defers v0.10 per ADR-0031 §10.1).
    // =======================================================================

    /// Mark a local binding as moved, recording the move-site span.
    /// No-op when the name isn't a tracked local (functions, types,
    /// Check whether using a name at a given span is valid. Fires
    /// E2420 `UseAfterMove` if the binding is currently in `Moved`
    /// Walk an expression node looking for the **base identifier** of
    /// an operand chain. `Expr::Identifier(name)` → `name`;
    /// `Expr::FieldAccess { object, .. }` → recurse into `object`.
    /// Any other expression form → `None` (operand grammar per
    /// Snapshot the current move-state map. Used by branch-aware
    /// constructs (`if` / `match` / loop bodies) to evaluate each
    /// Join two branch-end move-state maps with **any-branch-moves**
    /// semantics per ADR-0031 §4 over-strict approach: a binding is
    /// `Moved` in the join iff it's `Moved` in either branch. Span
    /// from the first-seen move wins (informational only).
    ///
    /// This is conservative — rejects code that NLL would accept
    /// (one branch moves, other doesn't, join point only reachable
    fn check_program(&mut self) {
        // Pass 1: register every top-level function/const so calls and
        // references can resolve forward.
        for item in self.items {
            self.declare_item(item);
        }
        // Pass 2: check bodies.
        for item in self.items {
            self.check_item(item);
        }
    }

    // ====================================================================
    // Items
    // ====================================================================

    fn declare_item(&mut self, item: &Spanned<Item>) {
        match &item.node {
            Item::Function { def } => {
                // Push a frame so generic type parameters are visible
                // during parameter/return type resolution (mirror
                // struct/enum below).
                self.env.push_frame();
                for param in &def.type_parameters {
                    self.env
                        .declare(&param.name, Type::TypeParameter(param.name.clone()));
                }
                let parameters: Vec<Type> = def
                    .parameters
                    .iter()
                    .map(|p| self.resolve_type(p.type_annotation))
                    .collect();
                let return_type = def
                    .return_type
                    .map_or(Type::Unit, |id| self.resolve_type(id));
                self.env.pop_frame();
                let function_type = Type::Function {
                    type_parameters: def.type_parameters.clone(),
                    parameters,
                    return_type: Box::new(return_type),
                };
                self.declare_or_record_dup(&def.name, function_type, item.span.clone());
            }
            Item::Constant {
                name,
                type_annotation,
                value,
                ..
            } => {
                let declared = type_annotation.map(|id| self.resolve_type(id));
                let inferred = self.infer_expression(*value);
                let ty = match declared {
                    Some(annotated) => {
                        if !annotated.matches(&inferred) {
                            self.errors.push(TypeError::Mismatch {
                                expected: annotated.clone(),
                                found: inferred,
                                span: self.arena.expression(*value).span.clone(),
                            });
                        }
                        annotated
                    }
                    None => inferred,
                };
                self.declare_or_record_dup(name, ty, item.span.clone());
            }
            Item::TypeAlias { .. } => {
                // V0.1: type aliases are accepted syntactically but the
                // checker does not yet expand them. Names registered in
                // declare_or_record_dup are not used as type names.
            }
            Item::Import { .. } | Item::ImportFrom { .. } => {
                // Imports are syntactic placeholders until the module
                // loader (v0.2.x.6) ships. Names introduced by `import`
                // / `from … import …` are not yet bound here.
            }
            Item::Module { .. } => {
                // Module declarations are not yet checked; the module
                // loader (v0.2.x.6) will recurse into inline content
                // and resolve external file-bound modules.
            }
            Item::Struct { def } => {
                // Push a frame where type parameters are visible during
                // field type resolution.
                self.env.push_frame();
                for param in &def.type_parameters {
                    self.env
                        .declare(&param.name, Type::TypeParameter(param.name.clone()));
                }
                let fields: Vec<(String, Type)> = def
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), self.resolve_type(f.type_annotation)))
                    .collect();
                self.env.pop_frame();
                let ty = Type::UserStruct {
                    name: def.name.clone(),
                    type_parameters: def.type_parameters.clone(),
                    fields,
                };
                self.declare_or_record_dup(&def.name, ty, item.span.clone());
            }
            Item::Enum { def } => {
                self.env.push_frame();
                for param in &def.type_parameters {
                    self.env
                        .declare(&param.name, Type::TypeParameter(param.name.clone()));
                }
                let variants: Vec<(String, Option<Box<Type>>)> = def
                    .variants
                    .iter()
                    .map(|v| {
                        let payload = v.payload.map(|tid| Box::new(self.resolve_type(tid)));
                        (v.name.clone(), payload)
                    })
                    .collect();
                self.env.pop_frame();
                let ty = Type::UserEnum {
                    name: def.name.clone(),
                    type_parameters: def.type_parameters.clone(),
                    variants,
                };
                self.declare_or_record_dup(&def.name, ty, item.span.clone());
            }
            Item::Trait { .. } | Item::Implementation { .. } => {
                // ADR-0061 Tier 1: trait/implement declaration + dispatch
                // routing land in T3/T4 (trait_table / impl_table). The parser
                // does not yet emit these variants (T2), so this arm is
                // currently inert — it exists for match exhaustiveness during T1.
            }
        }
    }

    fn check_item(&mut self, item: &Spanned<Item>) {
        match &item.node {
            Item::Function { def } => self.check_function(def),
            // ADR-0061 T3.4: typecheck each `implement` method body with
            // `self` bound to the concrete `for_type`. Conformance of the
            // signatures themselves (E1044) is verified separately by the
            // global `check_conformance` pass; here we only check bodies.
            Item::Implementation { def } => {
                let self_ty = self.resolve_type(def.for_type);
                let saved = self.current_self_type.replace(self_ty);
                for method in &def.methods {
                    self.check_function(method);
                }
                self.current_self_type = saved;
            }
            // Struct / Enum definitions have no runtime body to check
            // (field types are resolved during declaration).
            _ => {}
        }
    }

    fn check_function(&mut self, def: &FunctionDefinition) {
        // v0.9.x.atomic.7d: save/restore move state across function
        // boundary — each function body has its own move-tracking map.
        // v0.10.x.borrow.3: save/restore local-let-name set per
        // function for E2403 escape detection (ADR-0025 §8.2). The set
        // is populated by Stmt::Let; parameters are NOT in it (they
        // come from caller's owner trail).
        let saved_local_lets = std::mem::take(&mut self.local_let_names);
        // Push a frame so type parameters are visible during type
        // resolution of parameters + return type. Reused as the
        // function body's scope (parameters live in same frame).
        self.env.push_frame();
        // Declare generic type parameters first so `resolve_type` sees
        // them as `TypeParameter(name)` rather than `Unknown` (v0.7.4.1,
        // ADR-0019 Addendum §A7, Q2-A).
        for param in &def.type_parameters {
            self.env
                .declare(&param.name, Type::TypeParameter(param.name.clone()));
        }

        let return_type = def
            .return_type
            .map_or(Type::Unit, |id| self.resolve_type(id));

        self.current_return_type = Some(return_type.clone());

        // ADR-0046 §1: whitelist `-> &0 T` return-borrow; refuse
        // `-> &+ T` / `-> &0 mutable T` / `-> &- T` (deferred).
        if let Type::Reference(form, _) = &return_type
            && *form != ReferenceForm::BorrowReadOnly
        {
            let return_ty_str = return_type.to_string();
            let span = def
                .return_type
                .map(|id| self.arena.type_expression(id).span.clone())
                .unwrap_or(0..0);
            self.errors.push(TypeError::BorrowReturnNotYetSupported {
                return_ty: return_ty_str,
                span,
            });
        }

        for parameter in &def.parameters {
            let ty = self.resolve_type(parameter.type_annotation);
            self.env.declare(&parameter.name, ty);
            // v0.9.x.atomic.7d: track parameter as Alive at entry.
            // Any function body move site updates this map.
            // B2.1a DELETED:             self.move_states
        }

        // v0.10.x.borrow.2 (ADR-0025 §3): lifetime elision check.
        // Property of the signature, not the body — evaluate before
        // descending into the body so failure messages aren't masked
        // by downstream errors. Conservative top-level scope per the
        // sub-task plan: nested borrows in return type (e.g.,
        // `Vector<&0 T>`, `(&0 T, X)`) defer v0.11+ corpus-driven.
        self.check_lifetime_elision(def, &return_type);

        // E2440: moved to MIR NLL borrowck (ADR-0051 B2.1b).

        match &def.body {
            FunctionBody::Block { block } => {
                // Block-form body is an `Expr::Block` node; infer it
                // through the expression checker.
                let body_ty = self.infer_expression(*block);
                if !return_type.matches(&body_ty) {
                    let span = self.arena.expression(*block).span.clone();
                    self.push_return_mismatch(&return_type, &body_ty, span);
                }
                // v0.10.x.borrow.3: block-form body's final expression
                // is the function's return value — check for E2403.
                // Inner `Stmt::Return` arms already check themselves.
                let final_expr = match &self.arena.expression(*block).node {
                    triet_syntax::Expr::Block { final_expr, .. } => *final_expr,
                    _ => None,
                };
                if let Some(final_expr) = final_expr {
                    self.check_escaping_weak_borrow(final_expr);
                }
            }
            FunctionBody::Expression { expr } => {
                let body_ty = self.infer_expression(*expr);
                if !return_type.matches(&body_ty) {
                    let span = self.arena.expression(*expr).span.clone();
                    self.push_return_mismatch(&return_type, &body_ty, span);
                }
                // v0.10.x.borrow.3: expression-form body IS the
                // function's return value — check for E2403.
                self.check_escaping_weak_borrow(*expr);
            }
            FunctionBody::External { .. } => {}
        }

        self.current_return_type = None;
        self.env.pop_frame();
        // v0.9.x.atomic.7d: restore move state from caller's frame.
        // v0.10.x.borrow.3: restore local-let-name set.
        self.local_let_names = saved_local_lets;
    }

    /// v0.10.x.borrow.2 — Lifetime elision per [ADR-0025 §3].
    ///
    /// Fires `E2400 BorrowLifetimeInferenceFailed` when:
    ///
    /// 1. Return type is a **top-level borrow** (`&0 T`, `&0 mutable T`,
    ///    or `&- T`), AND
    /// 2. Rule 1 fails: function has != 1 input borrow parameter, AND
    /// 3. Rule 2 fails: function does not have a borrow `self` receiver
    ///    as its first parameter, AND
    /// 4. Rule 3 is already excluded by the borrow-return guard (owned
    ///    `&+` returns transfer ownership; no elision needed).
    ///
    /// **Conservative scope:** only top-level borrow at return position
    /// is checked. Nested borrows inside generic containers (e.g.,
    /// `Vector<&0 T>`, `(&0 T, X)`, `T~&0 E`) defer v0.11+ per the
    /// refuse-over-guess principle ([VISION §6](../../../../VISION.md)).
    ///
    /// [ADR-0025 §3]: ../../../docs/decisions/0025-borrow-checker-rules.md
    fn check_lifetime_elision(&mut self, def: &FunctionDefinition, return_type: &Type) {
        // Step 1: only check when the return is a top-level borrow.
        // ReferenceForm partitions into two groups: owning (`&+` /
        // `&+ mutable`, via `is_owning()`) vs borrow (everything else —
        // `&0`, `&0 mutable`, `&-`). Owning returns are Rule 3 — no
        // inference needed; the function transfers ownership out.
        let Type::Reference(return_form, return_inner) = return_type else {
            return;
        };
        if return_form.is_owning() {
            return;
        }

        // Step 2: classify each parameter. Count input borrows; remember
        // whether the first parameter is a borrow `self` receiver
        // (Rule 2 trigger). ADR-0025 §3.2 uses `&0 self` / `&0 mutable
        // self` in the canonical example; `&-` self is a borrow receiver
        // by the same logic (output ties to self).
        //
        // Rule 2 is **dormant** as of v0.10.x.borrow.2 — the parser
        // refuses `self` as a parameter name (SelfKw is reserved). The
        // branch stays in place so Rule 2 lights up automatically once
        // the parser accepts `self`-parameter syntax.
        let mut input_borrow_count: usize = 0;
        let mut has_self_borrow_receiver = false;
        for (i, parameter) in def.parameters.iter().enumerate() {
            let param_ty = self.resolve_type(parameter.type_annotation);
            if let Type::Reference(form, _) = &param_ty
                && !form.is_owning()
            {
                input_borrow_count += 1;
                if i == 0 && parameter.name == "self" {
                    has_self_borrow_receiver = true;
                }
            }
        }

        // Step 3: apply elision rules in order.
        // Rule 2: self receiver wins regardless of other borrow count.
        if has_self_borrow_receiver {
            return;
        }
        // Rule 1: exactly one input borrow → output ties to it.
        if input_borrow_count == 1 {
            return;
        }
        // Both rules failed; the return borrow has no unambiguous tie.
        // Span: the return type annotation, falling back to the function
        // name span if the annotation id is None (single-expression body
        // with inferred return — rare at v0.10).
        let span = def
            .return_type
            .map_or(0..0, |id| self.arena.type_expression(id).span.clone());
        let ty_str = format!("{return_inner}");
        self.errors.push(TypeError::Borrow(
            BorrowError::BorrowLifetimeInferenceFailed { ty: ty_str, span },
        ));
    }

    // ====================================================================
    // Statements / blocks
    // ====================================================================

    fn check_block(&mut self, statements: &[StmtId], final_expression: Option<ExprId>) -> Type {
        self.env.push_frame();
        let mut has_return = false;
        for stmt_id in statements {
            // Peek at statement node before checking — if the block
            // contains a `return`, the block's final type is irrelevant
            // (all paths diverge). Use `Type::Unknown` so the function-
            // level return-type check doesn't fire spuriously.
            if matches!(
                self.arena.statement(*stmt_id).node,
                Stmt::Return { .. } | Stmt::Break | Stmt::Continue
            ) {
                has_return = true;
            }
            self.check_statement(*stmt_id);
        }
        let value_type = if has_return && final_expression.is_none() {
            // Block ends with return/break/continue — diverges.
            // Never is the bottom type: compatible with any expected
            // return type. Individual return statements are already
            // type-checked against the declared return type.
            Type::Never
        } else {
            final_expression.map_or(Type::Unit, |id| self.infer_expression(id))
        };
        self.env.pop_frame();
        value_type
    }

    fn check_statement(&mut self, id: StmtId) {
        let stmt = self.arena.statement(id).clone();
        match stmt.node {
            Stmt::Let {
                name,
                is_mutable,
                type_annotation,
                init,
            } => {
                let ty = self.check_initializer(type_annotation, init);
                self.env.declare_with_mut(&name, ty, is_mutable);
                // v0.9.x.atomic.7d: new local binding starts Alive.
                // B2.1a DELETED:                 self.move_states.insert(name.clone(), MoveState::Alive);
                // v0.10.x.borrow.3: track as local-let (NOT parameter)
                // for E2403 escape detection (ADR-0025 §8.2).
                self.local_let_names.insert(name.clone());
            }
            Stmt::Assignment { target, value } => {
                self.check_assignment(target, value, stmt.span.clone());
            }
            Stmt::Const {
                name,
                type_annotation,
                value,
            } => {
                let ty = self.check_initializer(type_annotation, value);
                self.env.declare(&name, ty);
            }
            Stmt::Return { value } => {
                let actual = value.map_or(Type::Unit, |id| self.infer_expression(id));
                if let Some(expected) = self.current_return_type.clone()
                    && !expected.matches(&actual)
                {
                    let span = value.map_or(stmt.span.clone(), |id| {
                        self.arena.expression(id).span.clone()
                    });
                    self.errors.push(TypeError::Mismatch {
                        expected,
                        found: actual,
                        span,
                    });
                }
                // v0.10.x.borrow.3: E2403 — `return &- local` escapes.
                if let Some(id) = value {
                    self.check_escaping_weak_borrow(id);
                }
            }
            Stmt::Break => {
                // `Stmt::Break` is a unit variant in the schema AST —
                // break-with-value is not modeled.
            }
            Stmt::Continue => {}
            Stmt::For {
                variable,
                iterable,
                body,
            } => {
                let iter_ty = self.infer_expression(iterable);
                let element_ty = match &iter_ty {
                    Type::Range(inner) => (**inner).clone(),
                    _ => Type::Unknown,
                };
                self.env.push_frame();
                self.bind_pattern(variable, &element_ty);
                // v0.9.x.atomic.7d: for-body may iterate 0 or N times;
                // same join semantics as while/loop.
                let _ = self.infer_expression(body);
                self.env.pop_frame();
            }
            Stmt::While {
                condition,
                body,
                treat_unknown_as_false,
            } => {
                let cond_ty = self.infer_expression(condition);
                let cond_span = self.arena.expression(condition).span.clone();
                self.check_condition_type(cond_ty, treat_unknown_as_false, cond_span);
                // v0.9.x.atomic.7d: loop body may run 0 or N times.
                // Snapshot before; walk body; join initial with
                // after-body to model "didn't enter" vs "ran ≥1 time".
                let _ = self.infer_expression(body);
            }
            Stmt::Loop { body } => {
                // Same as while — body may not run (`loop { break }`).
                let _ = self.infer_expression(body);
            }
            Stmt::Expression { expr } => {
                let _ = self.infer_expression(expr);
            }
        }
    }

    /// Shared logic for `let` / `const` initializers: resolve the
    /// optional annotation, infer the value, and verify they agree.
    /// Returns the binding's final type (annotation if present, else
    /// inferred). On mismatch, records a `Mismatch` error and falls
    /// back to the annotated type for downstream checking.
    fn check_initializer(&mut self, type_annotation: Option<TypeId>, value: ExprId) -> Type {
        let declared = type_annotation.map(|tid| self.resolve_type(tid));
        // v0.7.4.3-debt.3 (WA-5): when the binding has an explicit
        // type annotation, push it as the local expected type while
        // checking the initializer. Outcome constructors (`~0` in
        // particular) consult this stack first — letting `let x: T?
        // = ~0` succeed inside a function returning `T~E` instead of
        // false-positive E1025.
        let inferred = if let Some(expected) = declared.clone() {
            self.with_expected(expected, |s| s.infer_expression(value))
        } else {
            self.infer_expression(value)
        };
        match declared {
            Some(annotated) => {
                if !annotated.matches(&inferred) {
                    self.push_initializer_mismatch(
                        &annotated,
                        &inferred,
                        self.arena.expression(value).span.clone(),
                    );
                }
                annotated
            }
            None => inferred,
        }
    }

    fn check_assignment(&mut self, target: ExprId, value: ExprId, stmt_span: Span) {
        let value_ty = self.infer_expression(value);
        let value_span = self.arena.expression(value).span.clone();
        // `Stmt::Assignment` stores the target as an `ExprId`; only
        // identifier targets are assignable (parser-enforced).
        let name = match &self.arena.expression(target).node {
            triet_syntax::Expr::Identifier { name } => name.clone(),
            _ => return,
        };
        let Some(binding) = self.env.lookup_binding(&name).cloned() else {
            self.errors.push(TypeError::UndefinedName {
                name,
                span: stmt_span,
            });
            return;
        };
        if !binding.mutable {
            self.errors.push(TypeError::AssignToImmutable {
                name,
                span: stmt_span,
            });
        }
        if !binding.ty.matches(&value_ty) {
            self.errors.push(TypeError::Mismatch {
                expected: binding.ty,
                found: value_ty,
                span: value_span,
            });
        }
    }

    /// Specialized return-type mismatch: when the declared type is
    /// `Trilean!` and the body produces generic `Trilean`, raise
    /// E1034 `TrileanReturnNotRefined` per [ADR-0021] §2.7 — the
    /// narrowing-direction error has its own diagnostic with help
    /// text about `.assume_known()` and refactoring. Other mismatches
    /// fall through to the generic E1003 Mismatch.
    fn push_return_mismatch(&mut self, expected: &Type, found: &Type, span: Span) {
        if matches!(expected, Type::Trilean { refined: true })
            && matches!(found, Type::Trilean { refined: false })
        {
            self.errors
                .push(TypeError::TrileanReturnNotRefined { span });
        } else {
            self.errors.push(TypeError::Mismatch {
                expected: expected.clone(),
                found: found.clone(),
                span,
            });
        }
    }

    /// Specialized let/const initializer mismatch — reroutes the
    /// frozen-to-mutable promotion pattern (`&+ T` → `&+ mutable T`)
    /// to `E2411 CannotPromoteFrozenToMutable` per [ADR-0025] §7.2.
    /// Other mismatches fall through to the generic E1003.
    ///
    /// [ADR-0025]: ../../../docs/decisions/0025-borrow-checker-rules.md
    fn push_initializer_mismatch(&mut self, expected: &Type, found: &Type, span: Span) {
        if let (
            Type::Reference(expected_form, expected_inner),
            Type::Reference(found_form, found_inner),
        ) = (expected, found)
            && *expected_form == triet_syntax::ReferenceForm::StrongMutable
            && *found_form == triet_syntax::ReferenceForm::StrongFrozen
            && expected_inner.matches(found_inner)
        {
            self.errors.push(TypeError::Borrow(
                BorrowError::CannotPromoteFrozenToMutable {
                    ty: format!("{expected_inner}"),
                    span,
                },
            ));
            return;
        }
        self.errors.push(TypeError::Mismatch {
            expected: expected.clone(),
            found: found.clone(),
            span,
        });
    }

    /// v0.10.x.borrow.3 — Detect direct `return &- local` pattern per
    /// [ADR-0025] §8.2 (E2403 `WeakRefOutlivesOwner`).
    ///
    /// Conservative scope: only fires when the expression at a function
    /// return position is `Expr::Borrow { form: WeakObserver, operand }`
    /// AND `operand`'s base identifier was introduced by `let` in the
    /// current function body (NOT a parameter, NOT a module-level item).
    /// Parameters' owner trail extends to the caller's scope, so
    /// `return &- param` is allowed.
    ///
    /// Full owner-trail tracking (assign-to-outer-scope, struct-field
    /// store, multi-hop through function calls) defers v0.11+ per
    /// §8.3 algorithm — refuse-over-guess.
    ///
    /// [ADR-0025]: ../../../docs/decisions/0025-borrow-checker-rules.md
    fn check_escaping_weak_borrow(&mut self, expr_id: ExprId) {
        let expr = self.arena.expression(expr_id).clone();
        if let triet_syntax::Expr::Borrow { form, operand } = expr.node
            && form == triet_syntax::ReferenceForm::WeakObserver
            && let triet_syntax::Expr::Identifier { name } = &self.arena.expression(operand).node
            && self.local_let_names.contains(name)
        {
            self.errors
                .push(TypeError::Borrow(BorrowError::EscapingBorrow {
                    span: expr.span,
                }));
        }
    }

    fn check_condition_type(&mut self, cond_ty: Type, treat_unknown_as_false: bool, span: Span) {
        match cond_ty {
            Type::Unknown => { /* recovery path — earlier error suppresses */ }
            Type::Trilean { refined: true } => { /* OK — Trilean! is plain-`if` safe */ }
            Type::Trilean { refined: false } => {
                // ADR-0021 §3: plain `if cond` rejects generic Trilean
                // at compile time. `if?` form sets treat_unknown_as_false
                // so this raise is suppressed for the relaxed `if?` /
                // `while?` / match-guard contexts.
                if !treat_unknown_as_false {
                    self.errors
                        .push(TypeError::PossiblyUnknownCondition { span });
                }
            }
            other => {
                self.errors
                    .push(TypeError::NonTrileanCondition { found: other, span });
            }
        }
    }

    // ====================================================================
    // Patterns
    // ====================================================================

    fn bind_pattern(&mut self, id: PatternId, scrutinee: &Type) {
        let pattern = self.arena.pattern(id).node.clone();
        match pattern {
            Pattern::Wildcard | Pattern::Null => {}
            Pattern::Variable(name) => {
                // If the scrutinee is an enum type and the variable name
                // matches a unit variant, record the resolution for the
                // lowerer (the pattern binds nothing — it's a match arm
                // for a unit variant, not a variable capture).
                if let Type::UserEnum {
                    name: enum_name,
                    variants,
                    ..
                } = scrutinee
                    && let Some(variant_idx) =
                        variants.iter().position(|(n, p)| n == &name && p.is_none())
                {
                    self.pattern_resolutions.insert(
                        id,
                        crate::EnumVariantResolution {
                            enum_name: enum_name.clone(),
                            variant_name: name.clone(),
                            // Invariant: variant_idx is bounded by enum definition limits (far below i64::MAX)
                            #[allow(clippy::cast_possible_wrap)]
                            discriminant: variant_idx as i64,
                            has_payload: false,
                        },
                    );
                    // Don't declare as a variable — it's a unit variant match.
                    return;
                }
                self.env.declare(&name, scrutinee.clone());
            }
            Pattern::Tuple(children) => {
                if let Type::Tuple(elements) = scrutinee {
                    for (child, element_type) in children.iter().zip(elements.iter()) {
                        self.bind_pattern(*child, element_type);
                    }
                } else {
                    for child in children {
                        self.bind_pattern(child, &Type::Unknown);
                    }
                }
            }
            Pattern::Or(alternatives) => {
                // Each alternative shares the same scrutinee shape; we
                // only bind from the first to avoid binding the same
                // variable to potentially differing types.
                if let Some(first) = alternatives.first() {
                    self.bind_pattern(*first, scrutinee);
                }
            }
            Pattern::Range { .. } | Pattern::Literal(_) => {}
            Pattern::EnumVariant {
                variant_name,
                payload,
                ..
            } => {
                if let Type::UserEnum {
                    name: enum_name,
                    variants,
                    ..
                } = scrutinee
                {
                    // Record the resolution for the lowerer — pattern ID
                    // maps to (enum_name, variant_name, discriminant, has_payload).
                    if let Some(variant_idx) = variants.iter().position(|(n, _)| n == &variant_name)
                    {
                        let has_payload = variants[variant_idx].1.is_some();
                        self.pattern_resolutions.insert(
                            id,
                            crate::EnumVariantResolution {
                                enum_name: enum_name.clone(),
                                variant_name: variant_name.clone(),
                                // Invariant: variant_idx is bounded by enum definition limits (far below i64::MAX)
                                #[allow(clippy::cast_possible_wrap)]
                                discriminant: variant_idx as i64,
                                has_payload,
                            },
                        );
                    }
                    if let Some((_, def_payload)) = variants
                        .iter()
                        .find(|(n, _)| n.as_str() == variant_name.as_str())
                        && let (Some(sub_pattern), Some(payload_ty)) = (payload, def_payload)
                    {
                        self.bind_pattern(sub_pattern, payload_ty);
                    }
                }
            }
            // v0.7.4.3-error.2 (ADR-0020 §5): outcome arm patterns.
            // Bind payload sub-pattern to the appropriate inner type:
            // - ~+ binding → bind to value_type
            // - ~- binding → bind to error_type
            // - ~0 → no binding (no payload)
            // For T?~E patterns where scrutinee is `T?` (nullable),
            // we synthesize an Outcome shape for binding purposes —
            // ADR-0020 §10.4 unifies these contexts.
            Pattern::OutcomeArm { arm, payload } => {
                use triet_syntax::OutcomeArm as Arm;
                if let Some(sub) = payload {
                    let inner_ty = match (&arm, scrutinee) {
                        (Arm::Positive, Type::Outcome { value_type, .. }) => (**value_type).clone(),
                        (Arm::Negative, Type::Outcome { error_type, .. }) => (**error_type).clone(),
                        // For nullable scrutinee with ~+ pattern, bind
                        // to the wrapped type (ADR-0020 §10.4 unified
                        // pattern semantics across T? and T?~E).
                        (Arm::Positive, Type::Nullable(inner)) => (**inner).clone(),
                        _ => Type::Unknown,
                    };
                    self.bind_pattern(sub, &inner_ty);
                }
                // `~0` has no payload; nothing to bind.
            }
        }
    }

    // ====================================================================
    // Type-expression resolution + helpers
    // ====================================================================

    #[allow(clippy::too_many_lines)]
    fn resolve_type(&mut self, id: TypeId) -> Type {
        let span = self.arena.type_expression(id).span.clone();
        match self.arena.type_expression(id).node.clone() {
            TypeExpr::Named(name) => match name.as_str() {
                "Trit" => Type::Trit,
                "Tryte" => Type::Tryte,
                "Integer" => Type::Integer,
                "Long" => Type::Long,
                "Trilean" => Type::TRILEAN,
                "String" => Type::String,
                "Unit" => Type::Unit,
                _ => {
                    // Look up user-defined types, type parameters, or aliases.
                    if let Some(ty) = self.env.lookup(&name).cloned() {
                        match &ty {
                            Type::UserStruct { .. }
                            | Type::UserEnum { .. }
                            | Type::TypeParameter(_) => ty,
                            _ => {
                                self.errors.push(TypeError::UnknownType { name, span });
                                Type::Unknown
                            }
                        }
                    } else {
                        self.errors.push(TypeError::UnknownType { name, span });
                        Type::Unknown
                    }
                }
            },
            TypeExpr::Generic { name, arguments } => {
                // Monomorphize: `Option<Integer>` → substitute `T→Integer`.
                let args: Vec<Type> = arguments.iter().map(|t| self.resolve_type(*t)).collect();

                // v0.7.4.2 (ADR-0019 Addendum §A7): `Vector<T>` and
                // `HashMap<K, V>` are built-in collection types
                // surfaced as pseudo user-struct shells for
                // typecheck purposes — IR carries the concrete
                // `TypeTag::Vector`/`TypeTag::HashMap` variants
                // (locked v0.7.3.1). Existing struct/enum
                // monomorphization machinery applies uniformly.
                match (name.as_str(), args.len()) {
                    ("Atomic", 1) => {
                        // v0.9.x.atomic.1 — enforce AtomicValue membership per
                        // ADR-0028 §2. Reject non-primitive payloads at typecheck.
                        let inner = args.into_iter().next().unwrap();
                        if !inner.is_atomic_value() {
                            self.errors.push(TypeError::NonAtomicValueType {
                                ty: format!("{inner}"),
                                span,
                            });
                            return Type::Unknown;
                        }
                        return Type::Atomic(Box::new(inner));
                    }
                    ("Vector", 1) => {
                        return Type::Vector(Box::new(args.into_iter().next().unwrap()));
                    }
                    ("HashMap", 2) => {
                        let mut iter = args.into_iter();
                        let key = iter.next().unwrap();
                        let value = iter.next().unwrap();
                        return Type::UserStruct {
                            name: "HashMap".into(),
                            type_parameters: Vec::new(),
                            fields: vec![("__key".into(), key), ("__value".into(), value)],
                        };
                    }
                    ("Vector", n) => {
                        self.errors.push(TypeError::WrongArity {
                            expected: 1,
                            found: n,
                            span,
                        });
                        return Type::Unknown;
                    }
                    ("HashMap", n) => {
                        self.errors.push(TypeError::WrongArity {
                            expected: 2,
                            found: n,
                            span,
                        });
                        return Type::Unknown;
                    }
                    _ => {}
                }

                if let Some(ty) = self.env.lookup(&name).cloned() {
                    match &ty {
                        Type::UserStruct {
                            type_parameters, ..
                        }
                        | Type::UserEnum {
                            type_parameters, ..
                        } => {
                            if type_parameters.len() != args.len() {
                                self.errors.push(TypeError::WrongArity {
                                    expected: type_parameters.len(),
                                    found: args.len(),
                                    span,
                                });
                                return Type::Unknown;
                            }
                            let map: std::collections::HashMap<_, _> = type_parameters
                                .iter()
                                .map(|p| p.name.clone())
                                .zip(args.iter().cloned())
                                .collect();
                            for tp in type_parameters {
                                if matches!(tp.bound, Some(triet_syntax::GenericBound::Send))
                                    && let Some(arg_ty) = map.get(&tp.name)
                                    && !arg_ty.is_send()
                                {
                                    self.errors.push(crate::error::TypeError::Concurrency(
                                                crate::error::ConcurrencyError::NotSendCannotCrossBoundary {
                                                    ty: arg_ty.to_string(),
                                                    span: span.clone(),
                                                }
                                            ));
                                }
                            }
                            return ty.substitute(&map);
                        }
                        // Non-struct types cannot have type parameters — fall through to UnknownType.
                        _ => {}
                    }
                }
                self.errors.push(TypeError::UnknownType { name, span });
                Type::Unknown
            }
            TypeExpr::Tuple(elements) => {
                Type::Tuple(elements.iter().map(|t| self.resolve_type(*t)).collect())
            }
            TypeExpr::Nullable(inner) => Type::Nullable(Box::new(self.resolve_type(inner))),
            TypeExpr::Function {
                parameters,
                return_type,
            } => Type::Function {
                // Function-type literal expressions (e.g., closure
                // type annotations) don't carry type parameters — those
                // are owned by function definitions, not function
                // types as values.
                type_parameters: Vec::new(),
                parameters: parameters.iter().map(|t| self.resolve_type(*t)).collect(),
                return_type: Box::new(self.resolve_type(return_type)),
            },
            // v0.7.4.3-error.2 (ADR-0020 §1): outcome type expressions
            // resolve to `Type::Outcome` proper. Reject nullable error
            // type per ADR-0020 §1.4 (E1024).
            TypeExpr::Outcome {
                value_type,
                error_type,
                allow_null_state,
            } => {
                let v_ty = self.resolve_type(value_type);
                let e_ty = self.resolve_type(error_type);
                // E1024: error type cannot itself be nullable.
                if matches!(e_ty, Type::Nullable(_)) {
                    self.errors
                        .push(TypeError::NullableErrorInOutcomeType { span });
                    return Type::Unknown;
                }
                Type::Outcome {
                    value_type: Box::new(v_ty),
                    error_type: Box::new(e_ty),
                    allow_null_state,
                }
            }
            // v0.7.4.3-debt.1: `Trilean!` annotation per ADR-0021 §2.7.
            TypeExpr::RefinedTrilean => Type::TRILEAN_KNOWN,
            // v0.8: reference forms. Enforcement deferred to v0.9+;
            // resolve transparently to the inner type for now.
            TypeExpr::Reference { form, inner } => {
                Type::Reference(form, Box::new(self.resolve_type(inner)))
            }
            // ADR-0061 T3.4: resolve `Self` → the `implement` block's
            // concrete `for_type`. Inside an impl method body, `self` is
            // typed as that receiver type; outside any impl context
            // (`current_self_type` is None) a stray `self` falls back to
            // Unknown. Reachable via the `self` parameter (T2.4).
            TypeExpr::SelfType => self.current_self_type.clone().unwrap_or(Type::Unknown),
        }
    }

    fn declare_or_record_dup(&mut self, name: &str, ty: Type, span: Span) {
        if !self.env.declare(name, ty) {
            self.errors.push(TypeError::DuplicateName {
                name: name.to_owned(),
                span,
            });
        }
    }
}

// ====================================================================
// Free helpers
// ====================================================================

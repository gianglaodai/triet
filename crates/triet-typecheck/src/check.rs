//! The actual type-checker: walks a `Program`, accumulates `TypeError`s.

mod exprs;
mod methods;

use triet_syntax::{
    Arena, Block, ExprId, FunctionBody, FunctionDef, Item, Pattern, PatternId, Program, Span,
    Spanned, Stmt, StmtId, TypeExpr, TypeId,
};

use crate::{env::TypeEnvironment, error::TypeError, types::Type};

/// Type-check a `Program`, returning all errors found.
///
/// Returns an empty `Vec` on success. The checker accumulates errors
/// rather than aborting on the first one, so a single call can surface
/// every problem at once. `Type::Unknown` is used as a recovery
/// placeholder so cascading errors don't compound.
#[must_use]
pub fn check(program: &Program) -> Vec<TypeError> {
    let mut checker = Checker::new(program);
    checker.check_program();
    checker.errors
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
    errors: Vec<TypeError>,
}

impl<'p> Checker<'p> {
    fn new(program: &'p Program) -> Self {
        Self {
            arena: &program.arena,
            items: &program.items,
            env: TypeEnvironment::with_prelude(),
            current_return_type: None,
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
            errors: Vec::new(),
        }
    }

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
            Item::Function(def) => {
                let parameters: Vec<Type> = def
                    .parameters
                    .iter()
                    .map(|p| self.resolve_type(p.type_annotation))
                    .collect();
                let return_type = def
                    .return_type
                    .map_or(Type::Unit, |id| self.resolve_type(id));
                let function_type = Type::Function {
                    parameters,
                    return_type: Box::new(return_type),
                };
                self.declare_or_record_dup(&def.name, function_type, item.span.clone());
            }
            Item::Const {
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
            Item::Import(_) | Item::ImportFrom(_) => {
                // Imports are syntactic placeholders until the module
                // loader (v0.2.x.6) ships. Names introduced by `import`
                // / `from … import …` are not yet bound here.
            }
            Item::Module(_) => {
                // Module declarations are not yet checked; the module
                // loader (v0.2.x.6) will recurse into inline content
                // and resolve external file-bound modules.
            }
            Item::Struct(def) => {
                // Push a frame where type params are visible during
                // field type resolution.
                self.env.push_frame();
                for param in &def.type_params {
                    self.env.declare(param, Type::TypeParam(param.clone()));
                }
                let fields: Vec<(String, Type)> = def
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), self.resolve_type(f.type_annotation)))
                    .collect();
                self.env.pop_frame();
                let ty = Type::UserStruct {
                    name: def.name.clone(),
                    type_params: def.type_params.clone(),
                    fields,
                };
                self.declare_or_record_dup(&def.name, ty, item.span.clone());
            }
            Item::Enum(def) => {
                self.env.push_frame();
                for param in &def.type_params {
                    self.env.declare(param, Type::TypeParam(param.clone()));
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
                    type_params: def.type_params.clone(),
                    variants,
                };
                self.declare_or_record_dup(&def.name, ty, item.span.clone());
            }
        }
    }

    fn check_item(&mut self, item: &Spanned<Item>) {
        if let Item::Function(def) = &item.node {
            self.check_function(def);
        }
        // Struct / Enum definitions have no runtime body to check
        // (field types are resolved during declaration).
    }

    fn check_function(&mut self, def: &FunctionDef) {
        let return_type = def
            .return_type
            .map_or(Type::Unit, |id| self.resolve_type(id));

        self.env.push_frame();
        self.current_return_type = Some(return_type.clone());

        for parameter in &def.parameters {
            let ty = self.resolve_type(parameter.type_annotation);
            self.env.declare(&parameter.name, ty);
        }

        match &def.body {
            FunctionBody::Block(block) => {
                let body_ty = self.check_block(block);
                if !return_type.matches(&body_ty) {
                    self.errors.push(TypeError::Mismatch {
                        expected: return_type,
                        found: body_ty,
                        span: block_span(self.arena, block),
                    });
                }
            }
            FunctionBody::Expression(expr) => {
                let body_ty = self.infer_expression(*expr);
                if !return_type.matches(&body_ty) {
                    self.errors.push(TypeError::Mismatch {
                        expected: return_type,
                        found: body_ty,
                        span: self.arena.expression(*expr).span.clone(),
                    });
                }
            }
        }

        self.current_return_type = None;
        self.env.pop_frame();
    }

    // ====================================================================
    // Statements / blocks
    // ====================================================================

    fn check_block(&mut self, block: &Block) -> Type {
        self.env.push_frame();
        for stmt_id in &block.statements {
            self.check_statement(*stmt_id);
        }
        let value_type = block
            .final_expression
            .map_or(Type::Unit, |id| self.infer_expression(id));
        self.env.pop_frame();
        value_type
    }

    fn check_statement(&mut self, id: StmtId) {
        let stmt = self.arena.statement(id).clone();
        match stmt.node {
            Stmt::Let {
                name,
                mutable,
                type_annotation,
                value,
            } => {
                let ty = self.check_initializer(type_annotation, value);
                self.env.declare_with_mut(&name, ty, mutable);
            }
            Stmt::Assign { target, value } => {
                self.check_assignment(&target, value, stmt.span.clone());
            }
            Stmt::Const {
                name,
                type_annotation,
                value,
            } => {
                let ty = self.check_initializer(type_annotation, value);
                self.env.declare(&name, ty);
            }
            Stmt::Return(value) => {
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
            }
            Stmt::Break(value) => {
                // For v0.1, break-with-value is allowed only inside `loop`;
                // we don't track loop context here, so just type-check.
                if let Some(id) = value {
                    let _ = self.infer_expression(id);
                }
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
                let _ = self.check_block(&body);
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
                let _ = self.check_block(&body);
            }
            Stmt::Loop(body) => {
                let _ = self.check_block(&body);
            }
            Stmt::ExprStmt(expr) => {
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
        let inferred = self.infer_expression(value);
        match declared {
            Some(annotated) => {
                if !annotated.matches(&inferred) {
                    self.errors.push(TypeError::Mismatch {
                        expected: annotated.clone(),
                        found: inferred,
                        span: self.arena.expression(value).span.clone(),
                    });
                }
                annotated
            }
            None => inferred,
        }
    }

    fn check_assignment(&mut self, target: &str, value: ExprId, stmt_span: Span) {
        let value_ty = self.infer_expression(value);
        let value_span = self.arena.expression(value).span.clone();
        let Some(binding) = self.env.lookup_binding(target).cloned() else {
            self.errors.push(TypeError::UndefinedName {
                name: target.to_owned(),
                span: stmt_span,
            });
            return;
        };
        if !binding.mutable {
            self.errors.push(TypeError::AssignToImmutable {
                name: target.to_owned(),
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

    fn check_condition_type(&mut self, cond_ty: Type, treat_unknown_as_false: bool, span: Span) {
        match cond_ty {
            Type::Trilean | Type::Unknown => {
                // Plain `if` requires a definite Trilean. The checker
                // can't tell statically whether a Trilean is "always
                // known", so we accept any Trilean here and rely on
                // `if?` for explicit unknown handling. A future pass
                // could refine this.
                let _ = treat_unknown_as_false;
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
                if let Type::UserEnum { variants, .. } = scrutinee
                    && let Some((_, def_payload)) = variants
                        .iter()
                        .find(|(n, _)| n.as_str() == variant_name.as_str())
                    && let (Some(sub_pattern), Some(payload_ty)) = (payload, def_payload)
                {
                    self.bind_pattern(sub_pattern, payload_ty);
                }
            }
        }
    }

    // ====================================================================
    // Type-expression resolution + helpers
    // ====================================================================

    fn resolve_type(&mut self, id: TypeId) -> Type {
        let span = self.arena.type_expression(id).span.clone();
        match self.arena.type_expression(id).node.clone() {
            TypeExpr::Named(name) => match name.as_str() {
                "Trit" => Type::Trit,
                "Tryte" => Type::Tryte,
                "Integer" => Type::Integer,
                "Long" => Type::Long,
                "Trilean" => Type::Trilean,
                "String" => Type::String,
                "Unit" => Type::Unit,
                _ => {
                    // Look up user-defined types, type params, or aliases.
                    if let Some(ty) = self.env.lookup(&name).cloned() {
                        match &ty {
                            Type::UserStruct { .. }
                            | Type::UserEnum { .. }
                            | Type::TypeParam(_) => ty,
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
                if let Some(ty) = self.env.lookup(&name).cloned() {
                    match &ty {
                        Type::UserStruct { type_params, .. }
                        | Type::UserEnum { type_params, .. } => {
                            if type_params.len() != args.len() {
                                self.errors.push(TypeError::WrongArity {
                                    expected: type_params.len(),
                                    found: args.len(),
                                    span,
                                });
                                return Type::Unknown;
                            }
                            let map: std::collections::HashMap<_, _> =
                                type_params.iter().cloned().zip(args).collect();
                            return ty.substitute(&map);
                        }
                        // Non-struct types cannot have type params — fall through to UnknownType.
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
                parameters: parameters.iter().map(|t| self.resolve_type(*t)).collect(),
                return_type: Box::new(self.resolve_type(return_type)),
            },
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

fn block_span(arena: &Arena, block: &Block) -> Span {
    if let Some(id) = block.final_expression {
        arena.expression(id).span.clone()
    } else if let Some(stmt_id) = block.statements.last() {
        arena.statement(*stmt_id).span.clone()
    } else {
        0..0
    }
}

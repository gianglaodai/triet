//! Type-check a multi-module [`ResolvedProgram`].
//!
//! This is the v0.2.x module-aware counterpart to [`crate::check::check`],
//! which operates on a flat single-file `Program`. The algorithm:
//!
//! 1. **Pass 1 — Collect signatures.** Walk every module and extract
//!    declared types (function signatures, struct fields, etc.) into
//!    a per-module type table.
//!
//! 2. **Pass 2 — Check per module.** For each module, build a
//!    `TypeEnvironment` pre-seeded with imported types (resolved via
//!    the binding map + pass 1 type tables), then run the standard
//!    single-module checker on that module's items.
//!
//! The returned error list is empty on success.

use std::collections::HashMap;

use triet_modules::ResolvedProgram;
use triet_syntax::{Item, Span};

use crate::{check::check_with_env, env::TypeEnvironment, error::TypeError, types::Type};

/// Key identifying a trait implementation: `(type_name, trait_name)`
/// (ADR-0061 §2.2). Tier 1 permits at most one impl per key (coherence).
pub(crate) type ImplKey = (String, String);

/// Collect `(Type, Trait)` keys for every `implement` block in a module,
/// with the block's span for diagnostics (ADR-0061 T3.3 coherence input).
/// `for_type` resolves via `name_table` (mirrors [`collect_declared_types`]).
/// The full method/mangling tables (T3.1) land with the verification pass
/// (T3.2) that consumes them — they are omitted here to avoid a populated-
/// but-unread table (Track B rule #4).
pub(crate) fn collect_impl_keys(
    arena: &triet_syntax::Arena,
    items: &[triet_syntax::Spanned<Item>],
    name_table: &HashMap<String, Type>,
) -> Vec<(ImplKey, Span)> {
    let mut keys = Vec::new();
    for item in items {
        if let Item::Implementation { def } = &item.node {
            let type_name = resolve_type_expr(arena, def.for_type, name_table).to_string();
            keys.push(((type_name, def.trait_name.clone()), item.span.clone()));
        }
    }
    keys
}

/// Enforce trait coherence: at most one `implement` per `(Type, Trait)`
/// pair across the whole program (ADR-0061 §2.2). Emits E1043
/// (`DuplicateImplementation`) for each duplicate. Shared by both the
/// cross-module and single-file entry points so coherence is checked once.
pub(crate) fn check_impl_coherence(keys: Vec<(ImplKey, Span)>) -> Vec<TypeError> {
    let mut seen: std::collections::HashSet<ImplKey> = std::collections::HashSet::new();
    let mut errors = Vec::new();
    for (key, span) in keys {
        if seen.contains(&key) {
            errors.push(TypeError::DuplicateImplementation {
                type_name: key.0,
                trait_name: key.1,
                span,
            });
        } else {
            seen.insert(key);
        }
    }
    errors
}

// ── ADR-0061 T3.1/T3.2: trait_table + impl_table + conformance ──────

/// A resolved trait declaration (ADR-0061 T3.1). Method signatures with
/// parameter/return types already resolved, so conformance verification
/// (T3.2) compares against them without re-walking the arena.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TraitInfo {
    /// Resolved method signatures, in declaration order.
    pub methods: Vec<TraitMethodSig>,
}

/// One resolved trait method signature. `parameters` includes the leading
/// `self` receiver at index 0 (resolved to `Type::Unknown` at the trait
/// level — the receiver is positional, not part of the verified contract).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TraitMethodSig {
    /// Method name (e.g. `compare`).
    pub name: String,
    /// Resolved parameter types, `self` first.
    pub parameters: Vec<Type>,
    /// Resolved return type (`Type::Unit` when omitted).
    pub return_type: Type,
}

/// A resolved trait implementation (ADR-0061 T3.1). Maps each method name
/// to its resolved signature + mangled function name `Type$Trait$method`
/// (ADR §2.4, consumed by dispatch/lowering in T4/T5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImplInfo {
    /// Method name → resolved info.
    pub methods: HashMap<String, ImplMethodInfo>,
    /// Source span of the `implement` block (diagnostic anchor).
    pub span: Span,
}

/// Resolved info for one method inside an `implement` block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImplMethodInfo {
    /// Mangled function name `Type$Trait$method` (ADR §2.4).
    pub mangled: String,
    /// Resolved parameter types, `self` first.
    pub parameters: Vec<Type>,
    /// Resolved return type (`Type::Unit` when omitted).
    pub return_type: Type,
}

/// Collect resolved trait declarations from a module (ADR-0061 T3.1).
pub(crate) fn collect_trait_defs(
    arena: &triet_syntax::Arena,
    items: &[triet_syntax::Spanned<Item>],
    name_table: &HashMap<String, Type>,
) -> Vec<(String, TraitInfo)> {
    let mut traits = Vec::new();
    for item in items {
        if let Item::Trait { def } = &item.node {
            let methods = def
                .methods
                .iter()
                .map(|m| TraitMethodSig {
                    name: m.name.clone(),
                    parameters: m
                        .parameters
                        .iter()
                        .map(|p| resolve_type_expr(arena, p.type_annotation, name_table))
                        .collect(),
                    return_type: m
                        .return_type
                        .map_or(Type::Unit, |id| resolve_type_expr(arena, id, name_table)),
                })
                .collect();
            traits.push((def.name.clone(), TraitInfo { methods }));
        }
    }
    traits
}

/// Collect resolved trait implementations from a module (ADR-0061 T3.1).
/// `self` parameters resolve to `Type::Unknown` here (no impl context in
/// the free resolver); the receiver is skipped during conformance, and
/// body checking re-resolves `self` to `for_type` via the Checker.
pub(crate) fn collect_impl_defs(
    arena: &triet_syntax::Arena,
    items: &[triet_syntax::Spanned<Item>],
    name_table: &HashMap<String, Type>,
) -> Vec<(ImplKey, ImplInfo)> {
    let mut impls = Vec::new();
    for item in items {
        if let Item::Implementation { def } = &item.node {
            let type_name = resolve_type_expr(arena, def.for_type, name_table).to_string();
            let trait_name = def.trait_name.clone();
            let methods = def
                .methods
                .iter()
                .map(|method| {
                    let info = ImplMethodInfo {
                        mangled: triet_syntax::mangle_trait_method(
                            &type_name,
                            &trait_name,
                            &method.name,
                        ),
                        parameters: method
                            .parameters
                            .iter()
                            .map(|p| resolve_type_expr(arena, p.type_annotation, name_table))
                            .collect(),
                        return_type: method
                            .return_type
                            .map_or(Type::Unit, |id| resolve_type_expr(arena, id, name_table)),
                    };
                    (method.name.clone(), info)
                })
                .collect();
            impls.push((
                (type_name, trait_name),
                ImplInfo {
                    methods,
                    span: item.span.clone(),
                },
            ));
        }
    }
    impls
}

/// Strict conformance comparison with Unknown-tolerance (ADR-0061 T3.2).
/// G demands an exact 1-1 match, so types must be equal — except either
/// side being `Type::Unknown` (a resolution-failure recovery placeholder)
/// passes, to avoid compounding an already-reported error.
fn types_conform(expected: &Type, found: &Type) -> bool {
    matches!(expected, Type::Unknown) || matches!(found, Type::Unknown) || expected == found
}

/// Verify every `implement` block conforms to its trait (ADR-0061 T3.2):
/// same method set, each with matching arity / parameter types / return
/// type. Emits E1044 (`TraitImplConformanceMismatch`) per failure. Reads
/// BOTH tables (defeats dead-field, Track B rule #2/#4). Impls whose trait
/// is unknown are skipped (no contract to check against — see note to O).
pub(crate) fn check_conformance(
    trait_table: &HashMap<String, TraitInfo>,
    impl_table: &HashMap<ImplKey, ImplInfo>,
) -> Vec<TypeError> {
    use crate::error::ConformanceKind;
    let mut errors = Vec::new();

    for ((type_name, trait_name), impl_info) in impl_table {
        let Some(trait_info) = trait_table.get(trait_name) else {
            // ADR-0061 T3.5: `implement <UnknownTrait> for T` — the trait
            // name is undefined. Don't silently accept (a compiler never
            // takes bad input quietly); reuse E1002 UndefinedName (O ruling:
            // no new code). The unknown-TYPE case (`implement Trait for
            // Bogus`) is already caught as E1001 by resolve_type during
            // body checking — verified, not re-emitted here.
            errors.push(TypeError::UndefinedName {
                name: trait_name.clone(),
                span: impl_info.span.clone(),
            });
            continue;
        };
        let span = impl_info.span.clone();
        let mut push = |kind| {
            errors.push(TypeError::TraitImplConformanceMismatch {
                trait_name: trait_name.clone(),
                type_name: type_name.clone(),
                kind,
                span: span.clone(),
            });
        };

        // Missing methods: declared by the trait, absent in the impl.
        for sig in &trait_info.methods {
            if !impl_info.methods.contains_key(&sig.name) {
                push(ConformanceKind::MissingMethod {
                    method: sig.name.clone(),
                });
            }
        }
        // Extra methods: present in the impl, not declared by the trait.
        for name in impl_info.methods.keys() {
            if !trait_info.methods.iter().any(|s| &s.name == name) {
                push(ConformanceKind::ExtraMethod {
                    method: name.clone(),
                });
            }
        }
        // Signature match for methods present on both sides.
        for sig in &trait_info.methods {
            let Some(impl_method) = impl_info.methods.get(&sig.name) else {
                continue;
            };
            // Arity counts the `self` receiver on both sides.
            if sig.parameters.len() != impl_method.parameters.len() {
                push(ConformanceKind::WrongArity {
                    method: sig.name.clone(),
                    expected: sig.parameters.len(),
                    found: impl_method.parameters.len(),
                });
            }
            // Parameter types: skip index 0 (the positional `self` receiver,
            // resolved to Unknown here). Compare the overlapping range.
            let overlap = sig.parameters.len().min(impl_method.parameters.len());
            for i in 1..overlap {
                if !types_conform(&sig.parameters[i], &impl_method.parameters[i]) {
                    push(ConformanceKind::ParamType {
                        method: sig.name.clone(),
                        position: i + 1,
                        expected: sig.parameters[i].to_string(),
                        found: impl_method.parameters[i].to_string(),
                    });
                }
            }
            // Return type.
            if !types_conform(&sig.return_type, &impl_method.return_type) {
                push(ConformanceKind::ReturnType {
                    method: sig.name.clone(),
                    expected: sig.return_type.to_string(),
                    found: impl_method.return_type.to_string(),
                });
            }
        }
    }

    errors
}

/// Type-check every module in a [`ResolvedProgram`].
///
/// This is the primary entry point for the v0.2.x pipeline. Single-file
/// programs still work — they produce a `ResolvedProgram` with one
/// module (the crate root) and zero imports.
///
/// Returns an empty `Vec` on success. Errors from all modules are
/// merged so the user sees the full failure surface in one run.
#[must_use]
pub fn check_resolved(program: &ResolvedProgram) -> Vec<TypeError> {
    let mut all_errors = Vec::new();

    // Pass 1: Collect declared types per module. Iterate to a fixed point
    // so cross-module type references resolve into full UserStruct/
    // UserEnum shapes rather than `Type::Unknown` — without this fixup,
    // `struct Spanned { token: Token }` declared in module A and imported
    // by module B would leave `Spanned`'s `token` field typed as Unknown
    // when B looks it up, breaking expressions like
    // `match spanned.token { Variant(payload) => ... }` (the
    // `bind_pattern` UserEnum guard fails, so the payload binding never
    // enters scope and E1002 fires on the payload reference). Bound at
    // `modules.len()` iterations — deeper dep chains converge before
    // that. Surfaced by the v0.7.5.2 parser.tri port (cross-module
    // imports of Token + SpannedToken from compiler/lexer.tri).
    let mut module_types: Vec<Vec<(String, Type)>> = program
        .modules
        .iter()
        .map(|module| {
            let arena = program.arena(module);
            collect_declared_types(arena, &module.items, &HashMap::new())
        })
        .collect();
    let max_iterations = program.modules.len();
    for _ in 0..max_iterations {
        let name_table: HashMap<String, Type> = module_types
            .iter()
            .flat_map(|m| m.iter().map(|(n, t)| (n.clone(), t.clone())))
            .collect();
        let next: Vec<Vec<(String, Type)>> = program
            .modules
            .iter()
            .map(|module| {
                let arena = program.arena(module);
                collect_declared_types(arena, &module.items, &name_table)
            })
            .collect();
        if next == module_types {
            break;
        }
        module_types = next;
    }

    // ADR-0061 T3.3: trait coherence — at most one `implement` per
    // (Type, Trait) across the whole program. Built once from the
    // converged name_table so user `for_type`s resolve to their real names.
    let name_table: HashMap<String, Type> = module_types
        .iter()
        .flat_map(|m| m.iter().map(|(n, t)| (n.clone(), t.clone())))
        .collect();
    let mut impl_keys = Vec::new();
    for module in &program.modules {
        let arena = program.arena(module);
        impl_keys.extend(collect_impl_keys(arena, &module.items, &name_table));
    }
    all_errors.extend(check_impl_coherence(impl_keys));

    // ADR-0061 T3.1/T3.2: build trait_table + impl_table across all
    // modules (a trait may be declared in one module, implemented in
    // another), then verify conformance. Both tables are read here, so
    // neither is a dead field (Track B rule #2/#4).
    let trait_table: HashMap<String, TraitInfo> = program
        .modules
        .iter()
        .flat_map(|module| collect_trait_defs(program.arena(module), &module.items, &name_table))
        .collect();
    let impl_table: HashMap<ImplKey, ImplInfo> = program
        .modules
        .iter()
        .flat_map(|module| collect_impl_defs(program.arena(module), &module.items, &name_table))
        .collect();
    all_errors.extend(check_conformance(&trait_table, &impl_table));

    // Pass 2: For each module, build env with imports, then check.
    for (idx, module) in program.modules.iter().enumerate() {
        let arena = program.arena(module);

        // Build a single-module Program view.
        let single_program = triet_syntax::Program {
            arena: arena.clone(),
            items: module.items.clone(),
            source_file: String::new(),
        };

        // Pre-seed the root module's environment with the standard
        // prelude (print, println, to_string, etc.) for backward-
        // compat with v0.1.x single-file programs. Child modules and
        // stdlib modules do not get the prelude — they must use
        // explicit `from std.io import …` declarations.
        let mut env = if idx == program.root.raw() {
            TypeEnvironment::with_prelude()
        } else {
            TypeEnvironment::default()
        };
        for (local_name, abs_path) in &module.bindings {
            let source_path = abs_path.module_path();
            let item_name = abs_path.name();

            // Skip own definitions — the checker's declare pass handles them.
            if *source_path == module.path {
                continue;
            }

            // Try user module first.
            if let Some(source_id) = program.find_module(source_path) {
                if source_id.raw() == idx {
                    continue; // self-import
                }
                let source_types = &module_types[source_id.raw()];
                if let Some((_, ty)) = source_types.iter().find(|(n, _)| n == item_name) {
                    env.declare(local_name, ty.clone());
                }
            } else if let Some(root) = source_path.root() {
                // v0.8.11: Ambient/Capability modules (`sys`, `dev`, `usr`, `std`, `core`)
                // do not have user-module definitions. We inject their names as `Type::Unknown`
                // so the typechecker doesn't complain about undefined names (E1002).
                if matches!(root, "sys" | "dev" | "usr" | "std" | "core") {
                    env.declare(local_name, Type::Unknown);
                }
            }
        }

        let errors = check_with_env(&single_program, env, impl_table.clone());
        all_errors.extend(errors);
    }

    all_errors
}

/// Walk a module's items and extract declared types for each named item.
/// `name_table` (typically built from the previous Pass 1 iteration) lets
/// cross-module user-type references resolve into their full `UserStruct`
/// / `UserEnum` shapes rather than falling through to `Type::Unknown`.
pub(crate) fn collect_declared_types(
    arena: &triet_syntax::Arena,
    items: &[triet_syntax::Spanned<Item>],
    name_table: &HashMap<String, Type>,
) -> Vec<(String, Type)> {
    let mut result = Vec::new();

    for item in items {
        match &item.node {
            Item::Function { def } => {
                let parameters: Vec<Type> = def
                    .parameters
                    .iter()
                    .map(|p| {
                        resolve_type_expr_with_params(
                            arena,
                            p.type_annotation,
                            &def.type_parameters,
                            name_table,
                        )
                    })
                    .collect();
                let return_type = def.return_type.map_or(Type::Unit, |id| {
                    resolve_type_expr_with_params(arena, id, &def.type_parameters, name_table)
                });
                result.push((
                    def.name.clone(),
                    Type::Function {
                        type_parameters: def.type_parameters.clone(),
                        parameters,
                        return_type: Box::new(return_type),
                    },
                ));
            }
            Item::Constant {
                name,
                type_annotation,
                ..
            } => {
                let ty = type_annotation
                    .map_or(Type::Unknown, |id| resolve_type_expr(arena, id, name_table));
                result.push((name.clone(), ty));
            }
            Item::Struct { def } => {
                let fields: Vec<(String, Type)> = def
                    .fields
                    .iter()
                    .map(|f| {
                        (
                            f.name.clone(),
                            resolve_type_expr(arena, f.type_annotation, name_table),
                        )
                    })
                    .collect();
                result.push((
                    def.name.clone(),
                    Type::UserStruct {
                        name: def.name.clone(),
                        type_parameters: def.type_parameters.clone(),
                        fields,
                    },
                ));
            }
            Item::Enum { def } => {
                let variants: Vec<(String, Option<Box<Type>>)> = def
                    .variants
                    .iter()
                    .map(|v| {
                        let payload = v
                            .payload
                            .map(|tid| Box::new(resolve_type_expr(arena, tid, name_table)));
                        (v.name.clone(), payload)
                    })
                    .collect();
                result.push((
                    def.name.clone(),
                    Type::UserEnum {
                        name: def.name.clone(),
                        type_parameters: def.type_parameters.clone(),
                        variants,
                    },
                ));
            }
            // ADR-0069: a capability is a ZST type — register it so type
            // annotations (`take(c: Cap)`) resolve through `resolve_type_expr`.
            // The level / mint-gate lives in the Checker (check.rs); here we
            // only contribute the type name.
            Item::Capability { name, .. } => {
                result.push((
                    name.clone(),
                    Type::UserStruct {
                        name: name.clone(),
                        type_parameters: Vec::new(),
                        fields: Vec::new(),
                    },
                ));
            }
            // ADR-0061 Tier 1: trait/implement don't contribute a value Type to
            // the name_table — traits live in a separate trait_table (T3). The
            // parser does not yet emit these (T2); inert for match exhaustiveness.
            Item::Trait { .. }
            | Item::Implementation { .. }
            | Item::TypeAlias { .. }
            | Item::Import { .. }
            | Item::ImportFrom { .. }
            | Item::Module { .. } => {}
        }
    }

    result
}

/// Resolve a type expression to a Type. Handles built-in names, tuples,
/// nullables, and function types. User-defined types resolve via
/// `name_table` when known; otherwise fall through to `Type::Unknown`
/// (single-module Pass 2 still re-resolves through env).
fn resolve_type_expr(
    arena: &triet_syntax::Arena,
    id: triet_syntax::TypeId,
    name_table: &HashMap<String, Type>,
) -> Type {
    resolve_type_expr_with_params(arena, id, &[], name_table)
}

/// Like [`resolve_type_expr`] but treats `type_parameters` (e.g. `T`, `U`)
/// as `Type::TypeParameter(name)` rather than `Type::Unknown`. Used by
/// generic function signature extraction (v0.7.4.1, ADR-0019 Addendum
/// §A7) so that a parameter typed `T` resolves to a type-param
/// reference, not the unknown sink.
#[allow(clippy::too_many_lines)]
fn resolve_type_expr_with_params(
    arena: &triet_syntax::Arena,
    id: triet_syntax::TypeId,
    type_parameters: &[triet_syntax::TypeParameter],
    name_table: &HashMap<String, Type>,
) -> Type {
    use triet_syntax::TypeExpr;
    match &arena.type_expression(id).node {
        TypeExpr::Named(name) => match name.as_str() {
            "Trit" => Type::Trit,
            "Tryte" => Type::Tryte,
            "Integer" => Type::Integer,
            "Long" => Type::Long,
            // ADR-0021: bare `Trilean` annotation in a type expression
            // is generic Trilean (might be Unknown). Authors who want
            // `Trilean!` annotation will need to write it explicitly —
            // syntax for that is deferred (see ADR-0021 §2.7 — function
            // return type narrowing is the main use case today).
            "Trilean" => Type::TRILEAN,
            "String" => Type::String,
            "Unit" => Type::Unit,
            other if type_parameters.iter().any(|p| p.name == other) => {
                Type::TypeParameter(other.to_owned())
            }
            other => name_table.get(other).cloned().unwrap_or(Type::Unknown),
        },
        TypeExpr::Tuple(elements) => Type::Tuple(
            elements
                .iter()
                .map(|t| resolve_type_expr_with_params(arena, *t, type_parameters, name_table))
                .collect(),
        ),
        TypeExpr::Nullable(inner) => Type::Nullable(Box::new(resolve_type_expr_with_params(
            arena,
            *inner,
            type_parameters,
            name_table,
        ))),
        TypeExpr::Function {
            parameters,
            return_type,
        } => Type::Function {
            type_parameters: Vec::new(),
            parameters: parameters
                .iter()
                .map(|t| resolve_type_expr_with_params(arena, *t, type_parameters, name_table))
                .collect(),
            return_type: Box::new(resolve_type_expr_with_params(
                arena,
                *return_type,
                type_parameters,
                name_table,
            )),
        },
        TypeExpr::Generic { name, arguments } if name == "Atomic" && arguments.len() == 1 => {
            // v0.9.x.atomic.5c (closes .1 deferred). Cross-module signature
            // resolution path. E1040 itself is fired by [`check.rs`] at the
            // original declaration site (single source of truth for error
            // attribution); here we just refuse to propagate a malformed
            // `Type::Atomic(<non-AtomicValue>)` into the shared name_table.
            // Returning `Type::Unknown` keeps downstream signature shapes
            // stable while avoiding bad-Atomic cascade. Per ADR-0028 §2
            // AtomicValue membership rule.
            let inner =
                resolve_type_expr_with_params(arena, arguments[0], type_parameters, name_table);
            if !inner.is_atomic_value() {
                return Type::Unknown;
            }
            Type::Atomic(Box::new(inner))
        }
        // v0.7.4.2: Vector<T>/HashMap<K,V> in stdlib stub signatures.
        // Mirror the pseudo-struct shells materialized by `check.rs`
        // so cross-module signature extraction round-trips. Other
        // user-generic instantiations (e.g. Option<T>) still resolve
        // to Unknown here — they're handled during full per-module
        // checking via the env-lookup path.
        TypeExpr::Generic { name, arguments } if name == "Vector" && arguments.len() == 1 => {
            Type::Vector(Box::new(resolve_type_expr_with_params(
                arena,
                arguments[0],
                type_parameters,
                name_table,
            )))
        }
        TypeExpr::Generic { name, arguments } if name == "HashMap" && arguments.len() == 2 => {
            Type::UserStruct {
                name: "HashMap".into(),
                type_parameters: Vec::new(),
                fields: vec![
                    (
                        "__key".into(),
                        resolve_type_expr_with_params(
                            arena,
                            arguments[0],
                            type_parameters,
                            name_table,
                        ),
                    ),
                    (
                        "__value".into(),
                        resolve_type_expr_with_params(
                            arena,
                            arguments[1],
                            type_parameters,
                            name_table,
                        ),
                    ),
                ],
            }
        }
        TypeExpr::Generic { .. } => Type::Unknown,
        // v0.7.4.3-error.2 (ADR-0020 §1): cross-module signature
        // extraction for `T~E` / `T?~E`. Mirrors `check.rs::resolve_type`
        // Outcome arm above (sans error reporting — this path runs at
        // import-collection time before per-module checker spins up).
        TypeExpr::Outcome {
            value_type,
            error_type,
            allow_null_state,
        } => Type::Outcome {
            value_type: Box::new(resolve_type_expr_with_params(
                arena,
                *value_type,
                type_parameters,
                name_table,
            )),
            error_type: Box::new(resolve_type_expr_with_params(
                arena,
                *error_type,
                type_parameters,
                name_table,
            )),
            allow_null_state: *allow_null_state,
        },
        // v0.7.4.3-debt.1: `Trilean!` annotation per ADR-0021 §2.7.
        TypeExpr::RefinedTrilean => Type::TRILEAN_KNOWN,
        TypeExpr::Reference { form, inner } => {
            let inner_ty =
                resolve_type_expr_with_params(arena, *inner, type_parameters, name_table);
            Type::Reference(*form, Box::new(inner_ty))
        }
        // ADR-0061 T3: resolve `Self` → receiver type. Placeholder until
        // impl-context resolution lands; reachable via T2.4 `self` param.
        TypeExpr::SelfType => Type::Unknown,
    }
}

#[cfg(test)]
#[allow(clippy::doc_markdown)]
mod tests {
    use super::*;

    // ── Helpers ─────────────────────────────────────────────────────

    fn check_in_memory(source: &str) -> Vec<TypeError> {
        let program = triet_modules::load_program_from_source(source).expect("load should succeed");
        filter_warnings(check_resolved(&program))
    }

    /// v0.7.4.3-error.2: drop Warning-severity diagnostics (W2001
    /// NullDeprecated). Stdlib stubs still use `null` until
    /// v0.7.4.3-error.4 migration tool — keep unit tests focused on
    /// hard errors.
    fn filter_warnings(errors: Vec<TypeError>) -> Vec<TypeError> {
        use miette::Diagnostic;
        errors
            .into_iter()
            .filter(|err| err.severity() != Some(miette::Severity::Warning))
            .collect()
    }

    fn check_filesystem(files: &[(&str, &str)]) -> Vec<TypeError> {
        let temp = tempfile::tempdir().expect("tempdir");
        let base = temp.path();
        let mut root_path = None;

        for (rel_path, contents) in files {
            let full = base.join(rel_path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).expect("create_dir_all");
            }
            std::fs::write(&full, contents).expect("write");
            if root_path.is_none() {
                root_path = Some(full);
            }
        }

        let program = triet_modules::load_program(root_path.as_ref().expect("at least one file"))
            .expect("load should succeed");
        filter_warnings(check_resolved(&program))
    }

    // ── Single-module (backward compat) ─────────────────────────────

    #[test]
    fn single_module_happy_path() {
        let errors = check_in_memory("function main() -> Integer = 42");
        assert!(errors.is_empty(), "no errors expected: {errors:?}");
    }

    #[test]
    fn single_module_type_error() {
        let errors = check_in_memory(r#"function main() -> Integer = "oops""#);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, TypeError::Mismatch { .. })),
            "expected Mismatch: {errors:?}"
        );
    }

    // ── ADR-0061 T3.3: trait coherence (E1043) ──────────────────────

    #[test]
    fn single_impl_is_coherent() {
        // One `implement` per (Type, Trait) → no coherence error.
        let errors = check_in_memory(
            "trait Comparable { function compare(self, other: Integer) -> Integer }\n\
             implement Comparable for Integer { function compare(self, other: Integer) -> Integer = other }\n\
             function main() -> Integer = 0",
        );
        assert!(
            !errors
                .iter()
                .any(|e| matches!(e, TypeError::DuplicateImplementation { .. })),
            "single impl must not raise E1043: {errors:?}"
        );
    }

    #[test]
    fn duplicate_impl_emits_e1043() {
        // Two `implement Comparable for Integer` → coherence conflict E1043.
        // Poison: make check_impl_coherence skip the duplicate check (always
        // insert) → this assertion goes red.
        let errors = check_in_memory(
            "trait Comparable { function compare(self, other: Integer) -> Integer }\n\
             implement Comparable for Integer { function compare(self, other: Integer) -> Integer = other }\n\
             implement Comparable for Integer { function compare(self, other: Integer) -> Integer = other }\n\
             function main() -> Integer = 0",
        );
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, TypeError::DuplicateImplementation { .. })),
            "duplicate (Integer, Comparable) impl must raise E1043: {errors:?}"
        );
    }

    // ── ADR-0061 T3.1: trait_table / impl_table build ───────────────

    fn impl_table_of(source: &str) -> HashMap<ImplKey, ImplInfo> {
        let program = triet_modules::load_program_from_source(source).expect("load");
        let module = &program.modules[program.root.raw()];
        let arena = program.arena(module);
        let name_table: HashMap<String, Type> =
            collect_declared_types(arena, &module.items, &HashMap::new())
                .into_iter()
                .collect();
        collect_impl_defs(arena, &module.items, &name_table)
            .into_iter()
            .collect()
    }

    #[test]
    fn impl_table_records_mangled_name() {
        // T3.1 teeth: impl_table holds the mangled `Type$Trait$method`.
        // Poison: change the `format!` mangling → this assertion goes red.
        let table = impl_table_of(
            "trait Comparable { function compare(self, other: Integer) -> Integer }\n\
             implement Comparable for Integer { function compare(self, other: Integer) -> Integer = other }\n\
             function main() -> Integer = 0",
        );
        let info = table
            .get(&("Integer".to_owned(), "Comparable".to_owned()))
            .expect("impl_table must contain (Integer, Comparable)");
        assert_eq!(
            info.methods.get("compare").map(|m| m.mangled.as_str()),
            Some("Integer$Comparable$compare"),
            "mangled name must be Integer$Comparable$compare: {info:?}"
        );
    }

    // ── ADR-0061 T3.2: conformance (E1044) ──────────────────────────

    use crate::error::ConformanceKind;

    fn conformance_kinds(errors: &[TypeError]) -> Vec<ConformanceKind> {
        errors
            .iter()
            .filter_map(|e| match e {
                TypeError::TraitImplConformanceMismatch { kind, .. } => Some(kind.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn conformant_impl_is_clean() {
        let errors = check_in_memory(
            "trait Comparable { function compare(self, other: Integer) -> Integer }\n\
             implement Comparable for Integer { function compare(self, other: Integer) -> Integer = other }\n\
             function main() -> Integer = 0",
        );
        assert!(
            conformance_kinds(&errors).is_empty(),
            "conformant impl must not raise E1044: {errors:?}"
        );
    }

    #[test]
    fn conformance_wrong_arity_emits_e1044() {
        // Impl `compare(self)` is missing `other` → WrongArity.
        // Poison: skip the arity check → this case slips → red.
        let errors = check_in_memory(
            "trait Comparable { function compare(self, other: Integer) -> Integer }\n\
             implement Comparable for Integer { function compare(self) -> Integer = 0 }\n\
             function main() -> Integer = 0",
        );
        assert!(
            conformance_kinds(&errors)
                .iter()
                .any(|k| matches!(k, ConformanceKind::WrongArity { .. })),
            "missing param must raise E1044 WrongArity: {errors:?}"
        );
    }

    #[test]
    fn conformance_wrong_return_emits_e1044() {
        let errors = check_in_memory(
            "trait Comparable { function compare(self, other: Integer) -> Integer }\n\
             implement Comparable for Integer { function compare(self, other: Integer) -> Trit = 0_trit }\n\
             function main() -> Integer = 0",
        );
        assert!(
            conformance_kinds(&errors)
                .iter()
                .any(|k| matches!(k, ConformanceKind::ReturnType { .. })),
            "wrong return type must raise E1044 ReturnType: {errors:?}"
        );
    }

    #[test]
    fn conformance_wrong_param_type_emits_e1044() {
        let errors = check_in_memory(
            "trait Comparable { function compare(self, other: Integer) -> Integer }\n\
             implement Comparable for Integer { function compare(self, other: Trit) -> Integer = 0 }\n\
             function main() -> Integer = 0",
        );
        assert!(
            conformance_kinds(&errors)
                .iter()
                .any(|k| matches!(k, ConformanceKind::ParamType { .. })),
            "wrong param type must raise E1044 ParamType: {errors:?}"
        );
    }

    #[test]
    fn conformance_missing_method_emits_e1044() {
        let errors = check_in_memory(
            "trait Two { function a(self) -> Integer\n function b(self) -> Integer }\n\
             implement Two for Integer { function a(self) -> Integer = 0 }\n\
             function main() -> Integer = 0",
        );
        assert!(
            conformance_kinds(&errors)
                .iter()
                .any(|k| matches!(k, ConformanceKind::MissingMethod { method } if method == "b")),
            "missing method `b` must raise E1044 MissingMethod: {errors:?}"
        );
    }

    #[test]
    fn conformance_extra_method_emits_e1044() {
        let errors = check_in_memory(
            "trait One { function a(self) -> Integer }\n\
             implement One for Integer { function a(self) -> Integer = 0\n function z(self) -> Integer = 0 }\n\
             function main() -> Integer = 0",
        );
        assert!(
            conformance_kinds(&errors)
                .iter()
                .any(|k| matches!(k, ConformanceKind::ExtraMethod { method } if method == "z")),
            "extra method `z` must raise E1044 ExtraMethod: {errors:?}"
        );
    }

    // ── ADR-0061 T3.5: unknown trait / type in `implement` ──────────

    #[test]
    fn unknown_trait_in_impl_emits_e1002() {
        // `implement <undeclared trait> for Integer` must not be silently
        // accepted. Poison: revert the else-arm to a bare `continue` →
        // this negative test goes red (empty errors).
        let errors = check_in_memory(
            "implement Bogus for Integer { function a(self) -> Integer = 0 }\n\
             function main() -> Integer = 0",
        );
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, TypeError::UndefinedName { name, .. } if name == "Bogus")),
            "unknown trait `Bogus` in impl must raise E1002: {errors:?}"
        );
    }

    #[test]
    fn unknown_type_in_impl_emits_e1001() {
        // `implement Trait for <unknown type>` is already caught as E1001
        // UnknownType by resolve_type during body checking — confirm the
        // hole is closed (not silently accepted), without a second code.
        let errors = check_in_memory(
            "trait T { function a(self) -> Integer }\n\
             implement T for Bogus { function a(self) -> Integer = 0 }\n\
             function main() -> Integer = 0",
        );
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, TypeError::UnknownType { name, .. } if name == "Bogus")),
            "unknown type `Bogus` in impl must raise E1001: {errors:?}"
        );
    }

    // ── ADR-0061 T3.4: self resolves to for_type in method bodies ────

    #[test]
    fn self_resolves_to_for_type_in_body() {
        // The trait+impl signatures conform (both `-> String`), so no
        // E1044 fires. But the body `= self` returns the receiver, which
        // is `Integer` (for_type) — not `String`. That return mismatch
        // (E1004) only appears if `self` resolved to Integer.
        // Poison: make SelfType resolve to Unknown → Unknown matches
        // String → the mismatch vanishes → this assertion goes red.
        let errors = check_in_memory(
            "trait Id { function get(self) -> String }\n\
             implement Id for Integer { function get(self) -> String = self }\n\
             function main() -> Integer = 0",
        );
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, TypeError::Mismatch { .. })),
            "self typed as Integer must mismatch declared `-> String`: {errors:?}"
        );
    }

    // ── Cross-module ────────────────────────────────────────────────

    #[test]
    fn cross_module_function_call() {
        let errors = check_filesystem(&[
            (
                "main.tri",
                "module helper\nfrom khi.helper import greet\nfunction main() -> Integer = greet()",
            ),
            ("helper.tri", "public function greet() -> Integer = 42"),
        ]);
        assert!(errors.is_empty(), "no errors expected: {errors:?}");
    }

    #[test]
    fn cross_module_type_mismatch() {
        let errors = check_filesystem(&[
            (
                "main.tri",
                "module helper
from khi.helper import greet
function main() -> String = greet()",
            ),
            ("helper.tri", "public function greet() -> Integer = 42"),
        ]);
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, TypeError::Mismatch { .. })),
            "expected Mismatch from cross-module call: {errors:?}"
        );
    }

    // ── Stdlib import ───────────────────────────────────────────────

    #[test]
    fn stdlib_import_type_checks() {
        let errors = check_in_memory(
            r#"from std.io import println
function main() = println("hello")"#,
        );
        assert!(errors.is_empty(), "no errors expected: {errors:?}");
    }

    // ── Inline module ───────────────────────────────────────────────

    #[test]
    fn inline_module_checks_independently() {
        let errors =
            check_in_memory("module helper {\n    public function aid() -> Integer = 42\n}");
        assert!(errors.is_empty(), "no errors expected: {errors:?}");
    }

    // ── Trilean! refinement (ADR-0021, v0.7.4.3-error.3d) ───────────

    /// Helper — count diagnostics matching a predicate.
    fn count_errors<F: Fn(&TypeError) -> bool>(errors: &[TypeError], pred: F) -> usize {
        errors.iter().filter(|e| pred(e)).count()
    }

    /// Helper — assert that errors contains exactly `expected` count of
    /// `PossiblyUnknownCondition`. Other diagnostics are ignored so
    /// the test doesn't break when parser shape changes around the
    /// snippet.
    fn assert_e1033_count(source: &str, expected: usize) {
        let errors = check_in_memory(source);
        let count = count_errors(&errors, |e| {
            matches!(e, TypeError::PossiblyUnknownCondition { .. })
        });
        assert_eq!(
            count, expected,
            "expected {expected} E1033, got {count}. errors={errors:#?}",
        );
    }

    #[test]
    fn integer_comparison_yields_trilean_known_safe_for_strict_if() {
        // `n > 0` is Trilean! because Integer ordering is total — no
        // Unknown propagation. Plain `if` accepts it.
        assert_e1033_count(
            "function f(n: Integer) -> String = if n > 0 { \"pos\" } else { \"non-pos\" }",
            0,
        );
    }

    #[test]
    fn plain_if_on_trilean_variable_emits_e1033() {
        // Bare Trilean parameter feeds plain `if` → E1033.
        assert_e1033_count(
            "function f(t: Trilean) -> String = if t { \"yes\" } else { \"no\" }",
            1,
        );
    }

    #[test]
    fn if_question_on_trilean_variable_typechecks_no_e1033() {
        // Relaxed `if?` accepts generic Trilean.
        assert_e1033_count(
            "function f(t: Trilean) -> String = if? t { \"yes\" } else { \"no\" }",
            0,
        );
    }

    #[test]
    fn trilean_eq_trilean_returns_generic_trilean_strict_if_rejects() {
        // `t == true`: Trilean × Trilean! → Trilean (per ADR-0021 §2.2
        // because Trilean side might be Unknown, Unknown == true is
        // Unknown per ADR-0010 §4). Plain `if` rejects.
        assert_e1033_count(
            "function f(t: Trilean) -> String = if t == true { \"y\" } else { \"n\" }",
            1,
        );
    }

    #[test]
    fn refined_and_refined_logic_op_preserves_refinement() {
        // Both `true` and `false` are Trilean!, so `true && false` is
        // Trilean! and plain `if` accepts. Literal-only expressions
        // also typecheck as Trilean! per §2.1.
        assert_e1033_count(
            "function f() -> String = if true && false { \"a\" } else { \"b\" }",
            0,
        );
    }

    #[test]
    fn refined_and_generic_logic_op_poisons_refinement() {
        // Trilean! && Trilean → Trilean (one side might be Unknown).
        assert_e1033_count(
            "function f(t: Trilean) -> String = if true && t { \"a\" } else { \"b\" }",
            1,
        );
    }

    #[test]
    fn nullable_comparison_yields_generic_trilean_rejected() {
        // T? == T? propagates null → Unknown per ADR-0010 §3.
        assert_e1033_count(
            "function f(a: Integer?, b: Integer?) -> String = if a == b { \"eq\" } else { \"ne\" }",
            1,
        );
    }

    #[test]
    fn assume_known_requires_message_argument_and_returns_refined() {
        // .assume_known("msg") returns Trilean! — usable in plain `if`.
        assert_e1033_count(
            "function f(t: Trilean) -> String = \
             if t.assume_known(\"validated upstream\") { \"y\" } else { \"n\" }",
            0,
        );
    }

    #[test]
    fn match_on_trilean_three_arm_typechecks() {
        // Exhaustive 3-arm match dispatches all states explicitly.
        assert_e1033_count(
            "function f(t: Trilean) -> String = match t { \
                true => \"y\", false => \"n\", unknown => \"?\" }",
            0,
        );
    }

    #[test]
    fn function_return_trilean_known_with_refined_body_typechecks() {
        // -> Trilean! body is `n > 0` (Trilean!) — OK.
        let errors = check_in_memory("function pos(n: Integer) -> Trilean = n > 0");
        // Returning Trilean! into Trilean slot is widening — no error.
        assert!(errors.is_empty(), "{errors:#?}");
    }

    #[test]
    fn function_returns_generic_trilean_in_refined_slot_does_not_widen() {
        // The function's declared return type itself is `Trilean` (not
        // refined), so this case doesn't trigger E1034 — that fires
        // only when the return annotation is `Trilean!`. Today the
        // parser doesn't expose Trilean! syntax in annotation
        // position; E1034 is reserved for when it does (the parser
        // extension is part of ADR-0021 §2.7 future work). For now
        // the test pins the placeholder: bare Trilean accepts both.
        let errors = check_in_memory("function f(t: Trilean) -> Trilean = t");
        assert!(errors.is_empty(), "{errors:#?}");
    }

    #[test]
    fn while_on_trilean_emits_e1033() {
        // `while` follows same rule as `if`. Statement form requires a
        // block body wrapping the loop.
        assert_e1033_count("function f(t: Trilean) -> Unit { while t { } }", 1);
    }

    #[test]
    fn match_guard_on_trilean_emits_e1033() {
        // Match guard expression must be `Trilean!`.
        assert_e1033_count(
            "function f(n: Integer, t: Trilean) -> String = \
             match n { _ if t => \"a\", _ => \"b\" }",
            1,
        );
    }

    #[test]
    fn negation_of_refined_stays_refined() {
        // !true is Trilean!, plain `if` accepts.
        assert_e1033_count(
            "function f() -> String = if !true { \"a\" } else { \"b\" }",
            0,
        );
    }

    #[test]
    fn equal_on_strings_yields_refined() {
        // Strings have total equality — `s1 == s2` is Trilean!.
        assert_e1033_count(
            "function f(a: String, b: String) -> String = \
             if a == b { \"eq\" } else { \"ne\" }",
            0,
        );
    }

    // ── Cross-module type resolution (v0.7.5.2) ─────────────────────

    /// Pre-fix: a struct whose field type was a user-defined enum from
    /// another module resolved to `Type::Unknown` in Pass 1 (cross-
    /// module names had no name table), so downstream match destructure
    /// against the field failed with E1002 "undefined name". This test
    /// proves the iterated Pass-1 fixed-point resolves the field type
    /// into the real `UserEnum` shape so `bind_pattern` can introduce
    /// the payload binding.
    #[test]
    fn cross_module_struct_field_match_payload_binds() {
        let errors = check_filesystem(&[
            (
                "main.tri",
                "module lib
from khi.lib import Token, Spanned, IntPayload
function describe(sp: Spanned) -> Integer = match sp.token {
    IntLit(p) => p.value,
    Kw => -1,
}
function main() -> Integer = describe(Spanned { token: IntLit(IntPayload { value: 7 }), span_start: 0 })",
            ),
            (
                "lib.tri",
                "public struct IntPayload { value: Integer }
public enum Token { Kw, IntLit(IntPayload) }
public struct Spanned { token: Token, span_start: Integer }",
            ),
        ]);
        assert!(
            errors.is_empty(),
            "cross-module struct.field match must type-check: {errors:?}"
        );
    }

    /// v0.9.x.atomic.5c — closes .1 deferred path. When module A declares
    /// a function whose Atomic payload type is valid AtomicValue
    /// (`Atomic<Integer>`), the cross-module name_table must propagate
    /// the proper `Type::Atomic(Integer)` shape so module B's call site
    /// resolves correctly.
    #[test]
    fn cross_module_valid_atomic_signature_round_trips() {
        let errors = check_filesystem(&[
            (
                "main.tri",
                "module helper
from khi.helper import bump
function main() {}",
            ),
            (
                "helper.tri",
                "public function bump(a: &+ Atomic<Integer>) {}",
            ),
        ]);
        assert!(
            errors.is_empty(),
            "valid cross-module Atomic<Integer> sig must round-trip: {errors:?}"
        );
    }

    /// v0.9.x.atomic.5c — closes .1 deferred path. When module A declares
    /// a malformed `Atomic<String>` signature, `check.rs` fires E1040 at
    /// the declaration site (module A). The cross-module resolver in
    /// `check_resolved.rs` must NOT additionally cascade a `bad-Atomic`
    /// type through module B's `name_table` — it returns `Type::Unknown`
    /// instead. Net effect: any E1040 fires are attributed to module A
    /// only; module B sees the import as `Unknown` and types-cleanly.
    ///
    /// Implementation note: `check.rs` resolves parameter types BOTH
    /// during `declare_item` AND `check_function`, so the same site
    /// fires twice — a pre-existing pattern unrelated to .5c. This
    /// test asserts the cross-module path does not *add* dups beyond
    /// check.rs's baseline by comparing counts at both source spans.
    #[test]
    fn cross_module_invalid_atomic_signature_no_cross_cascade() {
        use crate::error::TypeError;
        let errors = check_filesystem(&[
            (
                "main.tri",
                "module helper
from khi.helper import bad
function main() {}",
            ),
            ("helper.tri", "public function bad(a: &+ Atomic<String>) {}"),
        ]);
        let e1040_errors: Vec<_> = errors
            .iter()
            .filter(|e| matches!(e, TypeError::NonAtomicValueType { .. }))
            .collect();
        // At least one E1040 attributed to helper.tri's signature site.
        assert!(
            !e1040_errors.is_empty(),
            "expected E1040 for bad Atomic<String> sig, got: {errors:?}"
        );
        // All E1040 emissions point at the SAME source span (the
        // `String` token in helper.tri) — confirms no cascade into
        // main.tri's import or anywhere else. Distinct spans here
        // would mean the cross-module name_table propagated a bad
        // `Type::Atomic(String)` that fired again at a non-original
        // site, defeating .1's E1040 attribution.
        let unique_spans: std::collections::HashSet<_> = e1040_errors
            .iter()
            .filter_map(|e| match e {
                TypeError::NonAtomicValueType { span, .. } => Some(span.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            unique_spans.len(),
            1,
            "E1040 emissions must all point at the same helper.tri span (no cross-module cascade), got distinct spans: {unique_spans:?}"
        );
    }

    /// Same shape but the struct field is itself a struct (not an
    /// enum). Pre-fix the imported nested struct's fields were also
    /// Unknown, so `outer.inner.leaf` couldn't resolve `leaf`.
    #[test]
    fn cross_module_nested_struct_field_access() {
        let errors = check_filesystem(&[
            (
                "main.tri",
                "module lib
from khi.lib import Outer, Inner
function read(o: Outer) -> Integer = o.inner.leaf
function main() -> Integer = read(Outer { inner: Inner { leaf: 9 }, tag: 0 })",
            ),
            (
                "lib.tri",
                "public struct Inner { leaf: Integer }
public struct Outer { inner: Inner, tag: Integer }",
            ),
        ]);
        assert!(
            errors.is_empty(),
            "cross-module nested struct access must type-check: {errors:?}"
        );
    }
}

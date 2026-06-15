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
use triet_syntax::Item;

use crate::{check::check_with_env, env::TypeEnvironment, error::TypeError, types::Type};

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

        let errors = check_with_env(&single_program, env);
        all_errors.extend(errors);
    }

    all_errors
}

/// Walk a module's items and extract declared types for each named item.
/// `name_table` (typically built from the previous Pass 1 iteration) lets
/// cross-module user-type references resolve into their full `UserStruct`
/// / `UserEnum` shapes rather than falling through to `Type::Unknown`.
fn collect_declared_types(
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

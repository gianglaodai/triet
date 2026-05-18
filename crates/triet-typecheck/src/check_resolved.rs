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

    // Pass 1: Collect declared types per module.
    let module_types: Vec<Vec<(String, Type)>> = program
        .modules
        .iter()
        .map(|module| {
            let arena = program.arena(module);
            collect_declared_types(arena, &module.items)
        })
        .collect();

    // Pass 2: For each module, build env with imports, then check.
    for (idx, module) in program.modules.iter().enumerate() {
        let arena = program.arena(module);

        // Build a single-module Program view.
        let single_program = triet_syntax::Program {
            arena: arena.clone(),
            items: module.items.clone(),
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
            }
        }

        let errors = check_with_env(&single_program, env);
        all_errors.extend(errors);
    }

    all_errors
}

/// Walk a module's items and extract declared types for each named item.
fn collect_declared_types(
    arena: &triet_syntax::Arena,
    items: &[triet_syntax::Spanned<Item>],
) -> Vec<(String, Type)> {
    let mut result = Vec::new();

    for item in items {
        match &item.node {
            Item::Function(def) => {
                let parameters: Vec<Type> = def
                    .parameters
                    .iter()
                    .map(|p| {
                        resolve_type_expr_with_params(arena, p.type_annotation, &def.type_params)
                    })
                    .collect();
                let return_type = def.return_type.map_or(Type::Unit, |id| {
                    resolve_type_expr_with_params(arena, id, &def.type_params)
                });
                result.push((
                    def.name.clone(),
                    Type::Function {
                        type_params: def.type_params.clone(),
                        parameters,
                        return_type: Box::new(return_type),
                    },
                ));
            }
            Item::Const {
                name,
                type_annotation,
                ..
            } => {
                let ty = type_annotation.map_or(Type::Unknown, |id| resolve_type_expr(arena, id));
                result.push((name.clone(), ty));
            }
            Item::Struct(def) => {
                let fields: Vec<(String, Type)> = def
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), resolve_type_expr(arena, f.type_annotation)))
                    .collect();
                result.push((
                    def.name.clone(),
                    Type::UserStruct {
                        name: def.name.clone(),
                        type_params: def.type_params.clone(),
                        fields,
                    },
                ));
            }
            Item::Enum(def) => {
                let variants: Vec<(String, Option<Box<Type>>)> = def
                    .variants
                    .iter()
                    .map(|v| {
                        let payload = v.payload.map(|tid| Box::new(resolve_type_expr(arena, tid)));
                        (v.name.clone(), payload)
                    })
                    .collect();
                result.push((
                    def.name.clone(),
                    Type::UserEnum {
                        name: def.name.clone(),
                        type_params: def.type_params.clone(),
                        variants,
                    },
                ));
            }
            Item::TypeAlias { .. } | Item::Import(_) | Item::ImportFrom(_) | Item::Module(_) => {}
        }
    }

    result
}

/// Resolve a type expression to a Type. Handles built-in names, tuples,
/// nullables, and function types. User-defined types resolve to Unknown
/// at this stage (they're handled during full checking).
fn resolve_type_expr(arena: &triet_syntax::Arena, id: triet_syntax::TypeId) -> Type {
    resolve_type_expr_with_params(arena, id, &[])
}

/// Like [`resolve_type_expr`] but treats `type_params` (e.g. `T`, `U`)
/// as `Type::TypeParam(name)` rather than `Type::Unknown`. Used by
/// generic function signature extraction (v0.7.4.1, ADR-0019 Addendum
/// §A7) so that a parameter typed `T` resolves to a type-param
/// reference, not the unknown sink.
fn resolve_type_expr_with_params(
    arena: &triet_syntax::Arena,
    id: triet_syntax::TypeId,
    type_params: &[String],
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
            other if type_params.iter().any(|p| p == other) => Type::TypeParam(other.to_owned()),
            _ => Type::Unknown,
        },
        TypeExpr::Tuple(elements) => Type::Tuple(
            elements
                .iter()
                .map(|t| resolve_type_expr_with_params(arena, *t, type_params))
                .collect(),
        ),
        TypeExpr::Nullable(inner) => Type::Nullable(Box::new(resolve_type_expr_with_params(
            arena,
            *inner,
            type_params,
        ))),
        TypeExpr::Function {
            parameters,
            return_type,
        } => Type::Function {
            type_params: Vec::new(),
            parameters: parameters
                .iter()
                .map(|t| resolve_type_expr_with_params(arena, *t, type_params))
                .collect(),
            return_type: Box::new(resolve_type_expr_with_params(
                arena,
                *return_type,
                type_params,
            )),
        },
        // v0.7.4.2: Vector<T>/HashMap<K,V> in stdlib stub signatures.
        // Mirror the pseudo-struct shells materialized by `check.rs`
        // so cross-module signature extraction round-trips. Other
        // user-generic instantiations (e.g. Option<T>) still resolve
        // to Unknown here — they're handled during full per-module
        // checking via the env-lookup path.
        TypeExpr::Generic { name, arguments } if name == "Vector" && arguments.len() == 1 => {
            Type::UserStruct {
                name: "Vector".into(),
                type_params: Vec::new(),
                fields: vec![(
                    "__element".into(),
                    resolve_type_expr_with_params(arena, arguments[0], type_params),
                )],
            }
        }
        TypeExpr::Generic { name, arguments } if name == "HashMap" && arguments.len() == 2 => {
            Type::UserStruct {
                name: "HashMap".into(),
                type_params: Vec::new(),
                fields: vec![
                    (
                        "__key".into(),
                        resolve_type_expr_with_params(arena, arguments[0], type_params),
                    ),
                    (
                        "__value".into(),
                        resolve_type_expr_with_params(arena, arguments[1], type_params),
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
                type_params,
            )),
            error_type: Box::new(resolve_type_expr_with_params(
                arena,
                *error_type,
                type_params,
            )),
            allow_null_state: *allow_null_state,
        },
        // v0.7.4.3-debt.1: `Trilean!` annotation per ADR-0021 §2.7.
        TypeExpr::RefinedTrilean => Type::TRILEAN_KNOWN,
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
                "module helper\nfrom crate.helper import greet\nfunction main() -> Integer = greet()",
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
from crate.helper import greet
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
}
